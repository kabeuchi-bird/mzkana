/// Mozc protobuf types — hand-written to match `protocol/commands.proto`.
///
/// Field numbers and enum values are taken directly from the Mozc source.
/// Only the subset needed for kana input (send key, submit, create/delete session)
/// is represented here.
use super::codec::{
    decode, find_all_msg, find_bytes, find_msg, find_varint, write_len_field, write_varint_field,
};

// ── Enum constants ────────────────────────────────────────────────────────────

/// Input.CommandType
pub mod cmd {
    pub const CREATE_SESSION: u64 = 1;
    pub const DELETE_SESSION: u64 = 2;
    pub const SEND_KEY: u64 = 3;
    pub const SEND_COMMAND: u64 = 5;
}

/// SessionCommand.CommandType
pub mod session_cmd {
    pub const SUBMIT: u64 = 2;
    pub const REVERT: u64 = 6;
}

/// KeyEvent.SpecialKey
#[allow(dead_code)]
pub mod special_key {
    pub const SPACE: u64 = 4;
    pub const ENTER: u64 = 5;
    pub const LEFT: u64 = 6;
    pub const RIGHT: u64 = 7;
    pub const UP: u64 = 8;
    pub const DOWN: u64 = 9;
    pub const ESCAPE: u64 = 10;
    pub const DEL: u64 = 11;
    pub const BACKSPACE: u64 = 12;
    pub const HENKAN: u64 = 13;
    pub const KANA: u64 = 15;   // Hiragana_Katakana
    pub const HOME: u64 = 16;
    pub const END: u64 = 17;
    pub const TAB: u64 = 18;
    // F1=19 … F12=30
    pub const PAGE_UP: u64 = 31;
    pub const PAGE_DOWN: u64 = 32;
    pub const INSERT: u64 = 33;
}

/// KeyEvent.ModifierKey (field 4, repeated varint)
pub mod modifier_key {
    pub const SHIFT: u64 = 1;
    pub const CTRL:  u64 = 3;
    pub const ALT:   u64 = 4;
    // Super/Meta does not exist in Mozc's ModifierKey enum
}

/// KeyEvent.InputStyle
pub mod input_style {
    /// Key value follows current input mode.
    pub const FOLLOW_MODE: u64 = 0;
    /// Send key_string directly as-is (kana direct input).
    pub const DIRECT_INPUT: u64 = 2;
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
    pub const HIGHLIGHT: u64 = 2;
}

// ── Encoding ─────────────────────────────────────────────────────────────────

/// Encode a `Command { input: Input { type, id?, key?, command? } }`.
pub fn encode_command(input: &EncodedInput) -> Vec<u8> {
    let mut cmd = Vec::new();
    write_len_field(&mut cmd, 1, &input.0);
    cmd
}

/// A pre-encoded `Input` message ready to be wrapped in `Command`.
pub struct EncodedInput(Vec<u8>);

/// Build a CREATE_SESSION input.
pub fn input_create_session() -> EncodedInput {
    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::CREATE_SESSION);
    EncodedInput(buf)
}

/// Build a DELETE_SESSION input.
pub fn input_delete_session(session_id: u64) -> EncodedInput {
    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::DELETE_SESSION);
    write_varint_field(&mut buf, 2, session_id);
    EncodedInput(buf)
}

/// Build a SEND_KEY input sending a kana string with `DIRECT_INPUT` style.
pub fn input_send_kana(session_id: u64, kana: &str) -> EncodedInput {
    let mut key = Vec::new();
    write_len_field(&mut key, 5, kana.as_bytes());          // key_string = 5
    write_varint_field(&mut key, 6, input_style::DIRECT_INPUT); // input_style = 6

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_KEY);          // type = 1
    write_varint_field(&mut buf, 2, session_id);             // id = 2
    write_len_field(&mut buf, 3, &key);                      // key = 3
    EncodedInput(buf)
}

/// Build a SEND_KEY input sending a special key code.
pub fn input_send_special(session_id: u64, special: u64) -> EncodedInput {
    let mut key = Vec::new();
    write_varint_field(&mut key, 3, special);                // special_key = 3
    write_varint_field(&mut key, 6, input_style::FOLLOW_MODE); // input_style = 6

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_KEY);
    write_varint_field(&mut buf, 2, session_id);
    write_len_field(&mut buf, 3, &key);
    EncodedInput(buf)
}

