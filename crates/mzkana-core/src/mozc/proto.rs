/// Mozc protobuf types and encoding/decoding via prost.
///
/// Protobuf message definitions are auto-generated from vendored `protocol/commands.proto`
/// (and transitive imports: config.proto, candidate_window.proto, engine_builder.proto,
/// user_dictionary_storage.proto) using prost-build with vendored protoc.
///
/// See build.rs for proto compilation setup.
/// Original sources from: https://github.com/google/mozc/tree/master/src/protocol

use prost::Message;

// Include generated protobuf code in module hierarchy
// mozc is the top-level namespace that contains commands, config, etc.
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

pub use mozc::commands::{
    CompositionMode, Input, KeyEvent, Output, SessionCommand, Preedit, CandidateWindow,
};
// Alias the protobuf Result to avoid conflict with std::result::Result
pub use mozc::commands::Result as ProtoResult;

// ── Enum constants (re-exported for convenience) ────────────────────────────────

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
    pub const SWITCH_INPUT_MODE: i32 = 5;  // C3: for composition mode initialization
    pub const TURN_ON_IME: i32 = 22;       // C3: alternative initialization method
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
    pub const KANA: i32 = 15;   // Hiragana_Katakana
    pub const HOME: i32 = 16;
    pub const END: i32 = 17;
    pub const TAB: i32 = 18;
    pub const PAGE_UP: i32 = 31;
    pub const PAGE_DOWN: i32 = 32;
    pub const INSERT: i32 = 33;
}

/// KeyEvent.ModifierKey (field 4, repeated enum)
pub mod modifier_key {
    // Fixed to match official Mozc values
    pub const SHIFT: i32 = 1;
    pub const CTRL: i32 = 1;   // Same as SHIFT in Mozc (field-based, not exclusive)
    pub const ALT: i32 = 2;
}

/// KeyEvent.InputStyle
pub mod input_style {
    /// Key value follows current input mode.
    pub const FOLLOW_MODE: i32 = 0;
    /// Send key_string directly as-is (kana direct input).
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

/// Encode an Input message to wire format for IPC transmission.
pub fn encode_command(input: &EncodedInput) -> Vec<u8> {
    input.0.clone()
}

/// Build a CREATE_SESSION input (C3 prerequisite).
pub fn input_create_session() -> EncodedInput {
    let msg = Input {
        r#type: cmd::CREATE_SESSION,
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a DELETE_SESSION input.
pub fn input_delete_session(session_id: u64) -> EncodedInput {
    let msg = Input {
        r#type: cmd::DELETE_SESSION,
        id: Some(session_id),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_KEY input sending a kana string with `DIRECT_INPUT` style.
pub fn input_send_kana(session_id: u64, kana: &str) -> EncodedInput {
    let key = KeyEvent {
        key_string: Some(kana.to_string()),
        input_style: Some(input_style::DIRECT_INPUT),
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_KEY,
        id: Some(session_id),
        key: Some(key),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_KEY input sending a special key code.
pub fn input_send_special(session_id: u64, special: i32) -> EncodedInput {
    let key = KeyEvent {
        special_key: Some(special),
        input_style: Some(input_style::FOLLOW_MODE),
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_KEY,
        id: Some(session_id),
        key: Some(key),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_KEY input with a special key and modifier flags (e.g. S-!Left).
/// `mods` uses the same bitmask as `OutputAction::SendModifiedKey`.
pub fn input_send_special_with_mods(session_id: u64, special: i32, mods: u8) -> EncodedInput {
    let mut modifier_keys = Vec::new();
    if mods & 0x01 != 0 { modifier_keys.push(modifier_key::SHIFT); }
    if mods & 0x02 != 0 { modifier_keys.push(modifier_key::CTRL); }
    if mods & 0x04 != 0 { modifier_keys.push(modifier_key::ALT); }

    let key = KeyEvent {
        special_key: Some(special),
        input_style: Some(input_style::FOLLOW_MODE),
        modifier_keys,
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_KEY,
        id: Some(session_id),
        key: Some(key),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_KEY input with a key_code (ASCII) and modifier flags (e.g. C-z).
/// `key_code` is the ASCII code of the character (e.g. b'z' = 122).
pub fn input_send_key_code_with_mods(session_id: u64, key_code: u32, mods: u8) -> EncodedInput {
    let mut modifier_keys = Vec::new();
    if mods & 0x01 != 0 { modifier_keys.push(modifier_key::SHIFT); }
    if mods & 0x02 != 0 { modifier_keys.push(modifier_key::CTRL); }
    if mods & 0x04 != 0 { modifier_keys.push(modifier_key::ALT); }

    let key = KeyEvent {
        key_code: Some(key_code),
        modifier_keys,
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_KEY,
        id: Some(session_id),
        key: Some(key),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_COMMAND / SUBMIT input.
pub fn input_submit(session_id: u64) -> EncodedInput {
    let command = SessionCommand {
        r#type: session_cmd::SUBMIT,
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_COMMAND,
        id: Some(session_id),
        command: Some(command),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_COMMAND / REVERT input (cancel current preedit).
pub fn input_revert(session_id: u64) -> EncodedInput {
    let command = SessionCommand {
        r#type: session_cmd::REVERT,
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_COMMAND,
        id: Some(session_id),
        command: Some(command),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

/// Build a SEND_COMMAND / SWITCH_INPUT_MODE input to initialize composition mode (C3).
pub fn input_set_composition_mode(session_id: u64, mode: i32) -> EncodedInput {
    let command = SessionCommand {
        r#type: session_cmd::SWITCH_INPUT_MODE,
        composition_mode: Some(mode),
        ..Default::default()
    };

    let msg = Input {
        r#type: cmd::SEND_COMMAND,
        id: Some(session_id),
        command: Some(command),
        ..Default::default()
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode failed");
    EncodedInput(buf)
}

// ── Decoding ─────────────────────────────────────────────────────────────────

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

    // Extract result value
    if let Some(result) = output.result {
        out.result_value = Some(result.value);
    }

    // Extract preedit text and highlight flag
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

    std::result::Result::Ok(out)
}
