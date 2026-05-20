use std::time::Instant;

use crate::{config::load_layout, statemachine::*};

const TSUKI: &str = include_str!("../../../layouts/tsuki-2-263.toml");
const SHIN_GETA: &str = include_str!("../../../layouts/shin-geta.toml");
const NAGINATA: &str = include_str!("../../../layouts/naginata-v17.toml");
const T_CODE: &str = include_str!("../../../layouts/t-code.toml");

fn sm(toml: &str) -> StateMachine {
    StateMachine::new(load_layout(toml).expect("load layout"))
}

fn press_seq(sm: &mut StateMachine, keys: &[&str]) -> Vec<OutputAction> {
    let now = Instant::now();
    keys.iter()
        .flat_map(|k| sm.process(InputEvent::down(*k), now))
        .collect()
}

// ── Config loading ────────────────────────────────────────────────────────────

#[test]
fn load_tsuki() {
    let l = load_layout(TSUKI).unwrap();
    assert_eq!(l.meta.name, "月配列2-263式");
    assert!(!l.base_layer.is_empty());
    assert!(!l.prefix_layers.is_empty());
}

#[test]
fn load_shin_geta() {
    let l = load_layout(SHIN_GETA).unwrap();
    assert_eq!(l.chords.len(), 13);
}

#[test]
fn load_t_code() {
    let l = load_layout(T_CODE).unwrap();
    assert_eq!(l.directs.len(), 13);
}

// ── Grid parsing ──────────────────────────────────────────────────────────────

#[test]
fn grid_number_row_mapping() {
    let l = load_layout(SHIN_GETA).unwrap();
    // Row 0 maps to physical keys 1..0,minus,equal
    // shin-geta: key "1" → ぁ, key "6" → ゃ
    assert_eq!(l.base_layer.get("1").map(|s| s.as_str()), Some("ぁ"));
    assert_eq!(l.base_layer.get("6").map(|s| s.as_str()), Some("ゃ"));
    // Row 1 keys still work: "q" → た
    assert_eq!(l.base_layer.get("q").map(|s| s.as_str()), Some("た"));
}

#[test]
fn number_row_key_sends_kana() {
    let mut m = sm(SHIN_GETA);
    let actions = press_seq(&mut m, &["1"]);
    assert!(actions.contains(&OutputAction::SendKana("ぁ".to_string())));
}

#[test]
fn yen_key_mapping() {
    let l = load_layout(SHIN_GETA).unwrap();
    // yen key → ゛ (voiced sound mark)
    assert_eq!(l.base_layer.get("yen").map(|s| s.as_str()), Some("゛"));
}

#[test]
fn grid_row_mapping() {
    let l = load_layout(TSUKI).unwrap();
    // Row 1 (q..p): q → 。
    assert_eq!(l.base_layer.get("q").map(|s| s.as_str()), Some("。"));
    // Row 2 (a..;): a → う
    assert_eq!(l.base_layer.get("a").map(|s| s.as_str()), Some("う"));
    // Row 2: s → し
    assert_eq!(l.base_layer.get("s").map(|s| s.as_str()), Some("し"));
}

// ── Single key → kana ─────────────────────────────────────────────────────────

#[test]
fn single_key_base_layer() {
    let mut m = sm(TSUKI);
    let actions = press_seq(&mut m, &["q"]);
    assert_eq!(actions, vec![OutputAction::SendKana("。".to_string())]);
    assert_eq!(m.tentative_kana_string(), "。");
}

#[test]
fn single_key_home_row() {
    let mut m = sm(TSUKI);
    let actions = press_seq(&mut m, &["a"]);
    assert!(actions.contains(&OutputAction::SendKana("う".to_string())));
}

// ── Prefix shift (月配列) ─────────────────────────────────────────────────────

#[test]
fn prefix_shift_resolves() {
    let mut m = sm(TSUKI);
    // d (base=て) then w: prefix_d grid row1 col w → え
    let actions = press_seq(&mut m, &["d", "w"]);
    // Should contain: send_kana(て), backspace, send_kana(え)
    assert!(actions.contains(&OutputAction::Backspace));
    // After resolution preedit should be just the resolved kana
    assert_ne!(m.tentative_kana_string(), "て");
}

#[test]
fn prefix_trigger_alone_stays_base() {
    let mut m = sm(TSUKI);
    // d alone: speculative sends "て", no prefix resolved
    let actions = press_seq(&mut m, &["d"]);
    assert!(actions.contains(&OutputAction::SendKana("て".to_string())));
}

