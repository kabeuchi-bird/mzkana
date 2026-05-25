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
    // Row 1 (q..p): q → そ
    assert_eq!(l.base_layer.get("q").map(|s| s.as_str()), Some("そ"));
    // Row 2 (a..;): a → は
    assert_eq!(l.base_layer.get("a").map(|s| s.as_str()), Some("は"));
    // Row 2: s → か
    assert_eq!(l.base_layer.get("s").map(|s| s.as_str()), Some("か"));
}

#[test]
fn extended_keys_in_numbered_rows() {
    // ROW1: tab(0) q(1) ... p(10) bracketleft(11) bracketright(12) backslash(13)
    // ROW2: caps_lock(0) a(1) ... semicolon(10) quote(11)
    // ROW3: lshift(0) z(1) ... slash(10) intlro(11)
    let src = r#"
[meta]
name = "ext"
mode = "kanchoku"
[[layer]]
id   = "base"
kind = "single"
grid = """
. c0 c1 c2 c3 c4 c5 c6 c7 c8 c9 c10 c11 c12 c13
1  あ い う え お か き く け こ さ  し  す  せ
2  た ち つ て と な に ぬ ね の は  ひ  ＿  ＿
3  ま み む め も や ゆ よ ら り る  ろ  ＿  ＿
"""
"#;
    let l = load_layout(src).unwrap();
    assert_eq!(l.base_layer.get("tab").map(|s| s.as_str()),          Some("あ")); // ROW1[0]
    assert_eq!(l.base_layer.get("q").map(|s| s.as_str()),            Some("い")); // ROW1[1]
    assert_eq!(l.base_layer.get("bracketleft").map(|s| s.as_str()),  Some("し")); // ROW1[11]
    assert_eq!(l.base_layer.get("bracketright").map(|s| s.as_str()), Some("す")); // ROW1[12]
    assert_eq!(l.base_layer.get("backslash").map(|s| s.as_str()),    Some("せ")); // ROW1[13]
    assert_eq!(l.base_layer.get("caps_lock").map(|s| s.as_str()),    Some("た")); // ROW2[0]
    assert_eq!(l.base_layer.get("a").map(|s| s.as_str()),            Some("ち")); // ROW2[1]
    assert_eq!(l.base_layer.get("quote").map(|s| s.as_str()),        Some("ひ")); // ROW2[11]
    assert_eq!(l.base_layer.get("lshift").map(|s| s.as_str()),       Some("ま")); // ROW3[0]
    assert_eq!(l.base_layer.get("z").map(|s| s.as_str()),            Some("み")); // ROW3[1]
    assert_eq!(l.base_layer.get("intlro").map(|s| s.as_str()),       Some("ろ")); // ROW3[11]
}

// ── XX passthrough cell ───────────────────────────────────────────────────────

#[test]
fn xx_cell_base_layer_emits_passthrough() {
    let src = r#"
[meta]
name = "xx-test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. . q w
1 ＿ あ XX
"""
"#;
    let l = load_layout(src).unwrap();
    let mut m = StateMachine::new(l);
    let now = Instant::now();
    // q → SendKana("あ") as normal
    let a = m.process(InputEvent::down("q"), now);
    assert!(a.contains(&OutputAction::SendKana("あ".to_string())), "{a:?}");
    // w → Passthrough("w") because cell is XX
    let b = m.process(InputEvent::down("w"), now);
    assert!(b.contains(&OutputAction::Passthrough("w".to_string())), "{b:?}");
    assert!(!b.iter().any(|x| matches!(x, OutputAction::SendKana(_))), "{b:?}");
}

#[test]
fn xx_cell_modified_layer_emits_passthrough() {
    let src = r#"
[meta]
name = "xx-mod"
mode = "kana"
schema = 1
[[modifier]]
id   = "m"
key  = "space"
kind = "hold"
hold_detection = "interrupt"
[[layer]]
id   = "base"
kind = "single"
grid = """
. . q
1 ＿ あ
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "m"
grid     = """
. . q
1 ＿ XX
"""
"#;
    let l = load_layout(src).unwrap();
    let mut m = StateMachine::new(l);
    let now = Instant::now();
    // Hold space then press q → modified layer XX → Passthrough("q")
    m.process(InputEvent::down("space"), now);
    let a = m.process(InputEvent::down("q"), now);
    assert!(a.contains(&OutputAction::Passthrough("q".to_string())), "{a:?}");
    assert!(!a.iter().any(|x| matches!(x, OutputAction::SendKana(_))), "{a:?}");
}

