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
    // [[chord]] keys=["a","s"] symmetric=true output="ざ"
    // Try s first, then a
    let a1 = m.process(InputEvent::down("s"), now);
    // s in naginata base (row2 col1 = a? no... row2: a,s,d,f,g,h,j,k,l,semicolon)
    // base row2: は,と,に,い,り,の,す,き,る,ち -> a=は
    // So s→と
    assert!(!a1.is_empty());
    let a2 = m.process(InputEvent::down("a"), now);
    assert!(a2.contains(&OutputAction::Backspace));
    assert!(a2.contains(&OutputAction::SendKana("ざ".to_string())));
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
    // Hold space (center modifier), then press q
    m.process(InputEvent::down("space"), now);
    let a = m.process(InputEvent::down("q"), now);
    // center_shift layer row1 q → ぁ
    assert!(a.contains(&OutputAction::SendKana("ぁ".to_string())));
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