// ── Chord (新下駄) ────────────────────────────────────────────────────────────

#[test]
fn chord_rewrite() {
    let mut m = sm(SHIN_GETA);
    let now = Instant::now();
    // f: speculative き
    let a1 = m.process(InputEvent::down("f"), now);
    assert!(a1.contains(&OutputAction::SendKana("き".to_string())));
    // j (within chord window): chord [f,j] → を, rewrite
    let a2 = m.process(InputEvent::down("j"), now);
    assert!(a2.contains(&OutputAction::Backspace));
    assert!(a2.contains(&OutputAction::SendKana("を".to_string())));
    assert_eq!(m.tentative_kana_string(), "を");
}

#[test]
fn chord_symmetric_either_order() {
    let mut m = sm(NAGINATA);
    let now = Instant::now();
    // [[chord]] keys=["f","j"] symmetric=true output="が"
    // Try j first, then f (reverse order from definition)
    let a1 = m.process(InputEvent::down("j"), now);
    // base row2: a=ろ s=け d=と f=か g=っ h=く j=あ …
    // So j → あ speculatively
    assert!(!a1.is_empty());
    let a2 = m.process(InputEvent::down("f"), now);
    assert!(a2.contains(&OutputAction::Backspace));
    assert!(a2.contains(&OutputAction::SendKana("が".to_string())));
}

// ── Kanchoku / T-code ─────────────────────────────────────────────────────────

#[test]
fn kanchoku_two_stroke() {
    let mut m = sm(T_CODE);
    let actions = press_seq(&mut m, &["k", "j"]);
    assert!(actions.contains(&OutputAction::CommitDirect("日".to_string())));
    assert_eq!(m.tentative_kana_string(), ""); // preedit clear
}

#[test]
fn kanchoku_no_partial_display() {
    let mut m = sm(T_CODE);
    let now = Instant::now();
    // First stroke: nothing should appear
    let a1 = m.process(InputEvent::down("k"), now);
    assert!(a1.is_empty(), "kanchoku first stroke must be silent: {a1:?}");
    // Second stroke: commit
    let a2 = m.process(InputEvent::down("j"), now);
    assert!(a2.contains(&OutputAction::CommitDirect("日".to_string())));
}

#[test]
fn kanchoku_invalid_sequence_discarded() {
    let mut m = sm(T_CODE);
    // "k z" has no direct rule → pending cleared, nothing committed
    let actions = press_seq(&mut m, &["k", "z"]);
    assert!(!actions.iter().any(|a| matches!(a, OutputAction::CommitDirect(_))));
}

// ── BackSpace ─────────────────────────────────────────────────────────────────

#[test]
fn backspace_pops_tentative() {
    let mut m = sm(TSUKI);
    let now = Instant::now();
    m.process(InputEvent::down("q"), now); // 。
    m.process(InputEvent::down("w"), now); // か
    assert_eq!(m.tentative_kana_string(), "。か");
    let bs = m.process(InputEvent::down("bs"), now);
    assert!(bs.contains(&OutputAction::Backspace));
    assert_eq!(m.tentative_kana_string(), "。");
}

// ── Center-shift modifier (薙刀式) ────────────────────────────────────────────

#[test]
fn center_shift_modifier_layer() {
    let mut m = sm(NAGINATA);
    let now = Instant::now();
    // Hold space (center modifier), then press w
    m.process(InputEvent::down("space"), now);
    let a = m.process(InputEvent::down("w"), now);
    // center_shift layer row1 w → め
    assert!(a.contains(&OutputAction::SendKana("め".to_string())));
}

// ── Conflict detection ────────────────────────────────────────────────────────

#[test]
fn duplicate_direct_rules_error() {
    let toml = r#"
[meta]
name = "test"
mode = "kanchoku"
schema = 1
[[direct]]
sequence = ["a", "b"]
output = "X"
[[direct]]
sequence = ["a", "b"]
output = "Y"
"#;
    assert!(load_layout(toml).is_err());
}

#[test]
fn duplicate_direct_same_output_ok() {
    let toml = r#"
[meta]
name = "test"
mode = "kanchoku"
schema = 1
[[direct]]
sequence = ["a", "b"]
output = "X"
[[direct]]
sequence = ["a", "b"]
output = "X"
"#;
    assert!(load_layout(toml).is_ok());
}