// ── Single key → kana ─────────────────────────────────────────────────────────

#[test]
fn single_key_base_layer() {
    let mut m = sm(TSUKI);
    let actions = press_seq(&mut m, &["q"]);
    assert_eq!(actions, vec![OutputAction::SendKana("そ".to_string())]);
    assert_eq!(m.tentative_kana_string(), "そ");
}

#[test]
fn single_key_home_row() {
    let mut m = sm(TSUKI);
    let actions = press_seq(&mut m, &["a"]);
    assert!(actions.contains(&OutputAction::SendKana("は".to_string())));
}

// ── Prefix shift (月配列) ─────────────────────────────────────────────────────

#[test]
fn prefix_shift_resolves() {
    let mut m = sm(TSUKI);
    // d (base=＜) then w: prefix_d grid row1 col w → ひ
    let actions = press_seq(&mut m, &["d", "w"]);
    // Should contain: send_kana(＜), backspace, send_kana(ひ)
    assert!(actions.contains(&OutputAction::Backspace));
    // After resolution preedit should be just the resolved kana
    assert_ne!(m.tentative_kana_string(), "＜");
}

#[test]
fn prefix_trigger_alone_stays_base() {
    let mut m = sm(TSUKI);
    // d alone: speculative sends base output "＜", no prefix resolved
    let actions = press_seq(&mut m, &["d"]);
    assert!(actions.contains(&OutputAction::SendKana("＜".to_string())));
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

// ── Mutual chord / simultaneous chord ────────────────────────────────────────

const MUTUAL_CHORD_LAYOUT: &str = r#"
[meta]
name = "mutual-test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. .  q  w  e  r  t  y  u
1 ＿ ＿ ＿ ＿ る ＿ ＿ す
"""
[[chord]]
keys      = ["f", "j"]
output    = "が"
symmetric = true
[[chord]]
keys      = ["e", "j"]
output    = "で"
symmetric = true
[[chord]]
keys      = ["r", "u"]
output    = "だ"
symmetric = false
"#;

#[test]
fn mutual_chord_fires_on_overlap() {
    // symmetric=true: fires immediately when both keys are physically held.
    let layout = load_layout(MUTUAL_CHORD_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // j down: no chord yet (f not held)
    let a1 = m.process(InputEvent::down("j"), now);
    // j has no base layer mapping, so no speculative output
    assert!(a1.is_empty(), "j alone should produce no output: {a1:?}");
    // f down: now j+f are both held → mutual chord fires immediately
    let a2 = m.process(InputEvent::down("f"), now);
    assert!(a2.contains(&OutputAction::SendKana("が".to_string())), "{a2:?}");
    assert_eq!(m.tentative_kana_string(), "が");
}

#[test]
fn mutual_chord_rollover() {
    // After (f j)→「が」fires with j still held, pressing e fires (e j)→「で」.
    let layout = load_layout(MUTUAL_CHORD_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("j"), now);
    let a_fg = m.process(InputEvent::down("f"), now);
    assert!(a_fg.contains(&OutputAction::SendKana("が".to_string())), "{a_fg:?}");
    m.process(InputEvent::up("f"), now);
    // j is still held; pressing e triggers (e j) mutual chord
    let a_ej = m.process(InputEvent::down("e"), now);
    assert!(a_ej.contains(&OutputAction::SendKana("で".to_string())), "rollover should fire (e,j): {a_ej:?}");
    // 「が」must NOT have been backspaced
    assert!(!a_ej.contains(&OutputAction::Backspace), "「が」must not be overwritten: {a_ej:?}");
    assert_eq!(m.tentative_kana_string(), "がで");
}

#[test]
fn mutual_chord_no_timeout_after_window() {
    // symmetric=true chord fires regardless of timing — even well after chord_window_ms.
    let layout = load_layout(MUTUAL_CHORD_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("j"), now);
    // Press f long after any timeout would have expired (500 ms)
    let late = now + std::time::Duration::from_millis(500);
    let a = m.process(InputEvent::down("f"), late);
    assert!(a.contains(&OutputAction::SendKana("が".to_string())), "mutual chord must fire regardless of timing: {a:?}");
}

#[test]
fn timed_chord_reverse_order_allowed() {
    // symmetric=false chord should fire in either key order.
    let layout = load_layout(MUTUAL_CHORD_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // Definition order is ["r","u"]; try u first, then r
    let a1 = m.process(InputEvent::down("u"), now);
    assert!(a1.contains(&OutputAction::SendKana("す".to_string())), "u speculative: {a1:?}");
    let a2 = m.process(InputEvent::down("r"), now);
    assert!(a2.contains(&OutputAction::Backspace), "{a2:?}");
    assert!(a2.contains(&OutputAction::SendKana("だ".to_string())), "reverse-order timed chord: {a2:?}");
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
    m.process(InputEvent::down("q"), now); // そ
    m.process(InputEvent::down("w"), now); // こ
    assert_eq!(m.tentative_kana_string(), "そこ");
    let bs = m.process(InputEvent::down("bs"), now);
    assert!(bs.contains(&OutputAction::Backspace));
    assert_eq!(m.tentative_kana_string(), "そ");
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
. . q w
1 ＿ あ い
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
. . q w
1 ＿ あ !Tab
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
. . q
1 ＿ "、 !SuperFakeKey"
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
. . q w
1 ＿ ku_ret い
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
. . q
1 ＿ "。 !Return"
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
. . q w
1 ＿ ku_ret い
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
. . q w
1 ＿ ku_ret い
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

// f is row-2 index 4 (caps_lock a s d f g …); g is index 5.
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
. .    a    s    d    f    g
2 ＿   ＿   ＿   ＿   か   き
"""

[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "shift_f"
grid     = """
. .    a    s    d    f    g
2 ＿   ＿   ＿   ＿   ＿   ぎ
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
    // Modified-layer output goes into tentative buffer (so BackSpace can pop it).
    assert_eq!(m.tentative_kana_string(), "ぎ");
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
. . q
1 ＿ あ
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
    // Tap output goes into the tentative buffer (same as any other kana).
    assert_eq!(m.tentative_kana_string(), "　");
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
. . q
1 ＿ あ
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
. .    a    s    d    f
2 ＿   ＿   ＿   ＿   ku_ret
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "m"
grid     = """
. .    a    s    d    f    g
2 ＿   ＿   ＿   ＿   ＿   ぎ
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
. . q
1 ＿ あ
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
. . q
1 ＿ い
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "caps"
grid     = """
. . q
1 ＿ あ
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

// ── Additional boundary / regression tests ────────────────────────────────────

// Timeout-based hold detection for dual-role modifier.
// f is in row-2 index 3; g is index 4.
const TIMEOUT_DUAL_ROLE_LAYOUT: &str = r#"
[meta]
name   = "timeout dual-role test"
mode   = "kana"
schema = 1

[[modifier]]
id              = "shift_f"
key             = "f"
kind            = "hold"
hold_detection  = "timeout"
hold_timeout_ms = 150
tap_action      = "base_kana"

[[layer]]
id   = "base"
kind = "single"
grid = """
. .    a    s    d    f    g
2 ＿   ＿   ＿   ＿   か   き
"""

[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "shift_f"
grid     = """
. .    a    s    d    f    g
2 ＿   ＿   ＿   ＿   ＿   ぎ
"""
"#;

#[test]
fn dual_role_timeout_tap_emits_base_kana() {
    // Quick press+release (< timeout) should emit base kana on tap.
    let layout = load_layout(TIMEOUT_DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("f"), now);
    let actions = m.process(InputEvent::up("f"), now); // released immediately (0ms)
    assert!(
        actions.contains(&OutputAction::SendKana("か".to_string())),
        "quick tap should emit base kana: {actions:?}"
    );
}

#[test]
fn dual_role_timeout_hold_emits_shifted_kana() {
    // Pressing another key after the timeout has elapsed should activate shifted layer.
    let layout = load_layout(TIMEOUT_DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let press_time = Instant::now();
    m.process(InputEvent::down("f"), press_time);
    // Simulate pressing g after 200ms (> hold_timeout_ms = 150)
    let later = press_time + std::time::Duration::from_millis(200);
    let actions = m.process(InputEvent::down("g"), later);
    assert!(
        actions.contains(&OutputAction::SendKana("ぎ".to_string())),
        "key pressed after timeout should use shifted layer: {actions:?}"
    );
}

#[test]
fn dual_role_key_in_chord_fires_chord_not_tap() {
    // Pressing the dual-role modifier key and then another key together should
    // emit the shifted-layer output (modifier wins) and NOT the base-kana tap.
    // The modifier's `interrupted` flag prevents the tap from firing on key-up.
    let layout = load_layout(DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    m.process(InputEvent::down("f"), now);      // modifier held
    let a_g = m.process(InputEvent::down("g"), now); // interrupted → shifted layer ぎ
    assert!(
        a_g.contains(&OutputAction::SendKana("ぎ".to_string())),
        "other key should emit from shifted layer: {a_g:?}"
    );
    let a_up = m.process(InputEvent::up("f"), now);
    assert!(
        !a_up.contains(&OutputAction::SendKana("か".to_string())),
        "tap (base kana) must NOT fire after interrupted hold: {a_up:?}"
    );
}

#[test]
fn toggle_modifier_does_not_block_other_keys() {
    // While a toggle modifier is on, regular keys not in the shifted layer
    // should still fall through to the base layer.
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
. .    q    w
1 ＿   い   う
"""
[[layer]]
id       = "shifted"
kind     = "modified"
modifier = "caps"
grid     = """
. . q
1 ＿ あ
"""
"#;
    let layout = load_layout(toml).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // Toggle ON
    m.process(InputEvent::down("caps_lock"), now);
    m.process(InputEvent::up("caps_lock"), now);
    // q is in shifted layer → あ
    let a_q = m.process(InputEvent::down("q"), now);
    assert!(a_q.contains(&OutputAction::SendKana("あ".to_string())), "{a_q:?}");
    // w is NOT in shifted layer → falls through to base layer → う
    let a_w = m.process(InputEvent::down("w"), now);
    assert!(a_w.contains(&OutputAction::SendKana("う".to_string())), "{a_w:?}");
}

#[test]
fn tentative_cleared_after_conversion_then_modifier_tap() {
    // After Mozc conversion is notified, the tentative buffer is cleared.
    // A subsequent modifier tap should add only its own kana (no stale entries).
    let layout = load_layout(DUAL_ROLE_LAYOUT).unwrap();
    let mut m = StateMachine::new(layout);
    let now = Instant::now();
    // Build up some tentative kana
    m.process(InputEvent::down("g"), now); // base き
    assert_eq!(m.tentative_kana_string(), "き");
    // Mozc switches to conversion (preedit committed)
    m.notify_mozc_conversion();
    assert_eq!(m.tentative_kana_string(), "");
    // Tap modifier → base kana か added to (now-empty) tentative
    m.process(InputEvent::down("f"), now);
    m.process(InputEvent::up("f"), now);
    assert_eq!(m.tentative_kana_string(), "か", "no stale entries after conversion");
}

#[test]
fn tap_output_set_without_output_action_is_error() {
    // Having tap_output but tap_action != "output" should be an error.
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[modifier]]
id         = "m"
key        = "space"
tap_action = "none"
tap_output = "　"
"#;
    let err = load_layout(toml).unwrap_err();
    assert!(
        err.to_string().contains("tap_output"),
        "error should mention tap_output: {err}"
    );
}

