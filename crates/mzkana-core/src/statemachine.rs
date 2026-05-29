use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use crate::config::{HoldDetection, Layout, LayoutMode, ModifierKind, TapAction, TriggerKind};

// ── Public event types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEventKind {
    Down,
    Up,
}

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub key: String,
    pub kind: KeyEventKind,
    pub shift: bool,
    pub is_repeat: bool,
}

impl InputEvent {
    pub fn down(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind: KeyEventKind::Down,
            shift: false,
            is_repeat: false,
        }
    }

    pub fn up(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind: KeyEventKind::Up,
            shift: false,
            is_repeat: false,
        }
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }
}

/// Output actions produced by the state machine for each input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputAction {
    /// Send a kana string to Mozc preedit
    SendKana(String),
    /// Send BackSpace to Mozc preedit
    Backspace,
    /// Commit text directly (kanchoku)
    CommitDirect(String),
    /// Submit current Mozc preedit then commit text
    SubmitAndCommit(String),
    /// Pass the key through to the application unchanged
    Passthrough(String),
    /// Submit current Mozc preedit, then pass the key through to the application.
    /// Used for unassigned keys during active composition (C4).
    SubmitThenPassthrough(String),
    /// Notify Mozc that conversion is complete (e.g. Enter)
    MozcSubmit,
    /// Send a function/control key (e.g. Return, Tab, Up) bypassing Mozc preedit.
    /// The string is the XKB keysym name as used in the layout file after the `!` prefix.
    SendFunctionKey(String),
    /// Send a modifier+key combination via Mozc. If Mozc consumes it (e.g. S-!Left during
    /// conversion adjusts segment boundaries), the preedit is updated. If Mozc does not
    /// consume it, the key is forwarded to the application via forwardKey().
    /// `mods` is a bitmask: MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER.
    /// `key` is the XKB keysym name ("z", "Left", "Return", etc.).
    SendModifiedKey { key: String, mods: u8 },
}

pub const MOD_SHIFT: u8 = 0x01;
pub const MOD_CTRL:  u8 = 0x02;
pub const MOD_ALT:   u8 = 0x04;
pub const MOD_SUPER: u8 = 0x08;

/// Mozc control/editing keys (C4).
/// Keys in this set are sent to Mozc during composition for conversion/navigation.
/// Keys NOT in this set, when pressed during composition, trigger SubmitThenPassthrough.
const MOZC_CONTROL_KEYS: &[&str] = &[
    "space", "Return", "Tab", "Left", "Right", "Up", "Down",
    "Home", "End", "Prior", "Next", "Delete", "Escape",
    "Henkan", "Muhenkan", "Hiragana_Katakana",
];

// ── Internal state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TentativeChar {
    kana: String,
    source_keys: Vec<String>,
    /// Some(deadline) → chord rewrite window; None → sequence (permanent) or confirmed
    rewrite_deadline: Option<Instant>,
    /// Tokens after the leading kana that are emitted once this char is confirmed
    /// (chord deadline expired or no chord candidates). Used for sequences like
    /// `"、 !Enter"` whose leading kana is sent speculatively.
    pending_tail: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MozcMode {
    Composition,
    Conversion,
}

/// State of a center-shift (hold-type) modifier.
#[derive(Debug, Clone)]
struct ModifierState {
    id: String,
    /// True while the physical key is held down (both interrupt and timeout modes).
    held: bool,
    /// True when another key was pressed during the hold (interrupt-style detection).
    interrupted: bool,
    /// When the key was pressed (for timeout detection and tap disambiguation).
    pressed_at: Instant,
    /// Oneshot mode: true after press, cleared after the next non-modifier key.
    oneshot_pending: bool,
    /// Toggle mode: true when the layer is toggled on (sticky).
    toggled: bool,
}

/// State of the direct trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectTriggerState {
    Inactive,
    /// Key is physically held (hold mode) or toggle is on.
    Active,
}

pub struct StateMachine {
    layout: Layout,
    pending_keys: Vec<(String, Instant)>,
    tentative_buffer: Vec<TentativeChar>,
    modifier_states: Vec<ModifierState>,
    direct_trigger_state: DirectTriggerState,
    direct_trigger_interrupted: bool,
    mozc_mode: MozcMode,
    /// Chord timer: tracks the earliest rewrite_deadline in tentative_buffer.
    chord_deadline: Option<Instant>,
    /// Physically held regular (non-modifier, non-trigger) keys, for mutual chord detection.
    held_keys: HashSet<String>,
}

