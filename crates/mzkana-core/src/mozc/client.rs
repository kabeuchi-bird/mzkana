use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::codec::DecodeError;
use super::proto::{
    decode_response, encode_command, input_create_session, input_delete_session,
    input_revert, input_send_kana, input_send_special, input_submit,
    special_key, DecodedOutput,
};
use super::MozcOutput;

/// Maximum accepted response frame size (1 MiB). Responses larger than this
/// are rejected to prevent allocation of attacker-controlled memory.
const MAX_FRAME_SIZE: usize = 1024 * 1024;

/// Default path to the Mozc server socket.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".mozc").join("session.sock")
}

#[derive(Debug)]
pub enum MozcError {
    Io(io::Error),
    Decode(DecodeError),
    Protocol(String),
    NotConnected,
}

impl std::fmt::Display for MozcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Protocol(s) => write!(f, "protocol error: {s}"),
            Self::NotConnected => write!(f, "no active Mozc session"),
        }
    }
}

impl std::error::Error for MozcError {}

impl From<io::Error> for MozcError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}
impl From<DecodeError> for MozcError {
    fn from(e: DecodeError) -> Self { Self::Decode(e) }
}

/// Blocking Mozc IPC client over a Unix domain socket.
///
/// Wire framing: `uint32_le(length) | proto_bytes` in both directions.
pub struct MozcClient {
    stream: UnixStream,
    pub session_id: Option<u64>,
}

impl MozcClient {
    /// Connect to the Mozc server socket and create a session.
    pub fn connect(socket_path: Option<&Path>) -> Result<Self, MozcError> {
        let path = match socket_path {
            Some(p) => p.to_path_buf(),
            None => default_socket_path(),
        };
        let stream = UnixStream::connect(&path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut client = Self { stream, session_id: None };
        client.create_session()?;
        Ok(client)
    }

    fn send_recv(&mut self, input: &super::proto::EncodedInput) -> Result<DecodedOutput, MozcError> {
        let cmd_bytes = encode_command(input);
        // Write: 4-byte LE length + proto bytes
        let len = cmd_bytes.len() as u32;
        self.stream.write_all(&len.to_le_bytes())?;
        self.stream.write_all(&cmd_bytes)?;
        self.stream.flush()?;
        // Read response: 4-byte LE length + proto bytes
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        if resp_len > MAX_FRAME_SIZE {
            return Err(MozcError::Protocol(format!(
                "response frame too large: {resp_len} > {MAX_FRAME_SIZE}"
            )));
        }
        let mut resp_buf = vec![0u8; resp_len];
        self.stream.read_exact(&mut resp_buf)?;
        let out = decode_response(&resp_buf)?;
        Ok(out)
    }

    fn create_session(&mut self) -> Result<(), MozcError> {
        let out = self.send_recv(&input_create_session())?;
        let id = out
            .session_id
            .ok_or_else(|| MozcError::Protocol("CREATE_SESSION returned no id".into()))?;
        self.session_id = Some(id);
        Ok(())
    }

    fn sid(&self) -> Result<u64, MozcError> {
        self.session_id.ok_or(MozcError::NotConnected)
    }

    /// Send a kana string to Mozc preedit (DIRECT_INPUT style).
    pub fn send_kana(&mut self, kana: &str) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_send_kana(sid, kana))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Send a BackSpace key.
    pub fn send_backspace(&mut self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_send_special(sid, special_key::BACKSPACE))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Submit (commit) the current preedit.
    pub fn submit(&mut self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_submit(sid))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Revert (cancel) the current preedit.
    pub fn revert(&mut self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_revert(sid))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Send a Space key (typically starts conversion).
    pub fn send_space(&mut self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_send_special(sid, special_key::SPACE))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Send an Enter key.
    pub fn send_enter(&mut self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        let out = self.send_recv(&input_send_special(sid, special_key::ENTER))?;
        Ok(MozcOutput::from_decoded(out))
    }

    /// Send a function key by its XKB keysym name (e.g. `"Return"`, `"Up"`, `"F1"`).
    /// Returns `Err(Protocol)` for names that have no Mozc SpecialKey mapping.
    pub fn send_function_key(&mut self, name: &str) -> Result<MozcOutput, MozcError> {
        let code = xkb_name_to_mozc_special(name).ok_or_else(|| {
            MozcError::Protocol(format!("no Mozc SpecialKey for function key: {name}"))
        })?;
        let sid = self.sid()?;
        let out = self.send_recv(&input_send_special(sid, code))?;
        Ok(MozcOutput::from_decoded(out))
    }
}

/// Map an XKB keysym name (as used after `!` in layout files) to a Mozc SpecialKey value.
pub fn xkb_name_to_mozc_special(name: &str) -> Option<u64> {
    match name {
        "Return"              => Some(special_key::ENTER),
        "Tab"                 => Some(special_key::TAB),
        "Escape"              => Some(special_key::ESCAPE),
        "BackSpace"           => Some(special_key::BACKSPACE),
        "Delete"              => Some(special_key::DEL),
        "Insert"              => Some(special_key::INSERT),
        "Home"                => Some(special_key::HOME),
        "End"                 => Some(special_key::END),
        "Up"                  => Some(special_key::UP),
        "Down"                => Some(special_key::DOWN),
        "Left"                => Some(special_key::LEFT),
        "Right"               => Some(special_key::RIGHT),
        "Prior" | "PageUp"    => Some(special_key::PAGE_UP),
        "Next"  | "PageDown"  => Some(special_key::PAGE_DOWN),
        "space"               => Some(special_key::SPACE),
        "Henkan"              => Some(special_key::HENKAN),
        "Hiragana_Katakana"   => Some(special_key::KANA),
        // F1–F12: SpecialKey values 19–30
        s if s.starts_with('F') => s[1..].parse::<u64>().ok()
            .filter(|&n| n >= 1 && n <= 12)
            .map(|n| 18 + n),
        _ => None,
    }
}

impl Drop for MozcClient {
    fn drop(&mut self) {
        if let Some(sid) = self.session_id.take() {
            let _ = self.send_recv(&input_delete_session(sid));
        }
    }
}
