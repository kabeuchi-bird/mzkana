use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mzkana_core::{
    load_layout, InputEvent, MozcOutput, MozcWorker, Op, OnFocusChange,
    OutputAction, PreeditFallback, SensitiveFieldBehavior, StateMachine, WorkerError,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct Engine {
    sm: StateMachine,
    mozc: Option<MozcWorker>,
    mozc_socket: Option<PathBuf>,
    mozc_retry_at: Option<Instant>,
    config_path: PathBuf,
    watcher: RecommendedWatcher,
    reload_rx: mpsc::Receiver<()>,
    trace_enabled: bool,
    preedit: String,
    /// Candidate window from the most recent Mozc output (current page), stored as
    /// NUL-terminated UTF-8 buffers so the FFI layer can hand out C string pointers
    /// that stay valid until the next key event. Read by the candidate accessors;
    /// the C++ layer renders them after each key event.
    candidates: Vec<FfiCandidate>,
    /// Focused candidate index during conversion; `None` for suggestion windows or
    /// when there is no candidate window.
    focused_index: Option<u32>,
}

/// Candidate prepared for the C ABI: NUL-terminated UTF-8 buffers plus the
/// Mozc-internal id. The trailing NUL is not counted in the returned length.
pub struct FfiCandidate {
    /// value bytes followed by a single NUL.
    pub value: Vec<u8>,
    /// annotation bytes followed by a single NUL, or `None` when absent.
    pub annotation: Option<Vec<u8>>,
    /// Mozc-internal candidate id, or `None` when not selectable by id.
    pub id: Option<i32>,
}

impl FfiCandidate {
    fn from_core(c: mzkana_core::Candidate) -> Self {
        let with_nul = |s: String| {
            let mut b = s.into_bytes();
            b.push(0);
            b
        };
        Self {
            value: with_nul(c.value),
            annotation: c.annotation.map(with_nul),
            id: c.id,
        }
    }
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
            watcher,
            reload_rx,
            trace_enabled: std::env::var_os("MZKANA_TRACE").is_some(),
            preedit: String::new(),
            candidates: Vec::new(),
            focused_index: None,
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
                // reverting first the speculative kana already sent to Mozc would be
                // orphaned in the Mozc preedit.
                self.reset();
                self.sm = StateMachine::new(layout);
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

    /// Switch to a different layout file (configtool selection path, §13.5) and
    /// reload it immediately. Moves the hot-reload watch to the new file's
    /// directory when it differs. Returns true if the new layout loaded; on
    /// failure the current layout is kept unchanged.
    pub fn reload_layout_from(&mut self, new_path: &Path) -> bool {
        let layout = match std::fs::read_to_string(new_path)
            .ok()
            .and_then(|src| load_layout(&src).ok())
        {
            Some(l) => l,
            None => {
                tracing::warn!(
                    "layout load failed for {}, keeping current config",
                    new_path.display()
                );
                return false;
            }
        };

        // H4: revert the in-flight composition before swapping (same rationale as
        // check_reload — the StateMachine's tentative buffer is dropped, so the
        // speculative kana already sent to Mozc must be reverted first).
        self.reset();
        self.sm = StateMachine::new(layout);

        // Move the hot-reload watch to the new file's directory if it changed, so
        // direct edits to the newly selected file are still picked up (§10).
        let old_parent = self.config_path.parent().map(Path::to_path_buf);
        let new_parent = new_path.parent().map(Path::to_path_buf);
        if old_parent != new_parent {
            if let Some(old) = old_parent.as_deref() {
                let _ = self.watcher.unwatch(old);
            }
            if let Some(new) = new_parent.as_deref() {
                if let Err(e) = self.watcher.watch(new, RecursiveMode::NonRecursive) {
                    tracing::warn!("hot-reload re-watch failed for {}: {e}", new.display());
                }
            }
        }

        self.config_path = new_path.to_path_buf();
        tracing::info!("layout switched to {}", self.config_path.display());
        true
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
        if self.trace_enabled {
            eprintln!(
                "[mzkana-trace] key={key:?} {} shift={shift} → actions={actions:?} tentative={:?}",
                if is_down { "DOWN" } else { "UP" },
                self.sm.tentative_kana_string(),
            );
        }
        self.dispatch_actions(actions, is_down)
    }

    pub fn tick(&mut self) -> ProcessResult {
        let now = Instant::now();
        let actions = self.sm.tick(now);
        self.dispatch_actions(actions, false)
    }

    /// Select and commit a candidate by its Mozc-internal id (number-key / click
    /// selection). Runs a single SELECT_CANDIDATE op on the worker, applies the
    /// resulting commit, and resets the local composition state so the next key
    /// starts fresh. Returns the commit (if any) for the C++ layer.
    pub fn select_candidate(&mut self, candidate_id: i32) -> ProcessResult {
        let mut commit: Option<String> = None;
        let mut consumed = false;
        if let Some(ref mut mozc) = self.mozc {
            match mozc.batch(vec![Op::SelectCandidate(candidate_id)]) {
                Ok(mut results) => match results.pop() {
                    Some(Ok(out)) => {
                        self.apply_mozc_output(out, &mut commit);
                        consumed = true;
                    }
                    _ => self.mozc = None,
                },
                Err(WorkerError::Dead) => self.mozc = None,
            }
        }
        // A committed candidate ends the composition: clear local SM state so the
        // released chord keys / tentative buffer don't leak into the next input,
        // and drop the cached preedit/candidates so the FFI accessors never report
        // stale UI state (including on the Mozc-failure path).
        self.sm.reset();
        self.clear_ui_cache();
        ProcessResult {
            consumed,
            preedit: self.preedit.clone(),
            commit,
            passthrough_key: None,
            forward_key: None,
            forward_mods: 0,
        }
    }

    /// Drop the cached preedit and candidate window so the FFI accessors
    /// (`candidate_count` / `focused_index` / `ProcessResult.preedit`) don't
    /// return state left over from a finished composition.
    fn clear_ui_cache(&mut self) {
        self.preedit.clear();
        self.candidates.clear();
        self.focused_index = None;
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
        self.clear_ui_cache();
    }

    /// Handle focus loss (IM deactivation), honoring the `on_focus_change` setting (H3).
    /// Returns `true` if the composition was preserved (caller should not clear UI).
    pub fn focus_out_preserved(&mut self) -> bool {
        match self.sm.settings().on_focus_change {
            OnFocusChange::Preserve => true,
            OnFocusChange::Reset => {
                self.reset();
                false
            }
        }
    }

    /// `sensitive_field_behavior` setting as an int for the FFI/C++ layer
    /// (0 = passthrough, 1 = buffer).
    pub fn sensitive_field_behavior(&self) -> i32 {
        match self.sm.settings().sensitive_field_behavior {
            SensitiveFieldBehavior::Passthrough => 0,
            SensitiveFieldBehavior::Buffer => 1,
        }
    }

    /// `preedit_fallback` setting as an int for the FFI/C++ layer
    /// (0 = client, 1 = panel, 2 = buffer, 3 = auto).
    pub fn preedit_fallback(&self) -> i32 {
        match self.sm.settings().preedit_fallback {
            PreeditFallback::Client => 0,
            PreeditFallback::Panel => 1,
            PreeditFallback::Buffer => 2,
            PreeditFallback::Auto => 3,
        }
    }

    /// Translate an `OutputAction` into the Mozc op it issues, if any.
    fn action_op(action: &OutputAction) -> Option<Op> {
        match action {
            OutputAction::SendKana(s) => Some(Op::SendKana(s.clone())),
            OutputAction::Backspace => Some(Op::Backspace),
            OutputAction::MozcSubmit => Some(Op::Submit),
            OutputAction::SendFunctionKey(name) => Some(Op::SendFunctionKey(name.clone())),
            OutputAction::SendModifiedKey { key, mods } => {
                if *mods & mzkana_core::MOD_SUPER != 0 {
                    None
                } else {
                    Some(Op::SendModified { key: key.clone(), mods: *mods })
                }
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
        // The latest output's candidate window reflects the current UI state.
        self.candidates = out.candidates.into_iter().map(FfiCandidate::from_core).collect();
        self.focused_index = out.focused_index;
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

    /// Number of candidates in the most recent Mozc output.
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// The `i`-th prepared candidate (NUL-terminated buffers), or `None`.
    pub fn candidate(&self, i: usize) -> Option<&FfiCandidate> {
        self.candidates.get(i)
    }

    /// Focused candidate index during conversion, or `None`.
    pub fn focused_index(&self) -> Option<u32> {
        self.focused_index
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

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../layouts")
            .join(name)
    }

    /// Build an engine without touching a real Mozc server: a bogus socket path
    /// makes `connect` skip server auto-spawn and fail fast (mozc → None).
    fn engine_for(layout: &str) -> Engine {
        let bogus = Path::new("/nonexistent/mzkana-test.sock");
        Engine::new(&layout_path(layout), Some(bogus)).expect("engine builds without Mozc")
    }

    #[test]
    fn reload_layout_switches_to_a_different_file() {
        let mut e = engine_for("naginata-v17.toml");
        let target = layout_path("tsuki-2-263.toml");
        assert!(e.reload_layout_from(&target), "valid layout should load");
        assert_eq!(e.config_path, target, "config_path tracks the new file");
    }

    #[test]
    fn reload_layout_keeps_current_on_failure() {
        let mut e = engine_for("naginata-v17.toml");
        let original = e.config_path.clone();
        let missing = Path::new("/nonexistent/does-not-exist.toml");
        assert!(!e.reload_layout_from(missing), "missing file should fail");
        assert_eq!(e.config_path, original, "config_path unchanged on failure");
    }
}
