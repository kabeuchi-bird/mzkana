use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use mzkana_core::{
    load_layout, InputEvent, MozcClient, MozcOutput, OutputAction, StateMachine,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct Engine {
    sm: StateMachine,
    mozc: Option<MozcClient>,
    config_path: PathBuf,
    _watcher: RecommendedWatcher,
    reload_rx: mpsc::Receiver<()>,
    preedit: String,
}

impl Engine {
    pub fn new(config_path: &Path, socket_path: Option<&Path>) -> Result<Self, String> {
        let src = std::fs::read_to_string(config_path)
            .map_err(|e| format!("cannot read config {}: {e}", config_path.display()))?;
        let layout = load_layout(&src).map_err(|e| format!("config error: {e}"))?;
        let sm = StateMachine::new(layout);

        let mozc = match MozcClient::connect(socket_path) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("Mozc not available: {e}; running without Mozc");
                None
            }
        };

        let (tx, reload_rx) = mpsc::channel::<()>();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| {
                if let Ok(ev) = result {
                    if matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        let _ = tx.send(());
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| format!("notify watcher error: {e}"))?;

        if let Some(parent) = config_path.parent() {
            if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                tracing::warn!("hot-reload watch failed for {}: {e}", parent.display());
            }
        }

        Ok(Self {
            sm,
            mozc,
            config_path: config_path.to_path_buf(),
            _watcher: watcher,
            reload_rx,
            preedit: String::new(),
        })
    }

    /// Returns true if config was successfully reloaded.
    pub fn check_reload(&mut self) -> bool {
        let triggered = self.reload_rx.try_recv().is_ok();
        // drain remaining events to avoid repeated triggers
        while self.reload_rx.try_recv().is_ok() {}

        if !triggered {
            return false;
        }

        let reloaded = std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|src| load_layout(&src).ok());

        match reloaded {
            Some(layout) => {
                self.sm = StateMachine::new(layout);
                self.preedit.clear();
                tracing::info!("config reloaded from {}", self.config_path.display());
                true
            }
            None => {
                tracing::warn!(
                    "config reload failed for {}, keeping current config",
                    self.config_path.display()
                );
                false
            }
        }
    }

    pub fn key_event(&mut self, key: &str, is_down: bool, shift: bool) -> ProcessResult {
        let now = Instant::now();
        let event = if is_down {
            InputEvent { key: key.into(), kind: mzkana_core::KeyEventKind::Down, shift, is_repeat: false }
        } else {
            InputEvent { key: key.into(), kind: mzkana_core::KeyEventKind::Up, shift: false, is_repeat: false }
        };

        let actions = self.sm.process(event, now);
        self.dispatch_actions(actions, is_down)
    }

    pub fn tick(&mut self) -> ProcessResult {
        let now = Instant::now();
        let actions = self.sm.tick(now);
        self.dispatch_actions(actions, false)
    }

    pub fn reset(&mut self) {
        self.sm.reset();
        if let Some(ref mut mozc) = self.mozc {
            let _ = mozc.revert();
        }
        self.preedit.clear();
    }

    fn dispatch_actions(&mut self, actions: Vec<OutputAction>, is_key_down: bool) -> ProcessResult {
        if actions.is_empty() {
            return ProcessResult {
                consumed: is_key_down,
                preedit: self.preedit.clone(),
                commit: None,
                passthrough_key: None,
                forward_key: None,
                forward_mods: 0,
            };
        }

        let mut commit: Option<String> = None;
        let mut passthrough_key: Option<String> = None;
        let mut forward_key: Option<String> = None;
        let mut forward_mods: u8 = 0;
        let mut any_consumed = false;

        for action in &actions {
            match action {
                OutputAction::Passthrough(k) => {
                    passthrough_key = Some(k.clone());
                }
                OutputAction::CommitDirect(s) => {
                    commit = Some(s.clone());
                    any_consumed = true;
                }
                OutputAction::SubmitAndCommit(s) => {
                    if let Some(ref mut mozc) = self.mozc {
                        if let Ok(out) = mozc.submit() {
                            self.apply_mozc_output(out, &mut commit);
                        }
                    }
                    // Append the direct commit after any Mozc commit
                    let direct = s.clone();
                    match commit {
                        Some(ref mut c) => c.push_str(&direct),
                        None => commit = Some(direct),
                    }
                    any_consumed = true;
                }
                OutputAction::SendModifiedKey { key, mods } => {
                    let mozc_consumed = self.mozc.as_mut()
                        .and_then(|mozc| mozc.send_modified_key(key, *mods).ok())
                        .map(|out| {
                            let consumed = out.consumed;
                            self.apply_mozc_output(out, &mut commit);
                            consumed
                        })
                        .unwrap_or(false);

                    if !mozc_consumed {
                        forward_key = Some(key.clone());
                        forward_mods = *mods;
                    }
                    any_consumed = true;
                }
                _ => {
                    let result = self.dispatch_to_mozc(action);
                    // Only consume the key if Mozc was connected (it handled or will handle
                    // the action) or the fallback returned output (SendKana without Mozc).
                    // When Mozc is absent and dispatch returned None, let the key pass through.
                    if result.is_some() || self.mozc.is_some() {
                        if let Some(out) = result {
                            self.apply_mozc_output(out, &mut commit);
                        }
                        any_consumed = true;
                    }
                }
            }
        }

        let consumed = any_consumed || passthrough_key.is_none();
        ProcessResult { consumed, preedit: self.preedit.clone(), commit, passthrough_key, forward_key, forward_mods }
    }

    fn dispatch_to_mozc(&mut self, action: &OutputAction) -> Option<MozcOutput> {
        if let Some(ref mut mozc) = self.mozc {
            return match action {
                OutputAction::SendKana(s) => mozc.send_kana(s).ok(),
                OutputAction::Backspace => mozc.send_backspace().ok(),
                OutputAction::MozcSubmit => mozc.submit().ok(),
                OutputAction::SendFunctionKey(name) => mozc.send_function_key(name).ok(),
                _ => None,
            };
        }
        // Mozc not connected: commit kana directly so raw kana input still works.
        if let OutputAction::SendKana(s) = action {
            return Some(MozcOutput {
                preedit: String::new(),
                result: Some(s.clone()),
                is_converting: false,
                mode: 0,
                consumed: true,
            });
        }
        None
    }

    fn apply_mozc_output(&mut self, out: MozcOutput, commit: &mut Option<String>) {
        self.preedit = out.preedit;
        if let Some(result) = out.result {
            match commit {
                Some(ref mut c) => c.push_str(&result),
                None => *commit = Some(result),
            }
        }
        if out.is_converting {
            self.sm.notify_mozc_conversion();
        } else {
            self.sm.notify_mozc_composition();
        }
    }
}

pub struct ProcessResult {
    pub consumed: bool,
    pub preedit: String,
    pub commit: Option<String>,
    pub passthrough_key: Option<String>,
    /// Set when a modifier+key token was not consumed by Mozc; the C++ layer should
    /// call ic->forwardKey() with this key name and the modifier bitmask.
    pub forward_key: Option<String>,
    pub forward_mods: u8,
}