// ── Fix regressions ───────────────────────────────────────────────────────────

#[test]
fn unknown_row_label_is_error() {
    // Row label "r1" is not 0-3 → must fail
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
r1 あ い
"""
"#;
    let err = load_layout(toml).unwrap_err();
    assert!(err.to_string().contains("r1"), "error should mention the bad label");
}

#[test]
fn backspace_with_pending_only_no_external_bs() {
    // When only pending_keys exist (e.g. mid-prefix), BS should not forward externally
    let mut m = sm(TSUKI);
    let now = Instant::now();
    // Press 'd' — it has prefix candidates, so only goes into pending (speculative sends て)
    m.process(InputEvent::down("d"), now);
    // Pop the tentative 'て' so tentative_buffer is empty but pending still in progress
    m.process(InputEvent::down("bs"), now); // pops tentative+pending
    // Now both are clear; next BS should forward externally
    let bs = m.process(InputEvent::down("bs"), now);
    assert!(bs.contains(&OutputAction::Backspace));
}

#[test]
fn reset_clears_modifier_state() {
    let mut m = sm(NAGINATA);
    let now = Instant::now();
    // Activate center modifier
    m.process(InputEvent::down("space"), now);
    // Reset — modifier should be cleared
    m.reset();
    // Now pressing a key should NOT go through the modified layer
    let a = m.process(InputEvent::down("q"), now);
    // After reset, space modifier is gone; q should produce base kana (ば), not ぁ
    assert!(!a.contains(&OutputAction::SendKana("ぁ".to_string())));
}

#[test]
fn function_key_in_chord() {
    // A chord whose output is a function key should emit SendFunctionKey, not SendKana
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
1 あ い
"""
[[chord]]
keys   = ["q", "w"]
output = "!Return"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("q"), now);
    let actions = m.process(InputEvent::down("w"), now);
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "chord !Return should produce SendFunctionKey: {actions:?}"
    );
    // Must not appear in preedit
    assert_eq!(m.tentative_kana_string(), "");
}

#[test]
fn function_key_in_base_layer() {
    // A base layer cell with !Tab should emit SendFunctionKey immediately
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
1 あ !Tab
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    let actions = m.process(InputEvent::down("w"), now);
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Tab".to_string())),
        "{actions:?}"
    );
    assert_eq!(m.tentative_kana_string(), "");
}

#[test]
fn invalid_function_key_name_is_error() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[chord]]
keys   = ["a", "b"]
output = "!SuperFakeKey"
"#;
    assert!(load_layout(toml).is_err());
}

#[test]
fn invalid_function_key_in_multi_token_output_is_error() {
    // An output like "、 !SuperFakeKey" splits into two tokens;
    // the second token must be validated even though the whole string
    // does not start with '!'.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q
1 "、 !SuperFakeKey"
"""
"#;
    assert!(load_layout(toml).is_err());
}

#[test]
fn invalid_function_key_in_alias_is_error() {
    // An alias value containing an invalid !-prefixed token must be rejected.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[alias]]
bad_alias = "、 !SuperFakeKey"
"#;
    assert!(load_layout(toml).is_err());
}

#[test]
fn empty_alias_sequence_is_error() {
    // An alias mapped to an empty string must fail validation to prevent
    // a panic at tokens[0] in the state machine.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[alias]]
empty_alias = ""
"#;
    let err = load_layout(toml).unwrap_err();
    assert!(
        err.to_string().contains("empty_alias"),
        "error should mention the alias name: {err}"
    );
}

#[test]
fn direct_trigger_tap_returns_passthrough() {
    // Hybrid layout with direct trigger; tapping the trigger with no other key
    // should emit a Passthrough action for the trigger key.
    let toml = r#"
[meta]
name = "test"
mode = "hybrid"
schema = 1
[direct_trigger]
keys       = ["henkan"]
kind       = "hold"
tap_action = "passthrough"
[[direct]]
sequence = ["a", "b"]
output   = "日"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // Press and release trigger without pressing any other key
    m.process(InputEvent::down("henkan"), now);
    let actions = m.process(InputEvent::up("henkan"), now);
    assert!(
        actions.contains(&OutputAction::Passthrough("henkan".to_string())),
        "tap should produce passthrough: {actions:?}"
    );
}

// ── Alias / quoted-sequence features ─────────────────────────────────────────

#[test]
fn alias_single_key_no_chord_emits_all_tokens() {
    // A key with an alias output and no chord candidate emits all tokens immediately.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
1 ku_ret い
"""
[[alias]]
ku_ret = "、 !Return"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    let actions = m.process(InputEvent::down("q"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("、".to_string())),
        "should emit kana: {actions:?}"
    );
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "should emit function key: {actions:?}"
    );
}