#[test]
fn direct_trigger_base_kana_tap_action_is_error() {
    // base_kana has no effect on direct triggers and must be rejected.
    let toml = r#"
[meta]
name = "test"
mode = "hybrid"
schema = 1
[direct_trigger]
keys       = ["henkan"]
tap_action = "base_kana"
"#;
    assert!(load_layout(toml).is_err());
}

#[test]
fn direct_trigger_output_tap_action_is_error() {
    // output has no effect on direct triggers and must be rejected.
    let toml = r#"
[meta]
name = "test"
mode = "hybrid"
schema = 1
[direct_trigger]
keys       = ["henkan"]
tap_action = "output"
tap_output = "あ"
"#;
    assert!(load_layout(toml).is_err());
}

// ── Mozc codec unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod codec_tests {
    use crate::mozc::codec::{decode, find_bytes, find_msg, find_varint, write_len_field, write_varint_field};

    fn round_trip_varint(field: u32, value: u64) {
        let mut buf = Vec::new();
        write_varint_field(&mut buf, field, value);
        let fields = decode(&buf).unwrap();
        assert_eq!(find_varint(&fields, field), Some(value));
    }

    #[test]
    fn codec_varint_roundtrip_small() {
        round_trip_varint(1, 0);
        round_trip_varint(1, 1);
        round_trip_varint(2, 127);
        round_trip_varint(3, 128);
        round_trip_varint(15, u64::MAX);
    }

    #[test]
    fn codec_len_field_roundtrip() {
        let data = "かきくけこ".as_bytes();
        let mut buf = Vec::new();
        write_len_field(&mut buf, 5, data);
        let fields = decode(&buf).unwrap();
        assert_eq!(find_bytes(&fields, 5), Some(data));
    }

    #[test]
    fn codec_multiple_fields() {
        let mut buf = Vec::new();
        write_varint_field(&mut buf, 1, 42);
        write_len_field(&mut buf, 2, b"hello");
        write_varint_field(&mut buf, 3, 99);
        let fields = decode(&buf).unwrap();
        assert_eq!(find_varint(&fields, 1), Some(42));
        assert_eq!(find_bytes(&fields, 2), Some(b"hello".as_slice()));
        assert_eq!(find_varint(&fields, 3), Some(99));
    }

    #[test]
    fn codec_nested_message_roundtrip() {
        // inner = { field 2: "かな" }
        let mut inner = Vec::new();
        write_len_field(&mut inner, 2, "かな".as_bytes());
        let mut outer = Vec::new();
        write_varint_field(&mut outer, 1, 7);
        write_len_field(&mut outer, 4, &inner);
        let fields = decode(&outer).unwrap();
        assert_eq!(find_varint(&fields, 1), Some(7));
        let inner_fields = find_msg(&fields, 4).unwrap().unwrap();
        assert_eq!(find_bytes(&inner_fields, 2), Some("かな".as_bytes()));
    }

    #[test]
    fn codec_group_encode_decode() {
        // Proto2 group: field 2, START_GROUP wire=3, fields, END_GROUP wire=4
        // Start:  tag = (2 << 3) | 3 = 0x13
        // End:    tag = (2 << 3) | 4 = 0x14
        let mut buf = Vec::new();
        write_varint_field(&mut buf, 1, 3); // Preedit.cursor = 3
        buf.push(0x13);                     // Segment group start
        write_varint_field(&mut buf, 3, 2); // Segment.annotation = HIGHLIGHT(2)
        write_len_field(&mut buf, 4, "変換".as_bytes()); // Segment.value
        buf.push(0x14);                     // Segment group end

        let fields = decode(&buf).unwrap();
        assert_eq!(find_varint(&fields, 1), Some(3));
        let seg_fields = find_msg(&fields, 2).unwrap().unwrap();
        assert_eq!(find_varint(&seg_fields, 3), Some(2)); // HIGHLIGHT
        assert_eq!(find_bytes(&seg_fields, 4), Some("変換".as_bytes()));
    }

    #[test]
    fn proto_encode_send_kana_is_non_empty() {
        use crate::mozc::proto::{encode_command, input_send_kana};
        let encoded = encode_command(&input_send_kana(1234, "あ"));
        assert!(!encoded.is_empty());
        // encode_command returns raw Input bytes (no Command wrapper).
        let fields = decode(&encoded).unwrap();
        // Input.type = SEND_KEY (3)
        assert_eq!(find_varint(&fields, 1), Some(3), "Input.type should be SEND_KEY=3");
        // Input.id = 1234
        assert_eq!(find_varint(&fields, 2), Some(1234), "Input.id should be 1234");
    }

    /// Build a Command{output: Output{...}} with preedit segments (including HIGHLIGHT)
    /// and verify that decode_response() extracts all fields correctly.
    #[test]
    fn decode_response_full_roundtrip() {
        use crate::mozc::proto::decode_response;

        // ── Build Result { type=STRING(1), value="変換" } ──────────────────
        let mut result_msg = Vec::new();
        write_varint_field(&mut result_msg, 1, 1);                      // type = STRING
        write_len_field(&mut result_msg, 2, "変換".as_bytes());          // value

        // ── Build Preedit with two Segments using proto2 group encoding ────
        //    group start: tag = (2 << 3) | 3 = 0x13
        //    group end:   tag = (2 << 3) | 4 = 0x14
        let mut preedit_msg = Vec::new();
        write_varint_field(&mut preedit_msg, 1, 2);   // cursor = 2
        // Segment 1: UNDERLINE, value = "か"
        preedit_msg.push(0x13);
        write_varint_field(&mut preedit_msg, 3, 1);   // annotation = UNDERLINE
        write_len_field(&mut preedit_msg, 4, "か".as_bytes());
        write_varint_field(&mut preedit_msg, 5, 1);   // value_length = 1
        preedit_msg.push(0x14);
        // Segment 2: HIGHLIGHT, value = "な"
        preedit_msg.push(0x13);
        write_varint_field(&mut preedit_msg, 3, 2);   // annotation = HIGHLIGHT
        write_len_field(&mut preedit_msg, 4, "な".as_bytes());
        write_varint_field(&mut preedit_msg, 5, 1);   // value_length = 1
        preedit_msg.push(0x14);

        // ── Build Output ──────────────────────────────────────────────────
        let mut output_msg = Vec::new();
        write_varint_field(&mut output_msg, 1, 42);              // id = 42
        write_varint_field(&mut output_msg, 2, 1);               // mode = HIRAGANA
        write_varint_field(&mut output_msg, 3, 1);               // consumed = true
        write_len_field(&mut output_msg, 4, &result_msg);        // result
        write_len_field(&mut output_msg, 5, &preedit_msg);       // preedit

        // decode_response expects raw Output bytes (no Command wrapper).

        // ── Decode and verify ─────────────────────────────────────────────
        let out = decode_response(&output_msg).expect("decode_response failed");
        assert_eq!(out.session_id, Some(42));
        assert_eq!(out.mode, 1); // HIRAGANA
        assert!(out.consumed);
        assert_eq!(out.result_value.as_deref(), Some("変換"));
        assert_eq!(out.preedit_text, "かな");
        assert!(out.preedit_has_highlight, "HIGHLIGHT segment should be detected");
    }
}

