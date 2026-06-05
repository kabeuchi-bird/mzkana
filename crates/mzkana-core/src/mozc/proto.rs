//! Mozc protobuf types and encoding/decoding via prost.
//!
//! Protobuf message definitions are auto-generated from vendored `protocol/commands.proto`
//! (and transitive imports: config.proto, candidate_window.proto, engine_builder.proto,
//! user_dictionary_storage.proto) using prost-build with vendored protoc.
//!
//! See build.rs for proto compilation setup.
//! Original sources from: https://github.com/google/mozc/tree/master/src/protocol

use prost::Message;

// Include generated protobuf code in module hierarchy.
// The generated files carry upstream Mozc doc comments and define many messages
// we don't construct directly, so suppress lints we can't fix in generated code.
#[allow(dead_code, clippy::all, clippy::pedantic)]
pub mod mozc {
    include!(concat!(env!("OUT_DIR"), "/mozc.rs"));

    pub mod commands {
        include!(concat!(env!("OUT_DIR"), "/mozc.commands.rs"));
    }

    pub mod config {
        include!(concat!(env!("OUT_DIR"), "/mozc.config.rs"));
    }

    pub mod user_dictionary {
        include!(concat!(env!("OUT_DIR"), "/mozc.user_dictionary.rs"));
    }
}

pub use mozc::commands::{Input, KeyEvent, Output, SessionCommand};

// ── Enum constants ────────────────────────────────────────────────────────────

/// Input.CommandType
pub mod cmd {
    pub const CREATE_SESSION: i32 = 1;
    pub const DELETE_SESSION: i32 = 2;
    pub const SEND_KEY: i32 = 3;
    pub const SEND_COMMAND: i32 = 5;
}

/// SessionCommand.CommandType
pub mod session_cmd {
    pub const REVERT: i32 = 1;
    pub const SUBMIT: i32 = 2;
    /// Select a candidate by id and commit it (closes the candidate window).
    pub const SELECT_CANDIDATE: i32 = 3;
    pub const TURN_ON_IME: i32 = 22;
}

/// KeyEvent.SpecialKey
#[allow(dead_code)]
pub mod special_key {
    pub const SPACE: i32 = 4;
    pub const ENTER: i32 = 5;
    pub const LEFT: i32 = 6;
    pub const RIGHT: i32 = 7;
    pub const UP: i32 = 8;
    pub const DOWN: i32 = 9;
    pub const ESCAPE: i32 = 10;
    pub const DEL: i32 = 11;
    pub const BACKSPACE: i32 = 12;
    pub const HENKAN: i32 = 13;
    pub const MUHENKAN: i32 = 14;
    pub const KANA: i32 = 15;   // Hiragana_Katakana
    pub const HOME: i32 = 16;
    pub const END: i32 = 17;
    pub const TAB: i32 = 18;
    pub const F1: i32 = 19;   // F1..F12 are contiguous: F1=19 … F12=30
    pub const PAGE_UP: i32 = 31;
    pub const PAGE_DOWN: i32 = 32;
    pub const INSERT: i32 = 33;
}

/// KeyEvent.ModifierKey — values from generated ModifierKey enum (Ctrl=1, Alt=2, Shift=4).
pub mod modifier_key {
    pub const CTRL: i32 = 1;
    pub const ALT: i32 = 2;
    pub const SHIFT: i32 = 4;
}

/// KeyEvent.InputStyle
pub mod input_style {
    /// Key value follows current input mode (romaji→kana conversion table is used).
    pub const FOLLOW_MODE: i32 = 0;
    /// Insert key_string directly into the composition buffer, bypassing romaji conversion.
    /// Use this for kana direct input to trigger InsertCharacterPreedit().
    pub const AS_IS: i32 = 1;
    /// Commit key_string directly to result, bypassing preedit entirely.
    #[allow(dead_code)]
    pub const DIRECT_INPUT: i32 = 2;
}

/// Output.mode / CompositionMode
pub mod composition_mode {
    pub const DIRECT: i32 = 0;
    pub const HIRAGANA: i32 = 1;
    pub const FULL_KATAKANA: i32 = 2;
    pub const HALF_ASCII: i32 = 3;
    pub const FULL_ASCII: i32 = 4;
    pub const HALF_KATAKANA: i32 = 5;
}