#[test]
fn quoted_cell_no_chord_emits_all_tokens() {
    // A grid cell with "kana !FKey" (quoted) and no chord candidate emits both immediately.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q
1 "。 !Return"
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    let actions = m.process(InputEvent::down("q"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("。".to_string())),
        "kana: {actions:?}"
    );
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "fkey: {actions:?}"
    );
}

#[test]
fn alias_with_chord_candidate_defers_tail() {
    // When the key that carries a multi-token alias output is also part of a chord,
    // only the first kana is emitted speculatively; the tail is held until confirmed.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
1 ku_ret い
"""
[[chord]]
keys   = ["q", "w"]
output = "う"
window_ms = 50
[[alias]]
ku_ret = "、 !Return"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();

    // Press q — speculative: only 、 is emitted; !Return is deferred
    let a1 = m.process(InputEvent::down("q"), now);
    assert!(
        a1.contains(&OutputAction::SendKana("、".to_string())),
        "speculative kana emitted: {a1:?}"
    );
    assert!(
        !a1.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "tail must NOT be emitted yet: {a1:?}"
    );

    // tick() after deadline: tail should now be emitted
    let later = now + std::time::Duration::from_millis(100);
    let tick_actions = m.tick(later);
    assert!(
        tick_actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "tail emitted after deadline: {tick_actions:?}"
    );
}

#[test]
fn alias_chord_fires_no_double_emission() {
    // When the chord fires, the speculative kana is rewritten; the deferred tail
    // must NOT be emitted (it is discarded along with the speculative char).
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. q w
1 ku_ret い
"""
[[chord]]
keys   = ["q", "w"]
output = "う"
window_ms = 50
[[alias]]
ku_ret = "、 !Return"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();

    // q: speculative 、
    m.process(InputEvent::down("q"), now);
    // w (within window): chord fires → rewrite to う
    let a2 = m.process(InputEvent::down("w"), now);
    assert!(
        a2.contains(&OutputAction::Backspace),
        "should rewrite speculative: {a2:?}"
    );
    assert!(
        a2.contains(&OutputAction::SendKana("う".to_string())),
        "chord output: {a2:?}"
    );
    // !Return must NOT appear — the pending tail was discarded
    assert!(
        !a2.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "tail must be discarded on rewrite: {a2:?}"
    );

    // tick() after deadline: nothing extra from the discarded tail
    let later = now + std::time::Duration::from_millis(100);
    let tick_actions = m.tick(later);
    assert!(
        !tick_actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "no tail after rewrite: {tick_actions:?}"
    );
}

// ── tap_action = base_kana ────────────────────────────────────────────────────

// f is row-2 index 3 (a s d f g …); g is index 4.
// Header column labels are visual-only; physical keys come from row position.
const DUAL_ROLE_LAYOUT: &str = r#"
[meta]
name   = "dual-role test"
mode   = "kana"
schema = 1

[[modifier]]
id             = "shift_f"
key            = "f"
kind           = "hold"
hold_detection = "interrupt"
tap_action     = "base_kana"

[[layer]]
id   = "base"
kind = "single"
grid = """
. a    s    d    f    g
2 ＿   ＿   ＿   か   き
"""

[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "shift_f"
grid     = """
. a    s    d    f    g
2 ＿   ＿   ＿   ＿   ぎ
"""
"#;

#[test]
fn dual_role_tap_emits_base_kana() {
    // Tapping the modifier key alone should emit its base-layer kana.
    let layout = load_layout(DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("f"), now);
    let actions = m.process(InputEvent::up("f"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("か".to_string())),
        "tap should emit base kana: {actions:?}"
    );
    assert_eq!(m.tentative_kana_string(), "か");
}

#[test]
fn dual_role_hold_emits_shifted_kana() {
    // Holding the modifier key then pressing another key should emit the shifted kana.
    let layout = load_layout(DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("f"), now);      // modifier held
    let actions = m.process(InputEvent::down("g"), now); // shifted layer → ぎ
    assert!(
        actions.contains(&OutputAction::SendKana("ぎ".to_string())),
        "hold+key should emit shifted kana: {actions:?}"
    );
    // Releasing modifier after use must not emit base kana (was_interrupted=true)
    let up = m.process(InputEvent::up("f"), now);
    assert!(
        !up.contains(&OutputAction::SendKana("か".to_string())),
        "release after hold must not emit tap kana: {up:?}"
    );
}

// ── send_key alias, tap_action = "output", ModifierKind::Toggle ───────────────

#[test]
fn send_key_is_alias_for_passthrough() {
    // "send_key" should deserialize identically to "passthrough".
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id         = "m"
key        = "space"
tap_action = "send_key"
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("space"), now);
    let actions = m.process(InputEvent::up("space"), now);
    assert!(
        actions.contains(&OutputAction::Passthrough("space".to_string())),
        "send_key tap should produce passthrough: {actions:?}"
    );
}