impl StateMachine {
    pub fn new(layout: Layout) -> Self {
        let modifier_states = layout
            .modifiers
            .iter()
            .map(|m| ModifierState {
                id: m.id.clone(),
                held: false,
                interrupted: false,
                pressed_at: Instant::now(),
                oneshot_pending: false,
                toggled: false,
            })
            .collect();

        Self {
            layout,
            pending_keys: Vec::new(),
            tentative_buffer: Vec::new(),
            modifier_states,
            direct_trigger_state: DirectTriggerState::Inactive,
            direct_trigger_interrupted: false,
            mozc_mode: MozcMode::Composition,
            chord_deadline: None,
            held_keys: HashSet::new(),
        }
    }

    /// Reset all transient state (e.g. on focus change with reset policy).
    pub fn reset(&mut self) -> Vec<OutputAction> {
        let actions = self.flush_tentative();
        self.pending_keys.clear();
        self.mozc_mode = MozcMode::Composition;
        self.chord_deadline = None;
        for ms in &mut self.modifier_states {
            ms.held = false;
            ms.interrupted = false;
            ms.oneshot_pending = false;
            ms.toggled = false;
        }
        self.direct_trigger_state = DirectTriggerState::Inactive;
        self.direct_trigger_interrupted = false;
        self.held_keys.clear();
        actions
    }

    /// Call periodically to fire chord timers.
    /// Returns any actions from expired tentative chars (they become confirmed).
    pub fn tick(&mut self, now: Instant) -> Vec<OutputAction> {
        let mut actions = Vec::new();
        if let Some(deadline) = self.chord_deadline {
            if now >= deadline {
                // Confirm all expired tentative chars and emit any deferred tail tokens.
                for tc in &mut self.tentative_buffer {
                    if tc.rewrite_deadline.is_some_and(|d| now >= d) {
                        tc.rewrite_deadline = None;
                        for token in tc.pending_tail.drain(..) {
                            actions.push(Self::make_output_action(&token));
                        }
                    }
                }
                self.update_chord_deadline();
            }
        }
        actions
    }

    /// Process one input event, returning a list of output actions.
    pub fn process(&mut self, event: InputEvent, now: Instant) -> Vec<OutputAction> {
        if event.is_repeat {
            return Vec::new();
        }

        match event.kind {
            KeyEventKind::Down => self.process_key_down(&event.key, event.shift, now),
            KeyEventKind::Up => self.process_key_up(&event.key, now),
        }
    }

    // ── Key Down ──────────────────────────────────────────────────────────────

    fn process_key_down(&mut self, key: &str, shift: bool, now: Instant) -> Vec<OutputAction> {
        // Ctrl/Alt/Super → immediate passthrough (not modelled here; caller should filter)

        // Check if key is a modifier definition
        if let Some(idx) = self.modifier_index(key) {
            let kind = self.layout.modifiers[idx].kind;
            self.modifier_states[idx].interrupted = false;
            self.modifier_states[idx].pressed_at = now;
            match kind {
                ModifierKind::Toggle => {
                    self.modifier_states[idx].toggled ^= true;
                }
                ModifierKind::Oneshot => {
                    self.modifier_states[idx].oneshot_pending = true;
                }
                ModifierKind::Hold => {
                    self.modifier_states[idx].held = true;
                }
            }
            return Vec::new();
        }

        // Check if key is a direct trigger
        if self.is_direct_trigger_key(key) {
            return self.handle_direct_trigger_down(key, now);
        }

        // Track held regular keys for mutual chord detection.
        self.held_keys.insert(key.to_string());

        // Mark modifiers as interrupted (for center-shift interrupt detection)
        for ms in &mut self.modifier_states {
            if ms.held {
                ms.interrupted = true;
            }
        }

        // Backlspace: pop tentative buffer
        if key == "bs" || key == "BackSpace" || key == "backspace" {
            return self.handle_backspace();
        }

        // CONVERSION mode: new key starts fresh composition
        if self.mozc_mode == MozcMode::Conversion {
            self.tentative_buffer.clear();
            self.pending_keys.clear();
            self.mozc_mode = MozcMode::Composition;
        }

        // Check for direct trigger active → kanchoku mode
        if self.direct_trigger_state == DirectTriggerState::Active
            || self.layout.meta.mode == LayoutMode::Kanchoku
        {
            return self.process_direct_key(key, now);
        }

        // Find active modifier layer
        if let Some(mod_output) = self.check_modified_layer(key, now) {
            // Consume oneshot after first use
            for ms in &mut self.modifier_states {
                ms.oneshot_pending = false;
            }
            if mod_output == crate::config::PASSTHROUGH_CELL {
                return vec![OutputAction::Passthrough(key.to_string())];
            }
            let actions = vec![OutputAction::SendKana(mod_output.clone())];
            self.tentative_buffer.push(TentativeChar {
                kana: mod_output,
                source_keys: vec![key.to_string()],
                rewrite_deadline: None,
                pending_tail: Vec::new(),
            });
            return actions;
        }

        // Mutual chord (symmetric=true): fire immediately when all keys are held.
        // Checked before pending_keys.push so the new key hasn't been speculatively emitted yet.
        if let Some(chord) = self.find_mutual_chord_match(key) {
            return self.fire_mutual_chord(chord, now);
        }

        // Normal key: speculative emit
        self.pending_keys.push((key.to_string(), now));
        self.speculative_emit(key, shift, now)
    }