/// Preedit.Segment.Annotation
pub mod annotation {
    pub const HIGHLIGHT: i32 = 2;
}

// ── Encoding ─────────────────────────────────────────────────────────────────

/// An encoded Input message ready to send over IPC.
#[derive(Clone)]
pub struct EncodedInput(pub Vec<u8>);

fn encode_input(msg: Input) -> EncodedInput {
    EncodedInput(msg.encode_to_vec())
}

/// Build a CREATE_SESSION input.
pub fn input_create_session() -> EncodedInput {
    encode_input(Input { r#type: cmd::CREATE_SESSION, ..Default::default() })
}

/// Build a SEND_COMMAND / TURN_ON_IME input.
///
/// TURN_ON_IME transitions the session from IME-OFF (Direct) to the specified
/// composition mode.  New sessions start IME-OFF; this call is required before
/// kana input will be consumed by Mozc. (SWITCH_COMPOSITION_MODE alone does NOT
/// leave IME-OFF — verified on a live mozc_server — so TURN_ON_IME is used here.)
pub fn input_turn_on_ime(session_id: u64, mode: i32) -> EncodedInput {
    let command = SessionCommand {
        r#type: session_cmd::TURN_ON_IME,
        composition_mode: Some(mode),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_COMMAND, id: Some(session_id), command: Some(command), ..Default::default() })
}

/// Build a DELETE_SESSION input.
pub fn input_delete_session(session_id: u64) -> EncodedInput {
    encode_input(Input { r#type: cmd::DELETE_SESSION, id: Some(session_id), ..Default::default() })
}

/// Build a SEND_KEY input sending a kana string with `AS_IS` style.
///
/// InputStyle semantics:
///   FOLLOW_MODE (0): routes through romaji→kana table; kana strings get no output.
///   AS_IS       (1): calls InsertCharacterPreedit() — inserts kana directly into preedit.
///   DIRECT_INPUT(2): commits directly to result, bypassing preedit entirely.
pub fn input_send_kana(session_id: u64, kana: &str) -> EncodedInput {
    let key = KeyEvent {
        key_string: Some(kana.to_string()),
        input_style: Some(input_style::AS_IS),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_KEY, id: Some(session_id), key: Some(key), ..Default::default() })
}

/// Build a SEND_KEY input sending a special key code.
pub fn input_send_special(session_id: u64, special: i32) -> EncodedInput {
    let key = KeyEvent {
        special_key: Some(special),
        input_style: Some(input_style::FOLLOW_MODE),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_KEY, id: Some(session_id), key: Some(key), ..Default::default() })
}

/// Build a SEND_KEY input with a special key and modifier flags (e.g. S-!Left).
/// `mods` bitmask: 0x01=Shift, 0x02=Ctrl, 0x04=Alt.
pub fn input_send_special_with_mods(session_id: u64, special: i32, mods: u8) -> EncodedInput {
    let key = KeyEvent {
        special_key: Some(special),
        input_style: Some(input_style::FOLLOW_MODE),
        modifier_keys: mods_to_vec(mods),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_KEY, id: Some(session_id), key: Some(key), ..Default::default() })
}

/// Build a SEND_KEY input with a key_code (ASCII) and modifier flags (e.g. C-z).
/// `key_code` is the ASCII code of the character (e.g. b'z' = 122).
pub fn input_send_key_code_with_mods(session_id: u64, key_code: u32, mods: u8) -> EncodedInput {
    let key = KeyEvent {
        key_code: Some(key_code),
        modifier_keys: mods_to_vec(mods),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_KEY, id: Some(session_id), key: Some(key), ..Default::default() })
}

/// Map the statemachine modifier bitmask to Mozc ModifierKey values.
/// MOD_SUPER (0x08) has no Mozc equivalent and is intentionally omitted —
/// the caller must fall through to forward_key for Super-modified keys.
fn mods_to_vec(mods: u8) -> Vec<i32> {
    let mut v = Vec::new();
    if mods & 0x01 != 0 { v.push(modifier_key::SHIFT); }
    if mods & 0x02 != 0 { v.push(modifier_key::CTRL); }
    if mods & 0x04 != 0 { v.push(modifier_key::ALT); }
    v
}

