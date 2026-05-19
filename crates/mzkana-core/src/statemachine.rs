use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::config::{Layout, LayoutMode, TapAction, TriggerKind};

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
    /// Notify Mozc that conversion is complete (e.g. Enter)
    MozcSubmit,
}

// ── Internal state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TentativeChar {
    kana: String,
    source_keys: Vec<String>,
    #[allow(dead_code)]
    sent_at: Instant,
    /// Some(deadline) → chord rewrite window; None → sequence (permanent) or confirmed
    rewrite_deadline: Option<Instant>,
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
    /// True while the physical key is held down.
    held: bool,
    /// True when another key was pressed during the hold (interrupt-style detection).
    interrupted: bool,
    /// When the key was pressed (for timeout detection).
    pressed_at: Instant,
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
    /// Keys currently held for the direct trigger (for hold detection).
    direct_trigger_held: bool,
    direct_trigger_interrupted: bool,
    mozc_mode: MozcMode,
    /// Chord timer: tracks the earliest rewrite_deadline in tentative_buffer.
    chord_deadline: Option<Instant>,
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
            })
            .collect();

        Self {
            layout,
            pending_keys: Vec::new(),
            tentative_buffer: Vec::new(),
            modifier_states,
            direct_trigger_state: DirectTriggerState::Inactive,
            direct_trigger_held: false,
            direct_trigger_interrupted: false,
            mozc_mode: MozcMode::Composition,
            chord_deadline: None,
        }
    }

    /// Reset all transient state (e.g. on focus change with reset policy).
    pub fn reset(&mut self) -> Vec<OutputAction> {
        let actions = self.flush_tentative();
        self.pending_keys.clear();
        self.mozc_mode = MozcMode::Composition;
        self.chord_deadline = None;
        actions
    }

    /// Call periodically to fire chord timers.
    /// Returns any actions from expired tentative chars (they become confirmed).
    pub fn tick(&mut self, now: Instant) -> Vec<OutputAction> {
        if let Some(deadline) = self.chord_deadline {
            if now >= deadline {
                // Confirm all expired tentative chars.
                for tc in &mut self.tentative_buffer {
                    if tc.rewrite_deadline.map_or(false, |d| now >= d) {
                        tc.rewrite_deadline = None;
                    }
                }
                self.update_chord_deadline();
            }
        }
        Vec::new()
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
            self.modifier_states[idx].held = true;
            self.modifier_states[idx].interrupted = false;
            self.modifier_states[idx].pressed_at = now;
            return Vec::new();
        }

        // Check if key is a direct trigger
        if self.is_direct_trigger_key(key) {
            return self.handle_direct_trigger_down(key, now);
        }

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
        if let Some(mod_output) = self.check_modified_layer(key) {
            // Modified layer: output is final, no speculative needed
            let mut actions = Vec::new();
            actions.push(OutputAction::SendKana(mod_output.clone()));
            self.tentative_buffer.push(TentativeChar {
                kana: mod_output,
                source_keys: vec![key.to_string()],
                sent_at: now,
                rewrite_deadline: None,
            });
            return actions;
        }

        // Normal key: speculative emit
        self.pending_keys.push((key.to_string(), now));
        self.speculative_emit(key, shift, now)
    }

    // ── Key Up ────────────────────────────────────────────────────────────────

    fn process_key_up(&mut self, key: &str, now: Instant) -> Vec<OutputAction> {
        // Modifier key released
        if let Some(idx) = self.modifier_index(key) {
            let was_interrupted = self.modifier_states[idx].interrupted;
            self.modifier_states[idx].held = false;

            let modifier_def = &self.layout.modifiers[idx];
            if !was_interrupted {
                // Tap action
                match modifier_def.tap_action {
                    TapAction::SendKey | TapAction::Passthrough => {
                        return vec![OutputAction::Passthrough(key.to_string())];
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

    fn speculative_emit(&mut self, key: &str, shift: bool, now: Instant) -> Vec<OutputAction> {
        let mut actions = Vec::new();

        // Resolve what chord / prefix / postfix candidates exist
        let has_chord_candidate = self.has_chord_candidate(key);
        let has_prefix_continuation = self.has_prefix_continuation();
        let has_postfix_candidate = self.has_postfix_candidate(key);

        // Determine base kana from shift state
        let base_kana = if shift {
            self.lookup_base_shifted(key)
        } else {
            self.lookup_base(key)
        };

        // Check if a prefix sequence already in pending resolves to a new rule
        if let Some(resolved) = self.try_resolve_pending() {
            return self.apply_rule_match(resolved, now);
        }

        // Check for postfix match: current key is trigger for postfix of previous char
        if let Some(resolved) = self.try_resolve_postfix(key) {
            return self.apply_rule_match(resolved, now);
        }

        let Some(kana) = base_kana else {
            // No base assignment for this key (e.g. pure trigger key) — don't send tentative
            return actions;
        };

        // Send speculative kana
        actions.push(OutputAction::SendKana(kana.clone()));

        let rewrite_deadline = if has_chord_candidate {
            let window = self.chord_window_for(key);
            let deadline = now + Duration::from_millis(window as u64);
            self.chord_deadline = Some(match self.chord_deadline {
                Some(existing) => existing.min(deadline),
                None => deadline,
            });
            Some(deadline)
        } else if has_prefix_continuation || has_postfix_candidate {
            None // sequence: permanently rewritable until resolved
        } else {
            // Single hit, no continuations → immediately confirmed
            self.pending_keys.clear();
            None
        };

        self.tentative_buffer.push(TentativeChar {
            kana,
            source_keys: vec![key.to_string()],
            sent_at: now,
            rewrite_deadline,
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
            let chord_set: BTreeSet<&str> = chord.keys.iter().map(String::as_str).collect();
            if chord_set == key_set {
                // For non-symmetric chords, verify order matches
                if !chord.symmetric && keys.len() == chord.keys.len() {
                    if keys.iter().zip(chord.keys.iter()).any(|(a, b)| a != b) {
                        continue;
                    }
                }
                return Some(RuleMatch {
                    output: chord.output.clone(),
                    source_keys: keys.to_vec(),
                });
            }
        }
        None
    }

    fn apply_rule_match(&mut self, rule: RuleMatch, now: Instant) -> Vec<OutputAction> {
        let mut actions = Vec::new();

        // Find tentative chars to rewrite
        let affected_count = self
            .tentative_buffer
            .iter()
            .rev()
            .take_while(|tc| {
                tc.source_keys
                    .iter()
                    .any(|k| rule.source_keys.contains(k))
            })
            .count();

        for _ in 0..affected_count {
            actions.push(OutputAction::Backspace);
        }
        self.tentative_buffer
            .truncate(self.tentative_buffer.len() - affected_count);

        actions.push(OutputAction::SendKana(rule.output.clone()));

        self.tentative_buffer.push(TentativeChar {
            kana: rule.output,
            source_keys: rule.source_keys,
            sent_at: now,
            rewrite_deadline: None,
        });

        self.pending_keys.clear();
        self.update_chord_deadline();
        actions
    }

    // ── Direct (kanchoku) mode ────────────────────────────────────────────────

    fn process_direct_key(&mut self, key: &str, now: Instant) -> Vec<OutputAction> {
        self.pending_keys.push((key.to_string(), now));
        let pending: Vec<String> = self.pending_keys.iter().map(|(k, _)| k.clone()).collect();

        // Check for complete match
        for rule in &self.layout.directs.clone() {
            if rule.sequence == pending {
                let kanji = rule.output.clone();
                self.pending_keys.clear();

                let mut actions = Vec::new();
                // Flush any remaining tentative kana first
                let flush = self.flush_tentative();
                if !flush.is_empty() {
                    actions.extend(flush);
                    actions.push(OutputAction::MozcSubmit);
                }
                actions.push(OutputAction::CommitDirect(kanji));
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
            .map_or(false, |dt| dt.keys.iter().any(|k| k == key))
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
                self.direct_trigger_held = true;
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

    fn handle_direct_trigger_up(&mut self, _key: &str, _now: Instant) -> Vec<OutputAction> {
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

        self.direct_trigger_held = false;
        let was_interrupted = self.direct_trigger_interrupted;
        self.direct_trigger_state = DirectTriggerState::Inactive;
        self.pending_keys.clear();

        if !was_interrupted {
            match tap_action {
                TapAction::Passthrough | TapAction::SendKey => {
                    // tap: pass through the trigger key
                }
                TapAction::None => {}
            }
        }
        Vec::new()
    }

    // ── Backspace handling ────────────────────────────────────────────────────

    fn handle_backspace(&mut self) -> Vec<OutputAction> {
        if !self.tentative_buffer.is_empty() {
            self.tentative_buffer.pop();
            self.pending_keys.clear();
        }
        vec![OutputAction::Backspace]
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn lookup_base(&self, key: &str) -> Option<String> {
        self.layout.base_layer.get(key).cloned()
    }

    fn lookup_base_shifted(&self, key: &str) -> Option<String> {
        // For now, shift layer is not separately defined; fall back to base
        self.layout.base_layer.get(key).cloned()
    }

    fn check_modified_layer(&self, key: &str) -> Option<String> {
        for ms in &self.modifier_states {
            if ms.held {
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

    fn has_chord_candidate(&self, key: &str) -> bool {
        self.layout
            .chords
            .iter()
            .any(|c| c.keys.iter().any(|k| k == key) && c.keys.len() > 1)
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
        // Check for per-chord override
        for chord in &self.layout.chords {
            if chord.keys.iter().any(|k| k == key) {
                if let Some(w) = chord.window_ms {
                    return w;
                }
                if chord.symmetric {
                    return self.layout.settings.mutual_window_ms;
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

    /// Emit backspaces for all tentative chars (used before kanchoku commit, reset, etc.)
    fn flush_tentative(&mut self) -> Vec<OutputAction> {
        let count = self.tentative_buffer.len();
        self.tentative_buffer.clear();
        std::iter::repeat(OutputAction::Backspace).take(count).collect()
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
    output: String,
    source_keys: Vec<String>,
}
