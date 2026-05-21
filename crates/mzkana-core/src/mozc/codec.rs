/// Minimal protobuf wire-format encoder/decoder.
///
/// Handles the wire types present in Mozc's commands.proto:
/// - 0 (Varint): integers, bools, enums
/// - 2 (LEN): strings, bytes, embedded messages
/// - 3/4 (Start/End group): Preedit.Segment in proto2 group syntax
use std::fmt;

const WT_VARINT: u64 = 0;
const WT_LEN: u64 = 2;
const WT_START_GROUP: u64 = 3;
const WT_END_GROUP: u64 = 4;

// ── Encoder ──────────────────────────────────────────────────────────────────

pub fn write_varint(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(b);
            return;
        }
        buf.push(b | 0x80);
    }
}

fn write_tag(buf: &mut Vec<u8>, field: u32, wire: u64) {
    write_varint(buf, ((field as u64) << 3) | wire);
}

/// Encode a varint field (integer, bool, enum).
pub fn write_varint_field(buf: &mut Vec<u8>, field: u32, value: u64) {
    write_tag(buf, field, WT_VARINT);
    write_varint(buf, value);
}

/// Encode a length-delimited field (string, bytes, embedded message).
pub fn write_len_field(buf: &mut Vec<u8>, field: u32, data: &[u8]) {
    write_tag(buf, field, WT_LEN);
    write_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

// ── Decoder ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct DecodeError(pub String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "protobuf decode error: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

fn err(msg: impl Into<String>) -> DecodeError {
    DecodeError(msg.into())
}

/// A decoded protobuf field value.
#[derive(Debug, Clone)]
pub enum Value {
    Varint(u64),
    Bytes(Vec<u8>),
    Group(Vec<Field>),
}

/// A single decoded field: (field_number, value).
pub type Field = (u32, Value);

fn read_varint(data: &[u8], pos: &mut usize) -> Result<u64, DecodeError> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        if *pos >= data.len() {
            return Err(err("truncated varint"));
        }
        let b = data[*pos];
        *pos += 1;
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err(err("varint overflow"));
        }
    }
}

fn decode_into(
    data: &[u8],
    pos: &mut usize,
    end_group_field: Option<u32>,
) -> Result<Vec<Field>, DecodeError> {
    let mut fields = Vec::new();
    while *pos < data.len() {
        let tag = read_varint(data, pos)?;
        let field_num = (tag >> 3) as u32;
        let wire = tag & 0x7;

        match wire {
            WT_VARINT => {
                let v = read_varint(data, pos)?;
                fields.push((field_num, Value::Varint(v)));
            }
            WT_LEN => {
                let len = read_varint(data, pos)? as usize;
                let end = *pos + len;
                if end > data.len() {
                    return Err(err("LEN field exceeds buffer"));
                }
                fields.push((field_num, Value::Bytes(data[*pos..end].to_vec())));
                *pos = end;
            }
            WT_START_GROUP => {
                let nested = decode_into(data, pos, Some(field_num))?;
                fields.push((field_num, Value::Group(nested)));
            }
            WT_END_GROUP => {
                if end_group_field == Some(field_num) {
                    return Ok(fields);
                }
                return Err(err(format!("unexpected end-group for field {field_num}")));
            }
            wt => return Err(err(format!("unsupported wire type {wt}"))),
        }
    }
    if end_group_field.is_some() {
        return Err(err("missing end-group tag"));
    }
    Ok(fields)
}

/// Decode all top-level protobuf fields from `data`.
pub fn decode(data: &[u8]) -> Result<Vec<Field>, DecodeError> {
    let mut pos = 0;
    decode_into(data, &mut pos, None)
}

// ── Helper getters ────────────────────────────────────────────────────────────

/// Find the first field with the given number and return its varint value.
pub fn find_varint(fields: &[Field], field_num: u32) -> Option<u64> {
    fields.iter().find_map(|(n, v)| {
        (*n == field_num).then(|| if let Value::Varint(x) = v { Some(*x) } else { None }).flatten()
    })
}

/// Find the first field with the given number and return its bytes/string value.
pub fn find_bytes<'a>(fields: &'a [Field], field_num: u32) -> Option<&'a [u8]> {
    fields.iter().find_map(|(n, v)| {
        (*n == field_num).then(|| if let Value::Bytes(b) = v { Some(b.as_slice()) } else { None }).flatten()
    })
}

/// Decode a `Value` that is expected to be an embedded message or group.
/// Returns `Ok(None)` for other value types (silently skipped).
fn decode_msg_value(v: &Value) -> Result<Option<Vec<Field>>, DecodeError> {
    match v {
        Value::Bytes(b) => decode(b).map(Some),
        Value::Group(g) => Ok(Some(g.clone())),
        _ => Ok(None),
    }
}

/// Find the first field with the given number and return nested group/message fields.
/// Returns `Err` if the field is present but its bytes cannot be decoded.
pub fn find_msg(fields: &[Field], field_num: u32) -> Result<Option<Vec<Field>>, DecodeError> {
    for (n, v) in fields {
        if *n == field_num {
            return decode_msg_value(v);
        }
    }
    Ok(None)
}

/// Collect all occurrences of a field number as group/message field vectors.
/// Returns `Err` if any occurrence fails to decode.
pub fn find_all_msg(fields: &[Field], field_num: u32) -> Result<Vec<Vec<Field>>, DecodeError> {
    let mut out = Vec::new();
    for (n, v) in fields {
        if *n == field_num {
            if let Some(msg) = decode_msg_value(v)? {
                out.push(msg);
            }
        }
    }
    Ok(out)
}