/// Build a SEND_COMMAND / SUBMIT input.
pub fn input_submit(session_id: u64) -> EncodedInput {
    let command = SessionCommand { r#type: session_cmd::SUBMIT, ..Default::default() };
    encode_input(Input { r#type: cmd::SEND_COMMAND, id: Some(session_id), command: Some(command), ..Default::default() })
}

/// Build a SEND_COMMAND / REVERT input (cancel current preedit).
pub fn input_revert(session_id: u64) -> EncodedInput {
    let command = SessionCommand { r#type: session_cmd::REVERT, ..Default::default() };
    encode_input(Input { r#type: cmd::SEND_COMMAND, id: Some(session_id), command: Some(command), ..Default::default() })
}

/// Build a SEND_COMMAND / SELECT_CANDIDATE input (commit the candidate with the
/// given Mozc-internal `candidate_id`, closing the candidate window).
pub fn input_select_candidate(session_id: u64, candidate_id: i32) -> EncodedInput {
    let command = SessionCommand {
        r#type: session_cmd::SELECT_CANDIDATE,
        id: Some(candidate_id),
        ..Default::default()
    };
    encode_input(Input { r#type: cmd::SEND_COMMAND, id: Some(session_id), command: Some(command), ..Default::default() })
}

// ── Decoding ─────────────────────────────────────────────────────────────────

/// One entry in the Mozc candidate window (prediction or conversion).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidate {
    /// Display index within the candidate window (0-based).
    pub index: u32,
    /// Candidate text.
    pub value: String,
    /// Mozc-internal candidate id, used by SELECT_CANDIDATE / HIGHLIGHT_CANDIDATE.
    /// `None` when Mozc did not assign one (such a candidate cannot be selected
    /// by id and must not be exposed to the selection API).
    pub id: Option<i32>,
    /// Optional annotation (description / suffix, e.g. "[半][カナ]").
    pub annotation: Option<String>,
}

/// Decoded Output from Mozc.
#[derive(Debug, Default)]
pub struct DecodedOutput {
    pub session_id: Option<u64>,
    pub mode: i32,
    pub consumed: bool,
    pub result_value: Option<String>,
    pub preedit_text: String,
    /// True if any preedit segment is highlighted (indicates CONVERSION mode).
    pub preedit_has_highlight: bool,
    /// Candidates in the current window (current page only).
    pub candidates: Vec<Candidate>,
    /// Focused candidate index when converting (`has_focused_index`); `None` for a
    /// suggestion/prediction window or when there is no candidate window.
    pub focused_index: Option<u32>,
    /// Total candidate count across all pages (`CandidateWindow.size`).
    pub candidate_size: u32,
}

/// Decode a Mozc IPC response into `DecodedOutput`.
///
/// Mozc sends raw `Output` protobuf bytes on the wire.
pub fn decode_response(data: &[u8]) -> std::result::Result<DecodedOutput, prost::DecodeError> {
    let output = Output::decode(data)?;

    let mut out = DecodedOutput {
        session_id: output.id,
        mode: output.mode.unwrap_or(composition_mode::DIRECT),
        consumed: output.consumed.unwrap_or(false),
        ..Default::default()
    };

    if let Some(result) = output.result {
        out.result_value = Some(result.value);
    }

    if let Some(preedit) = output.preedit {
        let mut preedit_str = String::new();
        for segment in &preedit.segment {
            preedit_str.push_str(&segment.value);
            if segment.annotation == annotation::HIGHLIGHT {
                out.preedit_has_highlight = true;
            }
        }
        out.preedit_text = preedit_str;
    }

    if let Some(cw) = output.candidate_window {
        out.candidate_size = cw.size;
        // has_focused_index ⇒ conversion (focused) window; absent ⇒ suggestion.
        out.focused_index = cw.focused_index;
        for c in cw.candidate {
            // An annotation may carry prefix/suffix/description; surface the most
            // useful single string (description, falling back to suffix).
            let annotation = c.annotation.and_then(|a| a.description.or(a.suffix));
            out.candidates.push(Candidate {
                index: c.index,
                value: c.value,
                id: c.id,
                annotation,
            });
        }
    }

    Ok(out)
}