    // ── Key Up ────────────────────────────────────────────────────────────────

    fn process_key_up(&mut self, key: &str, now: Instant) -> Vec<OutputAction> {
        self.held_keys.remove(key);

        // Modifier key released
        if let Some(idx) = self.modifier_index(key) {
            let kind = self.layout.modifiers[idx].kind;

            // Toggle modifiers have no tap action — the toggle happened on key-down.
            if kind == ModifierKind::Toggle {
                return Vec::new();
            }

            let was_interrupted = self.modifier_states[idx].interrupted;
            let had_oneshot = self.modifier_states[idx].oneshot_pending;
            self.modifier_states[idx].held = false;
            self.modifier_states[idx].oneshot_pending = false;

            let modifier_def = self.layout.modifiers[idx].clone();
            // Tap action fires when key was released without triggering anything
            if !was_interrupted && !had_oneshot {
                match modifier_def.tap_action {
                    TapAction::Passthrough => {
                        return vec![OutputAction::Passthrough(key.to_string())];
                    }
                    TapAction::BaseKana => {
                        // Dual-role: resolve base-layer value through alias/sequence pipeline.
                        if let Some(raw) = self.lookup_base(key) {
                            let tokens: Vec<String> = self
                                .resolve_sequence(&raw)
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                            let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
                            return self.emit_sequence(&refs, vec![key.to_string()], None);
                        }
                    }
                    TapAction::Output => {
                        if let Some(raw) = modifier_def.tap_output.as_deref() {
                            let tokens: Vec<String> = self
                                .resolve_sequence(raw)
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                            let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
                            return self.emit_sequence(&refs, vec![key.to_string()], None);
                        }
                    }
                    TapAction::None => {}
                }
            }
            return Vec::new();
        }

        // Direct trigger key released
        if self.is_direct_trigger_key(key) {
            return self.handle_direct_trigger_up(key, now);
        }

        Vec::new()
    }

    // ── Speculative emit ──────────────────────────────────────────────────────