/// Build a SEND_KEY input with a special key and modifier flags (e.g. S-!Left).
/// `mods` uses the same bitmask as `OutputAction::SendModifiedKey`.
pub fn input_send_special_with_mods(session_id: u64, special: u64, mods: u8) -> EncodedInput {
    let mut key = Vec::new();
    write_varint_field(&mut key, 3, special);                    // special_key = 3
    write_varint_field(&mut key, 6, input_style::FOLLOW_MODE);   // input_style = 6
    if mods & 0x01 != 0 { write_varint_field(&mut key, 4, modifier_key::SHIFT); }
    if mods & 0x02 != 0 { write_varint_field(&mut key, 4, modifier_key::CTRL); }
    if mods & 0x04 != 0 { write_varint_field(&mut key, 4, modifier_key::ALT); }

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_KEY);
    write_varint_field(&mut buf, 2, session_id);
    write_len_field(&mut buf, 3, &key);
    EncodedInput(buf)
}

/// Build a SEND_KEY input with a key_code (ASCII) and modifier flags (e.g. C-z).
/// `key_code` is the ASCII code of the character (e.g. b'z' = 122).
pub fn input_send_key_code_with_mods(session_id: u64, key_code: u32, mods: u8) -> EncodedInput {
    let mut key = Vec::new();
    write_varint_field(&mut key, 1, key_code as u64);            // key_code = 1
    if mods & 0x01 != 0 { write_varint_field(&mut key, 4, modifier_key::SHIFT); }
    if mods & 0x02 != 0 { write_varint_field(&mut key, 4, modifier_key::CTRL); }
    if mods & 0x04 != 0 { write_varint_field(&mut key, 4, modifier_key::ALT); }

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_KEY);
    write_varint_field(&mut buf, 2, session_id);
    write_len_field(&mut buf, 3, &key);
    EncodedInput(buf)
}

/// Build a SEND_COMMAND / SUBMIT input.
pub fn input_submit(session_id: u64) -> EncodedInput {
    let mut sc = Vec::new();
    write_varint_field(&mut sc, 1, session_cmd::SUBMIT);     // SessionCommand.type = 1

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_COMMAND);
    write_varint_field(&mut buf, 2, session_id);
    write_len_field(&mut buf, 4, &sc);                       // command = 4
    EncodedInput(buf)
}

/// Build a SEND_COMMAND / REVERT input (cancel current preedit).
pub fn input_revert(session_id: u64) -> EncodedInput {
    let mut sc = Vec::new();
    write_varint_field(&mut sc, 1, session_cmd::REVERT);

    let mut buf = Vec::new();
    write_varint_field(&mut buf, 1, cmd::SEND_COMMAND);
    write_varint_field(&mut buf, 2, session_id);
    write_len_field(&mut buf, 4, &sc);
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

/// Decode a `Command` response received from the Mozc server.
/// Extracts the `output` sub-message and decodes it into `DecodedOutput`.
pub fn decode_response(data: &[u8]) -> Result<DecodedOutput, super::codec::DecodeError> {
    let cmd_fields = decode(data)?;

    // Command.output = field 2
    let out_fields = match find_msg(&cmd_fields, 2)? {
        Some(f) => f,
        None => {
            // No output field: fallback for unexpected responses
            return Ok(DecodedOutput {
                session_id: find_varint(&cmd_fields, 1),
                ..Default::default()
            });
        }
    };

    let mut out = DecodedOutput {
        session_id: find_varint(&out_fields, 1),
        mode: find_varint(&out_fields, 2).unwrap_or(0) as i32,
        consumed: find_varint(&out_fields, 3).map(|v| v != 0).unwrap_or(false),
        ..Default::default()
    };
    // Output.result = 4
    if let Some(result_fields) = find_msg(&out_fields, 4)? {
        // Result.value = 2
        if let Some(val) = find_bytes(&result_fields, 2) {
            out.result_value = String::from_utf8(val.to_vec()).ok();
        }
    }
    if let Some(pre_fields) = find_msg(&out_fields, 5)? {
        // Preedit.Segment is a proto2 group at field 2; annotation=3, value=4
        let segs = find_all_msg(&pre_fields, 2)?;
        let mut preedit = String::new();
        for seg in &segs {
            if let Some(v) = find_bytes(seg, 4) {
                // Avoid Cow allocation on the common valid-UTF-8 path
                match std::str::from_utf8(v) {
                    Ok(s) => preedit.push_str(s),
                    Err(_) => preedit.push_str(&String::from_utf8_lossy(v)),
                }
            }
            if !out.preedit_has_highlight && find_varint(seg, 3) == Some(annotation::HIGHLIGHT) {
                out.preedit_has_highlight = true;
            }
        }
        out.preedit_text = preedit;
    }

    Ok(out)
}