#[test]
fn tap_output_emits_specified_string() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id         = "m"
key        = "space"
tap_action = "output"
tap_output = "　"
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "m"
grid     = """
. q
1 あ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("space"), now);
    let actions = m.process(InputEvent::up("space"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("　".to_string())),
        "tap_output should emit the specified string: {actions:?}"
    );
}

#[test]
fn tap_output_with_alias_resolves() {
    // tap_output can reference an alias name.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[alias]]
ku_ret = "、 !Return"
[[modifier]]
id         = "m"
key        = "space"
tap_action = "output"
tap_output = "ku_ret"
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "m"
grid     = """
. q
1 あ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("space"), now);
    let actions = m.process(InputEvent::up("space"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("、".to_string())),
        "alias should expand: {actions:?}"
    );
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "alias tail should emit: {actions:?}"
    );
}

#[test]
fn tap_output_missing_is_error() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id         = "m"
key        = "space"
tap_action = "output"
"#;
    let err = load_layout(toml).unwrap_err();
    assert!(
        err.to_string().contains("tap_output"),
        "error should mention tap_output: {err}"
    );
}

#[test]
fn base_kana_resolves_alias() {
    // base_kana tap_action should expand alias names via resolve_sequence.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[alias]]
ku_ret = "、 !Return"
[[modifier]]
id             = "m"
key            = "f"
tap_action     = "base_kana"
hold_detection = "interrupt"
[[layer]]
id   = "base"
kind = "single"
grid = """
. a    s    d    f
2 ＿   ＿   ＿   ku_ret
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "m"
grid     = """
. a    s    d    f    g
2 ＿   ＿   ＿   ＿   ぎ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("f"), now);
    let actions = m.process(InputEvent::up("f"), now);
    // base layer f → alias "ku_ret" → "、 !Return"
    assert!(
        actions.contains(&OutputAction::SendKana("、".to_string())),
        "base_kana alias should expand to kana: {actions:?}"
    );
    assert!(
        actions.contains(&OutputAction::SendFunctionKey("Return".to_string())),
        "base_kana alias should expand to function key: {actions:?}"
    );
}

#[test]
fn toggle_modifier_activates_layer() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id   = "caps"
key  = "caps_lock"
kind = "toggle"
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "caps"
grid     = """
. q
1 あ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // First press: toggle ON
    m.process(InputEvent::down("caps_lock"), now);
    m.process(InputEvent::up("caps_lock"), now);
    // Now q should come from shifted layer
    let actions = m.process(InputEvent::down("q"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("あ".to_string())),
        "toggle ON: shifted layer active: {actions:?}"
    );
}

#[test]
fn toggle_modifier_deactivates_on_second_press() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id   = "caps"
key  = "caps_lock"
kind = "toggle"
[[layer]]
id   = "base"
kind = "single"
grid = """
. q
1 い
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "caps"
grid     = """
. q
1 あ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // Toggle ON
    m.process(InputEvent::down("caps_lock"), now);
    m.process(InputEvent::up("caps_lock"), now);
    // Toggle OFF
    m.process(InputEvent::down("caps_lock"), now);
    m.process(InputEvent::up("caps_lock"), now);
    // Now q should come from base layer (not shifted)
    let actions = m.process(InputEvent::down("q"), now);
    assert!(
        actions.contains(&OutputAction::SendKana("い".to_string())),
        "toggle OFF: base layer active: {actions:?}"
    );
    assert!(
        !actions.contains(&OutputAction::SendKana("あ".to_string())),
        "shifted layer must not be active: {actions:?}"
    );
}