    fn speculative_emit(&mut self, key: &str, _shift: bool, now: Instant) -> Vec<OutputAction> {
        let mut actions = Vec::new();

        // Resolve what chord / prefix / postfix candidates exist.
        // Timed chords (symmetric=false) use a deadline window.
        // Mutual chords (symmetric=true) fire via held_keys, so they use no deadline here
        // but still require speculative emission to allow later rewriting.
        let has_timed_chord = self.has_timed_chord_candidate(key);
        let has_mutual_chord = self.has_mutual_chord_candidate(key);
        let has_prefix_continuation = self.has_prefix_continuation();
        let has_postfix_candidate = self.has_postfix_candidate(key);

        let base_kana = self.lookup_base(key);

        // Check if a prefix sequence already in pending resolves to a new rule
        if let Some(resolved) = self.try_resolve_pending() {
            return self.apply_rule_match(resolved, now);
        }

        // Check for postfix match: current key is trigger for postfix of previous char
        if let Some(resolved) = self.try_resolve_postfix(key) {
            return self.apply_rule_match(resolved, now);
        }

        let Some(output) = base_kana else {
            // No base assignment for this key. Keep in pending_keys only if it may
            // participate in a timed chord (timed chord detection reads pending_keys).
            // Mutual chords fire via held_keys before pending_keys.push, so they don't
            // need the key here.
            if !has_timed_chord && !has_mutual_chord {
                self.pending_keys.pop();

                // C4: Key routing for unassigned keys during composition.
                // Check if we're actively composing (have sent kana to Mozc).
                let is_composing = !self.tentative_buffer.is_empty() || self.mozc_mode == MozcMode::Conversion;

                if is_composing {
                    // During active composition, route the key based on type:
                    // - Control keys (space, Return, etc.) → SendFunctionKey (send to Mozc)
                    // - Other keys → SubmitThenPassthrough (confirm preedit, then pass to app)
                    if Self::is_mozc_control_key(key) {
                        return vec![OutputAction::SendFunctionKey(key.to_string())];
                    } else {
                        // Submitting commits and clears the Mozc preedit, so drop the local
                        // tentative state too to keep the 1:1 invariant (tentative_buffer
                        // mirrors Mozc preedit) intact.
                        self.tentative_buffer.clear();
                        self.pending_keys.clear();
                        self.mozc_mode = MozcMode::Composition;
                        return vec![OutputAction::SubmitThenPassthrough(key.to_string())];
                    }
                } else {
                    // Not composing: pass through to application
                    return vec![OutputAction::Passthrough(key.to_string())];
                }
            }
            return actions;
        };

        if output == crate::config::PASSTHROUGH_CELL {
            // Passthrough keys are sent directly to the application and don't participate
            // in chord matching, so remove the pending entry to prevent spurious matches.
            self.pending_keys.pop();
            return vec![OutputAction::Passthrough(key.to_string())];
        }

        // Resolve to token sequence (alias / quoted sequence / single token)
        let tokens: Vec<String> = self
            .resolve_sequence(&output)
            .iter()
            .map(|s| s.to_string())
            .collect();

        // A leading function-key or modifier-key token: emit entire sequence immediately (non-speculative)
        if tokens.first().is_some_and(|t| Self::is_immediate_action_token(t)) {
            self.pending_keys.clear();
            let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
            return self.emit_sequence(&refs, vec![key.to_string()], None);
        }

        let needs_speculation =
            has_timed_chord || has_mutual_chord || has_prefix_continuation || has_postfix_candidate;

        // Multi-token with no speculation needed: emit all immediately
        if tokens.len() > 1 && !needs_speculation {
            self.pending_keys.clear();
            let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
            return self.emit_sequence(&refs, vec![key.to_string()], None);
        }

        // Speculative emit of first kana; tail (if any) deferred until confirmed
        let kana = tokens[0].clone();
        let pending_tail: Vec<String> = tokens[1..].to_vec();
        actions.push(OutputAction::SendKana(kana.clone()));

        let rewrite_deadline = if has_timed_chord {
            let window = self.chord_window_for(key);
            let deadline = now + Duration::from_millis(window as u64);
            self.chord_deadline = Some(match self.chord_deadline {
                Some(existing) => existing.min(deadline),
                None => deadline,
            });
            Some(deadline)
        } else if has_mutual_chord {
            // Use mutual_window_ms as the confirmation deadline so that tick() can flush
            // any deferred tail tokens (e.g. multi-token output) if no chord fires.
            let window = self.layout.settings.mutual_window_ms;
            let deadline = now + Duration::from_millis(window as u64);
            self.chord_deadline = Some(match self.chord_deadline {
                Some(existing) => existing.min(deadline),
                None => deadline,
            });
            Some(deadline)
        } else if has_prefix_continuation || has_postfix_candidate {
            None // sequence rule: permanently rewritable until resolved
        } else {
            // Confirmed immediately — no continuations; emit any tail right away
            self.pending_keys.clear();
            for t in &pending_tail {
                actions.push(Self::make_output_action(t));
            }
            self.tentative_buffer.push(TentativeChar {
                kana,
                source_keys: vec![key.to_string()],
                rewrite_deadline: None,
                pending_tail: Vec::new(),
            });
            return actions;
        };

        self.tentative_buffer.push(TentativeChar {
            kana,
            source_keys: vec![key.to_string()],
            rewrite_deadline,
            pending_tail,
        });

        actions
    }

