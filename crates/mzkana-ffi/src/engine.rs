use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mzkana_core::{
    load_layout, InputEvent, MozcClient, MozcOutput, OutputAction, StateMachine,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct Engine {
    sm: StateMachine,
    mozc: Option<MozcClient>,
    /// Explicit socket path passed at construction time; None = auto-discover.
    mozc_socket: Option<PathBuf>,
    /// When Some, don't attempt to reconnect until this instant.
    mozc_retry_at: Option<Instant>,
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
            Ok(c) => {
                eprintln!("[mzkana] Mozc connected (session {})", c.session_id());
                Some(c)
            }
            Err(e) => {
                eprintln!("[mzkana] Mozc not available: {e}");
                eprintln!("[mzkana]   socket tried: {}",
                    socket_path.map(|p| p.display().to_string())
                        .unwrap_or_else(|| mzkana_core::mozc::default_socket_path().display().to_string()));
                eprintln!("[mzkana]   abstract socket: {:?}",
                    mzkana_core::mozc::find_abstract_socket_name());
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
            mozc_socket: socket_path.map(Path::to_path_buf),
            mozc_retry_at: None,
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
        self.try_reconnect_mozc(now);
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

    pub fn mozc_available(&self) -> bool {
        self.mozc.is_some()
    }

    /// Try to connect Mozc if not currently connected and the backoff has expired.
    ///
    /// Uses a non-spawning quick connect (just checks existing sockets) so this
    /// is cheap to call on every key event.  Backoff is 5 seconds between attempts
    /// to avoid hammering the socket.
    pub fn try_reconnect_mozc(&mut self, now: Instant) -> bool {
        if self.mozc.is_some() {
            return false;
        }
        if let Some(retry_at) = self.mozc_retry_at {
            if now < retry_at {
                return false;
            }
        }
        match MozcClient::connect_quick(self.mozc_socket.as_deref()) {
            Ok(c) => {
                eprintln!("[mzkana] Mozc reconnected (session {})", c.session_id());
                self.mozc = Some(c);
                self.mozc_retry_at = None;
                true
            }
            Err(e) => {
                eprintln!("[mzkana] Mozc reconnect failed: {e}; retrying in 5s");
                self.mozc_retry_at = Some(now + Duration::from_secs(5));
                false
            }
        }
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
                    let send_result = self.mozc.as_mut()
                        .map(|mozc| mozc.send_modified_key(key, *mods));

                    let mozc_consumed = match send_result {
                        Some(Ok(out)) => {
                            let consumed = out.consumed;
                            self.apply_mozc_output(out, &mut commit);
                            consumed
                        }
                        Some(Err(_)) => {
                            // I/O failure — mark dead so try_reconnect_mozc fires next event.
                            self.mozc = None;
                            false
                        }
                        None => false,
                    };

                    if !mozc_consumed {
                        forward_key = Some(key.clone());
                        forward_mods = *mods;
                    }
                    any_consumed = true;
                }
                _ => {
                    match self.dispatch_to_mozc(action) {
                        Some(out) => {
                            let mozc_consumed = out.consumed;
                            self.apply_mozc_output(out, &mut commit);
                            if mozc_consumed {
                                any_consumed = true;
                            } else {
                                // Mozc returned but didn't consume (e.g. BackSpace on
                                // empty preedit) — passthrough the natural key so the
                                // application can handle it (e.g. delete text, undo).
                                let pt = match action {
                                    OutputAction::Backspace => Some("BackSpace"),
                                    OutputAction::MozcSubmit => Some("Return"),
                                    OutputAction::SendFunctionKey(name) => Some(name.as_str()),
                                    _ => { any_consumed = true; None }
                                };
                                if let Some(key) = pt {
                                    passthrough_key = Some(key.to_string());
                                }
                            }
                        }
                        None if self.mozc.is_some() => {
                            // I/O failure — mark dead so try_reconnect_mozc fires next event.
                            self.mozc = None;
                            any_consumed = true;
                        }
                        None => {
                            // Mozc absent: passthrough the natural key.
                            let pt = match action {
                                OutputAction::Backspace => Some("BackSpace"),
                                OutputAction::MozcSubmit => Some("Return"),
                                OutputAction::SendFunctionKey(name) => Some(name.as_str()),
                                _ => None,
                            };
                            if let Some(key) = pt {
                                tracing::warn!("Mozc absent; passing through as key: {key}");
                                passthrough_key = Some(key.to_string());
                            }
                        }
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
