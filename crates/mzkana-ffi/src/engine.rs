use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mzkana_core::{
    load_layout, InputEvent, MozcOutput, MozcWorker, Op, OutputAction, StateMachine, WorkerError,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct Engine {
    sm: StateMachine,
    /// Mozc IPC runs on a worker thread with a hard timeout so a slow/hung
    /// `mozc_server` can never freeze the UI (C5).
    mozc: Option<MozcWorker>,
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

        let mozc = match MozcWorker::connect(socket_path) {
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
                // H4: clear the in-flight composition before swapping configs.
                // Replacing the StateMachine drops its tentative buffer, so without
                // this the speculative kana already sent to Mozc would be orphaned
                // in the Mozc preedit. Revert cancels it cleanly first.
                self.sm.reset();
                if let Some(ref mut mozc) = self.mozc {
                    if mozc.batch(vec![Op::Revert]).is_err() {
                        self.mozc = None;
                    }
                }
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
        match MozcWorker::connect_quick(self.mozc_socket.as_deref()) {
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
            // Revert the Mozc preedit; if the worker has gone dead, discard it so
            // the next event reconnects.
            if mozc.batch(vec![Op::Revert]).is_err() {
                self.mozc = None;
            }
        }
        self.preedit.clear();
    }

    /// Translate an `OutputAction` into the Mozc op it issues, if any.
    fn action_op(action: &OutputAction) -> Option<Op> {
        match action {
            OutputAction::SendKana(s) => Some(Op::SendKana(s.clone())),
            OutputAction::Backspace => Some(Op::Backspace),
            OutputAction::MozcSubmit => Some(Op::Submit),
            OutputAction::SendFunctionKey(name) => Some(Op::SendFunctionKey(name.clone())),
            OutputAction::SendModifiedKey { key, mods } => {
                Some(Op::SendModified { key: key.clone(), mods: *mods })
            }
            OutputAction::SubmitAndCommit(_) | OutputAction::SubmitThenPassthrough(_) => Some(Op::Submit),
            OutputAction::Passthrough(_) | OutputAction::CommitDirect(_) => None,
        }
    }

    /// The natural key to forward to the application when Mozc is unavailable or
    /// did not consume the event.
    fn fallback_passthrough(action: &OutputAction) -> Option<String> {
        match action {
            OutputAction::Backspace => Some("BackSpace".to_string()),
            OutputAction::MozcSubmit => Some("Return".to_string()),
            OutputAction::SendFunctionKey(name) => Some(name.clone()),
            OutputAction::SubmitThenPassthrough(k) => Some(k.clone()),
            _ => None,
        }
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

        // Collect every Mozc op this keystroke issues and run them as ONE batch on
        // the worker thread (a single channel round-trip and one 150 ms timeout
        // budget, instead of one per op — important for chord rewrites that emit
        // BackSpace×N + SendKana).
        let ops: Vec<Op> = actions.iter().filter_map(Self::action_op).collect();
        let mut outputs: std::collections::VecDeque<MozcOutput> = std::collections::VecDeque::new();
        let mut mozc_alive = false;

        if !ops.is_empty() {
            if let Some(ref mut mozc) = self.mozc {
                match mozc.batch(ops) {
                    Ok(results) => {
                        mozc_alive = true;
                        for r in results {
                            match r {
                                Ok(out) => outputs.push_back(out),
                                Err(_) => {
                                    // A per-op I/O failure means the connection is
                                    // broken; drop it so the next event reconnects.
                                    self.mozc = None;
                                    mozc_alive = false;
                                    outputs.clear();
                                    break;
                                }
                            }
                        }
                    }
                    Err(WorkerError::Dead) => {
                        // Timed out or thread gone: discard the worker (its Drop
                        // detaches the possibly-hung thread) and proceed without it
                        // so the UI never blocks.  Reconnect happens next event.
                        self.mozc = None;
                    }
                }
            }
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
                    if let Some(out) = outputs.pop_front() {
                        self.apply_mozc_output(out, &mut commit);
                    }
                    // Append the direct commit after any Mozc commit.
                    match commit {
                        Some(ref mut c) => c.push_str(s),
                        None => commit = Some(s.clone()),
                    }
                    any_consumed = true;
                }
                OutputAction::SubmitThenPassthrough(k) => {
                    // Commit the current preedit (if Mozc answered), then let the raw
                    // key fall through to the application.
                    if let Some(out) = outputs.pop_front() {
                        self.apply_mozc_output(out, &mut commit);
                    }
                    passthrough_key = Some(k.clone());
                }
                OutputAction::SendModifiedKey { key, mods } => {
                    let mozc_consumed = match outputs.pop_front() {
                        Some(out) => {
                            let consumed = out.consumed;
                            self.apply_mozc_output(out, &mut commit);
                            consumed
                        }
                        None => false,
                    };
                    if !mozc_consumed {
                        forward_key = Some(key.clone());
                        forward_mods = *mods;
                    }
                    any_consumed = true;
                }
                // SendKana / Backspace / MozcSubmit / SendFunctionKey
                _ => {
                    if let Some(out) = outputs.pop_front() {
                        let mozc_consumed = out.consumed;
                        self.apply_mozc_output(out, &mut commit);
                        if mozc_consumed {
                            any_consumed = true;
                        } else if let Some(pt) = Self::fallback_passthrough(action) {
                            // Mozc returned but didn't consume (e.g. BackSpace on an
                            // empty preedit) — pass the natural key to the application.
                            passthrough_key = Some(pt);
                        } else {
                            any_consumed = true;
                        }
                    } else if mozc_alive {
                        // No output for an op that should have produced one: treat as
                        // consumed (the op ran) without further action.
                        any_consumed = true;
                    } else if let OutputAction::SendKana(s) = action {
                        // Mozc absent: commit kana directly so raw kana still works.
                        match commit {
                            Some(ref mut c) => c.push_str(s),
                            None => commit = Some(s.clone()),
                        }
                        any_consumed = true;
                    } else if let Some(pt) = Self::fallback_passthrough(action) {
                        tracing::warn!("Mozc absent; passing through as key: {pt}");
                        passthrough_key = Some(pt);
                    }
                }
            }
        }

        let consumed = any_consumed || passthrough_key.is_none();
        ProcessResult { consumed, preedit: self.preedit.clone(), commit, passthrough_key, forward_key, forward_mods }
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