    // ── Rule resolution ───────────────────────────────────────────────────────

    fn try_resolve_pending(&mut self) -> Option<RuleMatch> {
        if self.pending_keys.len() < 2 {
            return None;
        }

        let keys: Vec<String> = self.pending_keys.iter().map(|(k, _)| k.clone()).collect();

        // Try prefix layers
        for (trigger, grid) in &self.layout.prefix_layers {
            if keys[0] == *trigger {
                if let Some(kana) = grid.get(&keys[keys.len() - 1]) {
                    if self.has_full_sequence_pending(trigger, &keys[keys.len() - 1]) {
                        return Some(RuleMatch {
                            output: kana.clone(),
                            source_keys: keys,
                        });
                    }
                }
            }
        }

        // Try chord with all pending keys
        if let Some(m) = self.try_chord_match(&keys) {
            return Some(m);
        }

        None
    }

    fn try_resolve_postfix(&mut self, trigger_key: &str) -> Option<RuleMatch> {
        if self.tentative_buffer.is_empty() {
            return None;
        }

        // The last tentative char's source_keys + trigger_key form a postfix sequence
        let prev_keys = self.tentative_buffer.last()?.source_keys.clone();

        for (trigger, grid) in &self.layout.postfix_layers {
            if trigger_key == *trigger {
                if let Some(prev_key) = prev_keys.first() {
                    if let Some(kana) = grid.get(prev_key.as_str()) {
                        let mut source = prev_keys.clone();
                        source.push(trigger_key.to_string());
                        return Some(RuleMatch {
                            output: kana.clone(),
                            source_keys: source,
                        });
                    }
                }
            }
        }
        None
    }

    fn try_chord_match(&self, keys: &[String]) -> Option<RuleMatch> {
        let key_set: BTreeSet<&str> = keys.iter().map(String::as_str).collect();

        for chord in &self.layout.chords {
            // Mutual chords (symmetric=true) are handled by the held_keys mechanism.
            if chord.symmetric {
                continue;
            }
            let chord_set: BTreeSet<&str> = chord.keys.iter().map(String::as_str).collect();
            if chord_set == key_set {
                // Both key orders are accepted for timed chords.
                return Some(RuleMatch {
                    output: chord.output.clone(),
                    source_keys: keys.to_vec(),
                });
            }
        }
        None
    }

    fn apply_rule_match(&mut self, rule: RuleMatch, _now: Instant) -> Vec<OutputAction> {
        let mut actions = Vec::new();

        // Find tentative chars to rewrite
        let affected_count = self
            .tentative_buffer
            .iter()
            .rev()
            .take_while(|tc| tc.source_keys.iter().any(|k| rule.source_keys.contains(k)))
            .count();

        for _ in 0..affected_count {
            actions.push(OutputAction::Backspace);
        }
        self.tentative_buffer
            .truncate(self.tentative_buffer.len() - affected_count);

        // Resolve the output string to owned tokens, then emit
        let tokens: Vec<String> = self
            .resolve_sequence(&rule.output)
            .iter()
            .map(|s| s.to_string())
            .collect();
        let token_refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let seq_actions = self.emit_sequence(&token_refs, rule.source_keys, None);
        actions.extend(seq_actions);

        self.pending_keys.clear();
        self.update_chord_deadline();
        actions
    }

    // ── Direct (kanchoku) mode ────────────────────────────────────────────────

    fn process_direct_key(&mut self, key: &str, now: Instant) -> Vec<OutputAction> {
        self.direct_trigger_interrupted = true;
        self.pending_keys.push((key.to_string(), now));
        let pending: Vec<String> = self.pending_keys.iter().map(|(k, _)| k.clone()).collect();

        // Check for complete match
        for rule in &self.layout.directs.clone() {
            if rule.sequence == pending {
                let output = rule.output.clone();
                self.pending_keys.clear();

                let mut actions = Vec::new();
                // Flush any remaining tentative kana first
                let flush = self.flush_tentative();
                if !flush.is_empty() {
                    actions.extend(flush);
                    actions.push(OutputAction::MozcSubmit);
                }
                // Resolve aliases / sequences, then emit each token
                let tokens: Vec<String> = self
                    .resolve_sequence(&output)
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                for token in tokens {
                    actions.push(if Self::is_immediate_action_token(&token) {
                        Self::make_output_action(&token)
                    } else {
                        OutputAction::CommitDirect(token)
                    });
                }
                return actions;
            }
        }

        // Check for prefix match (more keys expected)
        let has_prefix = self
            .layout
            .directs
            .iter()
            .any(|r| r.sequence.starts_with(&pending) && r.sequence.len() > pending.len());

        if !has_prefix {
            // No match and no continuation → discard pending
            self.pending_keys.clear();
        }

        Vec::new()
    }