// ── Modifier key token tests ──────────────────────────────────────────────────

fn modifier_key_layout(output: &str) -> String {
    // Grid format: header `. 1` sets col_count=1.
    // `. q` sets explicit_keys=["q"] for subsequent data rows.
    // `1 <output>` maps key "q" to the given output.
    format!(
        r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. 1
. q
1 {output}
"""
"#,
        output = output
    )
}

#[test]
fn modified_key_ctrl_z() {
    let toml = modifier_key_layout("C-z");
    let mut m = sm(&toml);
    let actions = press_seq(&mut m, &["q"]);
    assert!(
        actions.contains(&OutputAction::SendModifiedKey {
            key: "z".to_string(),
            mods: MOD_CTRL,
        }),
        "expected SendModifiedKey for C-z, got {actions:?}"
    );
}

#[test]
fn modified_key_shift_up() {
    let toml = modifier_key_layout("S-!Up");
    let mut m = sm(&toml);
    let actions = press_seq(&mut m, &["q"]);
    assert!(
        actions.contains(&OutputAction::SendModifiedKey {
            key: "Up".to_string(),
            mods: MOD_SHIFT,
        }),
        "expected SendModifiedKey for S-!Up, got {actions:?}"
    );
}

#[test]
fn modified_key_shift_ctrl_s() {
    let toml = modifier_key_layout("S-C-s");
    let mut m = sm(&toml);
    let actions = press_seq(&mut m, &["q"]);
    assert!(
        actions.contains(&OutputAction::SendModifiedKey {
            key: "s".to_string(),
            mods: MOD_SHIFT | MOD_CTRL,
        }),
        "expected SendModifiedKey for S-C-s, got {actions:?}"
    );
}

#[test]
fn modified_key_not_in_tentative() {
    let toml = modifier_key_layout("C-z");
    let mut m = sm(&toml);
    press_seq(&mut m, &["q"]);
    assert!(
        m.tentative_kana_string().is_empty(),
        "modifier key output must not enter the tentative buffer"
    );
}

#[test]
fn modified_key_invalid_function_key_rejected() {
    let toml = modifier_key_layout("C-!BadKey");
    assert!(
        load_layout(&toml).is_err(),
        "layout with unknown function key after modifier prefix must fail to load"
    );
}

#[test]
fn modified_key_in_alias() {
    let toml = r#"
[meta]
name = "test"
mode = "kana"
schema = 1
[[layer]]
id   = "base"
kind = "single"
grid = """
. 1
. q
1 save
"""

[[alias]]
save = "S-C-s"
"#;
    let mut m = sm(toml);
    let actions = press_seq(&mut m, &["q"]);
    assert!(
        actions.contains(&OutputAction::SendModifiedKey {
            key: "s".to_string(),
            mods: MOD_SHIFT | MOD_CTRL,
        }),
        "alias containing modifier key token must expand correctly, got {actions:?}"
    );
}