    // ── Direct trigger ────────────────────────────────────────────────────────

    fn is_direct_trigger_key(&self, key: &str) -> bool {
        self.layout
            .direct_trigger
            .as_ref()
            .is_some_and(|dt| dt.keys.iter().any(|k| k == key))
    }

    fn handle_direct_trigger_down(&mut self, _key: &str, _now: Instant) -> Vec<OutputAction> {
        let kind = self
            .layout
            .direct_trigger
            .as_ref()
            .map(|dt| dt.kind)
            .unwrap_or(TriggerKind::Hold);

        match kind {
            TriggerKind::Hold => {
                self.direct_trigger_interrupted = false;
                self.direct_trigger_state = DirectTriggerState::Active;
                // Clear pending/tentative on activation
                let flush = self.flush_tentative();
                self.pending_keys.clear();
                flush
            }
            TriggerKind::Toggle => {
                if self.direct_trigger_state == DirectTriggerState::Inactive {
                    self.direct_trigger_state = DirectTriggerState::Active;
                    let flush = self.flush_tentative();
                    self.pending_keys.clear();
                    flush
                } else {
                    self.direct_trigger_state = DirectTriggerState::Inactive;
                    self.pending_keys.clear();
                    Vec::new()
                }
            }
        }
    }

    fn handle_direct_trigger_up(&mut self, key: &str, _now: Instant) -> Vec<OutputAction> {
        let kind = self
            .layout
            .direct_trigger
            .as_ref()
            .map(|dt| dt.kind)
            .unwrap_or(TriggerKind::Hold);

        if kind != TriggerKind::Hold {
            return Vec::new();
        }

        let tap_action = self
            .layout
            .direct_trigger
            .as_ref()
            .map(|dt| dt.tap_action)
            .unwrap_or(TapAction::Passthrough);

        let was_interrupted = self.direct_trigger_interrupted;
        self.direct_trigger_state = DirectTriggerState::Inactive;
        self.pending_keys.clear();

        if !was_interrupted {
            match tap_action {
                TapAction::Passthrough => {
                    return vec![OutputAction::Passthrough(key.to_string())];
                }
                // BaseKana / Output / None are not meaningful for direct triggers.
                TapAction::BaseKana | TapAction::Output | TapAction::None => {}
            }
        }
        Vec::new()
    }

    // ── Backspace handling ────────────────────────────────────────────────────

    fn handle_backspace(&mut self) -> Vec<OutputAction> {
        if !self.tentative_buffer.is_empty() {
            self.tentative_buffer.pop();
            self.pending_keys.clear();
            vec![OutputAction::Backspace]
        } else if !self.pending_keys.is_empty() {
            // Partial prefix/direct sequence in progress — discard silently
            self.pending_keys.clear();
            Vec::new()
        } else {
            // Nothing buffered internally — forward to application
            vec![OutputAction::Backspace]
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn lookup_base(&self, key: &str) -> Option<String> {
        self.layout.base_layer.get(key).cloned()
    }

    fn check_modified_layer(&self, key: &str, now: Instant) -> Option<String> {
        for (ms, mod_def) in self.modifier_states.iter().zip(self.layout.modifiers.iter()) {
            let active = if ms.oneshot_pending || ms.toggled {
                true
            } else if ms.held {
                match mod_def.hold_detection {
                    HoldDetection::Timeout => {
                        now.duration_since(ms.pressed_at).as_millis() as u32
                            >= mod_def.hold_timeout_ms
                    }
                    HoldDetection::Interrupt => true,
                }
            } else {
                false
            };

            if active {
                for (mod_id, grid) in &self.layout.modified_layers {
                    if *mod_id == ms.id {
                        if let Some(kana) = grid.get(key) {
                            return Some(kana.clone());
                        }
                    }
                }
            }
        }
        None
    }

    fn modifier_index(&self, key: &str) -> Option<usize> {
        self.layout.modifiers.iter().position(|m| m.key == key)
    }

    /// Return the first mutual chord (symmetric=true) whose key set is a subset of
    /// `held_keys` and that includes `new_key`. Called before the new key enters pending_keys.
    fn find_mutual_chord_match(&self, new_key: &str) -> Option<crate::config::ChordRule> {
        for chord in &self.layout.chords {
            if !chord.symmetric { continue; }
            if !chord.keys.iter().any(|k| k == new_key) { continue; }
            if chord.keys.iter().all(|k| self.held_keys.contains(k)) {
                return Some(chord.clone());
            }
        }
        None
    }

    /// Fire a mutual chord: rewrite any speculative emissions from pending chord keys,
    /// then emit the chord output.
    fn fire_mutual_chord(&mut self, chord: crate::config::ChordRule, _now: Instant) -> Vec<OutputAction> {
        let chord_key_set: HashSet<String> = chord.keys.iter().cloned().collect();

        // Pending keys that are part of this chord had speculative kana emitted.
        let pending_chord: HashSet<String> = self.pending_keys.iter()
            .map(|(k, _)| k.clone())
            .filter(|k| chord_key_set.contains(k))
            .collect();

        let affected = self.tentative_buffer.iter().rev()
            .take_while(|tc| tc.source_keys.iter().any(|k| pending_chord.contains(k)))
            .count();

        let mut actions: Vec<OutputAction> = std::iter::repeat_n(OutputAction::Backspace, affected).collect();
        self.tentative_buffer.truncate(self.tentative_buffer.len() - affected);

        self.pending_keys.retain(|(k, _)| !chord_key_set.contains(k));

        let output = chord.output.clone();
        let source_keys = chord.keys.clone();
        let tokens: Vec<String> = self.resolve_sequence(&output).iter().map(|s| s.to_string()).collect();
        let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let seq = self.emit_sequence(&refs, source_keys, None);
        actions.extend(seq);

        self.update_chord_deadline();
        actions
    }

    fn has_timed_chord_candidate(&self, key: &str) -> bool {
        self.layout.chords.iter().any(|c| !c.symmetric && c.keys.iter().any(|k| k == key) && c.keys.len() > 1)
    }

    fn has_mutual_chord_candidate(&self, key: &str) -> bool {
        self.layout.chords.iter().any(|c| c.symmetric && c.keys.iter().any(|k| k == key) && c.keys.len() > 1)
    }

    fn has_prefix_continuation(&self) -> bool {
        if self.pending_keys.is_empty() {
            return false;
        }
        let first = &self.pending_keys[0].0;
        self.layout
            .prefix_layers
            .iter()
            .any(|(trigger, _)| trigger == first)
    }

    fn has_postfix_candidate(&self, key: &str) -> bool {
        self.layout
            .postfix_layers
            .iter()
            .any(|(trigger, _)| trigger == key)
    }

    fn has_full_sequence_pending(&self, first: &str, second: &str) -> bool {
        self.pending_keys.len() == 2
            && self.pending_keys[0].0 == first
            && self.pending_keys[1].0 == second
    }

    fn chord_window_for(&self, key: &str) -> u32 {
        // Only timed chords (symmetric=false) use a window; mutual chords fire on overlap.
        for chord in &self.layout.chords {
            if chord.symmetric {
                continue;
            }
            if chord.keys.iter().any(|k| k == key) {
                if let Some(w) = chord.window_ms {
                    return w;
                }
            }
        }
        self.layout.settings.chord_window_ms
    }

    fn update_chord_deadline(&mut self) {
        self.chord_deadline = self
            .tentative_buffer
            .iter()
            .filter_map(|tc| tc.rewrite_deadline)
            .min();
    }

    /// Returns true if this token should be emitted immediately (not deferred speculatively).
    fn is_immediate_action_token(token: &str) -> bool {
        token.starts_with('!')
            || token.starts_with("S-")
            || token.starts_with("C-")
            || token.starts_with("A-")
            || token.starts_with("M-")
    }

    /// Check if a key is a Mozc control/editing key (C4).
    /// These keys are sent to Mozc during composition for conversion and navigation.
    /// Keys NOT in this set trigger SubmitThenPassthrough during composition.
    fn is_mozc_control_key(key: &str) -> bool {
        MOZC_CONTROL_KEYS.contains(&key)
    }

    /// Convert an output string from the layout config into the appropriate OutputAction.
    /// Strings starting with modifier prefixes (S-, C-, A-, M-) are SendModifiedKey.
    /// Strings starting with `!` are function keys (e.g. `!Return`, `!Tab`).
    fn make_output_action(output: &str) -> OutputAction {
        let mut mods = 0u8;
        let mut rest = output;
        loop {
            if      let Some(r) = rest.strip_prefix("S-") { mods |= MOD_SHIFT; rest = r; }
            else if let Some(r) = rest.strip_prefix("C-") { mods |= MOD_CTRL;  rest = r; }
            else if let Some(r) = rest.strip_prefix("A-") { mods |= MOD_ALT;   rest = r; }
            else if let Some(r) = rest.strip_prefix("M-") { mods |= MOD_SUPER; rest = r; }
            else { break; }
        }
        if mods != 0 {
            let key = rest.strip_prefix('!').unwrap_or(rest).to_string();
            return OutputAction::SendModifiedKey { key, mods };
        }
        if let Some(key_name) = output.strip_prefix('!') {
            OutputAction::SendFunctionKey(key_name.to_string())
        } else {
            OutputAction::SendKana(output.to_string())
        }
    }

    /// Expand an output string into a token sequence, resolving aliases and
    /// space-separated (quoted-cell) sequences.
    ///
    /// Priority: alias name > space-separated inline sequence > single token.
    fn resolve_sequence<'a>(&'a self, raw: &'a str) -> Vec<&'a str> {
        if let Some(tokens) = self.layout.aliases.get(raw) {
            return tokens.iter().map(String::as_str).collect();
        }
        if raw.contains(' ') {
            return raw.split_whitespace().collect();
        }
        vec![raw]
    }

    /// Emit OutputActions for a resolved token sequence and update tentative_buffer.
    /// Returns the emitted actions.
    fn emit_sequence(
        &mut self,
        tokens: &[&str],
        source_keys: Vec<String>,
        rewrite_deadline: Option<Instant>,
    ) -> Vec<OutputAction> {
        let mut actions = Vec::new();
        for &token in tokens {
            let action = Self::make_output_action(token);
            let skip_tentative = matches!(
                action,
                OutputAction::SendFunctionKey(_) | OutputAction::SendModifiedKey { .. }
            );
            actions.push(action);
            if !skip_tentative {
                self.tentative_buffer.push(TentativeChar {
                    kana: token.to_string(),
                    source_keys: source_keys.clone(),
                    rewrite_deadline,
                    pending_tail: Vec::new(),
                });
            }
        }
        actions
    }

    /// Emit backspaces for all tentative chars (used before kanchoku commit, reset, etc.)
    fn flush_tentative(&mut self) -> Vec<OutputAction> {
        let count = self.tentative_buffer.len();
        self.tentative_buffer.clear();
        std::iter::repeat_n(OutputAction::Backspace, count).collect()
    }

    // ── Mozc output feedback ──────────────────────────────────────────────────

    /// Call this when Mozc reports its mode has changed to CONVERSION.
    pub fn notify_mozc_conversion(&mut self) {
        if self.mozc_mode != MozcMode::Conversion {
            self.tentative_buffer.clear();
            self.pending_keys.clear();
        }
        self.mozc_mode = MozcMode::Conversion;
    }

    /// Call this when Mozc reports COMPOSITION mode (e.g. after commit).
    pub fn notify_mozc_composition(&mut self) {
        self.mozc_mode = MozcMode::Composition;
    }

    // ── Introspection (for tests / CLI) ───────────────────────────────────────

    pub fn tentative_kana_string(&self) -> String {
        self.tentative_buffer
            .iter()
            .map(|tc| tc.kana.as_str())
            .collect()
    }

    pub fn is_direct_active(&self) -> bool {
        self.direct_trigger_state == DirectTriggerState::Active
            || self.layout.meta.mode == LayoutMode::Kanchoku
    }
}

// ── Internal helper ───────────────────────────────────────────────────────────

struct RuleMatch {
    /// Raw output string (single token, alias name, or space-separated sequence)
    output: String,
    source_keys: Vec<String>,
}
