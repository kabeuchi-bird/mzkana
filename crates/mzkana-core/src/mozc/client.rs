use std::cell::Cell;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Maximum response size accepted from the Mozc server (1 MiB).
/// Prevents unbounded memory growth if the server misbehaves.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[cfg(target_os = "linux")]
use std::os::unix::io::FromRawFd;

use super::codec::DecodeError;
use super::proto::{
    decode_response, encode_command, input_create_session, input_delete_session,
    input_revert, input_send_kana, input_send_key_code_with_mods, input_send_special,
    input_send_special_with_mods, input_submit, special_key, DecodedOutput,
};
use super::MozcOutput;

/// Default path to the Mozc server socket (filesystem fallback).
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".mozc").join("session.sock")
}

/// Return the abstract socket name found in /proc/net/unix, or None.
#[cfg(target_os = "linux")]
pub fn find_abstract_socket_name() -> Option<String> {
    let unix_data = std::fs::read_to_string("/proc/net/unix").ok()?;
    unix_data
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .find(|n| n.starts_with('@') && n.contains(".mozc.") && n.ends_with(".session"))
}

#[cfg(not(target_os = "linux"))]
pub fn find_abstract_socket_name() -> Option<String> { None }

/// Connect to an abstract namespace socket by its `@`-prefixed name.
#[cfg(target_os = "linux")]
fn connect_by_abstract_name(abs_name: &str) -> io::Result<UnixStream> {
    let name = abs_name.strip_prefix('@').unwrap_or(abs_name);
    let name_bytes = name.as_bytes();
    if name_bytes.len() > 107 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "abstract socket name too long"));
    }

    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as _;
        let dst = addr.sun_path.as_mut_ptr() as *mut u8;
        std::ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst.add(1), name_bytes.len());

        let path_offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
        let addr_len = (path_offset + 1 + name_bytes.len()) as libc::socklen_t;

        let ret = libc::connect(fd, &addr as *const _ as *const libc::sockaddr, addr_len);
        if ret < 0 {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }

        Ok(UnixStream::from_raw_fd(fd))
    }
}

/// Discover and connect to the Mozc abstract socket.
/// Used by the CLI and diagnostic tooling; `MozcClient` caches the name internally.
#[cfg(target_os = "linux")]
pub fn connect_mozc_abstract() -> io::Result<UnixStream> {
    let abs_name = find_abstract_socket_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Mozc abstract socket not found in /proc/net/unix"))?;
    tracing::info!("connecting to Mozc abstract socket: {abs_name}");
    connect_by_abstract_name(&abs_name)
}

#[cfg(not(target_os = "linux"))]
pub fn connect_mozc_abstract() -> io::Result<UnixStream> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "abstract sockets not supported on this platform"))
}

/// Known paths for the mozc_server binary on common Linux distributions.
static MOZC_SERVER_PATHS: &[&str] = &[
    "/usr/lib/mozc/mozc_server",
    "/usr/lib64/mozc/mozc_server",
    "/usr/local/lib/mozc/mozc_server",
];

/// Try to start mozc_server if it is not already running.
///
/// Searches well-known installation paths and PATH, spawns the server if found,
/// then waits up to `timeout` for the abstract socket to appear in
/// `/proc/net/unix`.  Returns the abstract socket name on success.
///
/// `timeout` should be kept short (≤ 500 ms) because this is called from the
/// fcitx5 main event thread during `Engine::new`.
#[cfg(target_os = "linux")]
fn ensure_mozc_server(timeout: Duration) -> Option<String> {
    // Server already running? Check abstract socket first, then filesystem socket.
    if let Some(name) = find_abstract_socket_name() {
        return Some(name);
    }
    // Filesystem socket: try to connect to verify liveness rather than just
    // checking existence — a stale socket from a crashed server would exist
    // but refuse connections.
    let fs_path = default_socket_path();
    if fs_path.exists() {
        if UnixStream::connect(&fs_path).is_ok() {
            // Socket is live; return None so the caller uses the FS path.
            return None;
        }
        // Stale socket — remove it and fall through to spawn mozc_server.
        tracing::debug!("stale Mozc socket at {}; removing", fs_path.display());
        let _ = std::fs::remove_file(&fs_path);
    }

    // Find the binary: well-known paths first, then PATH search.
    let bin: String = if let Some(&known) = MOZC_SERVER_PATHS.iter()
        .find(|p| std::fs::metadata(p).is_ok())
    {
        known.to_string()
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("mozc_server"))
            .find(|p| p.exists())
            .and_then(|p| p.into_os_string().into_string().ok())?
    };

    tracing::info!("mozc_server not running; launching {bin}");
    let mut child = match std::process::Command::new(&bin)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to spawn {bin}: {e}");
            return None;
        }
    };

    // Brief pause then check for immediate exit (wrong binary, permission denied, etc.).
    std::thread::sleep(Duration::from_millis(50));
    if let Ok(Some(status)) = child.try_wait() {
        tracing::warn!("mozc_server ({bin}) exited immediately: {status}");
        // 抽象ソケットが少し遅れて現れる可能性があるので待機は継続する。
    }
    // Server appears to be running; drop the handle — mozc_server daemonises itself.
    drop(child);

    // Wait for the abstract socket to appear (up to `timeout`).
    let deadline = Instant::now() + timeout;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        if let Some(name) = find_abstract_socket_name() {
            tracing::info!("mozc_server started; socket: {name}");
            return Some(name);
        }
        if Instant::now() >= deadline {
            tracing::warn!("timed out waiting for mozc_server to start");
            return None;
        }
    }
}

/// Kept for backward compatibility with diagnostic tooling in mzkana-cli.
/// The hash in the socket name is NOT a key — authentication is via SO_PEERCRED.
pub fn ipc_key_from_socket_name(abs_name: &str) -> Option<Vec<u8>> {
    let name = abs_name.strip_prefix('@').unwrap_or(abs_name);
    let hash = name.strip_prefix("tmp/.mozc.")?.strip_suffix(".session")?;
    if hash.len() % 2 != 0 {
        return None;
    }
    let bytes: Vec<u8> = (0..hash.len()).step_by(2)
        .filter_map(|i| hash.get(i..i+2).and_then(|s| u8::from_str_radix(s, 16).ok()))
        .collect();
    if bytes.len() == hash.len() / 2 { Some(bytes) } else { None }
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
impl From<io::Error> for MozcError { fn from(e: io::Error) -> Self { Self::Io(e) } }
impl From<DecodeError> for MozcError { fn from(e: DecodeError) -> Self { Self::Decode(e) } }
impl From<prost::DecodeError> for MozcError {
    fn from(e: prost::DecodeError) -> Self {
        Self::Decode(DecodeError(e.to_string()))
    }
}

/// Blocking Mozc IPC client.
///
/// Wire protocol (from Mozc's `unix_ipc.cc`):
///   - NO length framing in either direction.
///   - Client sends raw serialized `Command` protobuf bytes, then calls
///     `shutdown(SHUT_WR)` to signal end-of-request.
///   - Server reads until EOF, processes, sends raw response bytes, then closes.
///   - Each request uses a fresh connection.
///   - Authentication is via SO_PEERCRED (kernel UID check on connect), no key exchange.
///
/// # Thread safety
///
/// `MozcClient` is intentionally `!Sync`: `session_id` is mutated in `Drop`
/// without synchronization. Callers must not share a reference across threads.
/// Ownership may be moved between threads (`Send` is safe because no raw
/// pointers or thread-locals are held).
pub struct MozcClient {
    /// Explicit filesystem socket path, or None to use abstract socket.
    socket_path: Option<PathBuf>,
    /// Cached abstract socket name for reconnections (e.g., "@tmp/.mozc.HASH.session").
    abstract_name: Option<String>,
    session_id: Option<u64>,
    /// Makes `MozcClient` explicitly `!Sync` (see type-level docs).
    _not_sync: PhantomData<Cell<()>>,
}

impl MozcClient {
    /// Quick connection attempt: discover existing sockets only, no server spawn.
    ///
    /// Checks the abstract namespace socket and the default filesystem path.
    /// Returns an error immediately if neither is found or connection fails.
    /// Use this for periodic reconnect attempts on the key-event thread.
    pub fn connect_quick(socket_path: Option<&Path>) -> Result<Self, MozcError> {
        let (sp, abs) = if let Some(path) = socket_path {
            (Some(path.to_path_buf()), None)
        } else {
            #[cfg(target_os = "linux")]
            {
                match find_abstract_socket_name() {
                    Some(name) => (None, Some(name)),
                    None => {
                        let fs = default_socket_path();
                        if fs.exists() {
                            (Some(fs), None)
                        } else {
                            return Err(MozcError::NotConnected);
                        }
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            { (Some(default_socket_path()), None) }
        };

        let mut client = Self {
            socket_path: sp,
            abstract_name: abs,
            session_id: None,
            _not_sync: PhantomData,
        };
        client.create_session()?;
        Ok(client)
    }

    /// Connect to the Mozc server and create a session.
    ///
    /// If `socket_path` is given it is used directly (filesystem socket).
    /// Otherwise, on Linux the abstract namespace socket is discovered from
    /// `/proc/net/unix`; if that fails, the filesystem fallback path is tried.
    pub fn connect(socket_path: Option<&Path>) -> Result<Self, MozcError> {
        let (sp, abs) = if let Some(path) = socket_path {
            (Some(path.to_path_buf()), None)
        } else {
            #[cfg(target_os = "linux")]
            {
                // Auto-start mozc_server if not running (waits up to 500 ms to avoid
                // blocking the fcitx5 main event thread for too long).
                match ensure_mozc_server(Duration::from_millis(500)) {
                    Some(name) => {
                        tracing::info!("Mozc abstract socket: {name}");
                        (None, Some(name))
                    }
                    None => {
                        tracing::debug!("Mozc abstract socket not found; trying filesystem path");
                        (Some(default_socket_path()), None)
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            { (Some(default_socket_path()), None) }
        };

        let mut client = Self {
            socket_path: sp,
            abstract_name: abs,
            session_id: None,
            _not_sync: PhantomData,
        };

        // Verify connection works before returning.
        client.create_session()?;
        Ok(client)
    }

    pub fn session_id(&self) -> u64 {
        self.session_id.unwrap_or(0)
    }

    /// Open a fresh connection to the Mozc server.
    ///
    /// When using an abstract socket, if the cached name fails (e.g. because
    /// mozc_server restarted and got a new socket name), the function
    /// rediscovers the current name from `/proc/net/unix` and retries once.
    fn new_connection(&self) -> io::Result<UnixStream> {
        let stream = if let Some(ref path) = self.socket_path {
            UnixStream::connect(path)?
        } else {
            #[cfg(target_os = "linux")]
            {
                if let Some(ref abs) = self.abstract_name {
                    match connect_by_abstract_name(abs) {
                        Ok(s) => s,
                        Err(_) => {
                            // Cached name stale (server restarted); rediscover and retry.
                            let new_name = find_abstract_socket_name().ok_or_else(|| {
                                io::Error::new(io::ErrorKind::NotFound,
                                    "Mozc abstract socket not found after reconnect attempt")
                            })?;
                            tracing::debug!("abstract socket name changed, reconnecting to {new_name}");
                            connect_by_abstract_name(&new_name)?
                        }
                    }
                } else {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "no Mozc socket configured"));
                }
            }
            #[cfg(not(target_os = "linux"))]
            { return Err(io::Error::new(io::ErrorKind::Unsupported, "abstract sockets unsupported")); }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(stream)
    }

    /// Send a command and receive the response over a fresh connection.
    ///
    /// Mozc IPC wire protocol (confirmed from `unix_ipc.cc` and `session_server.cc`):
    ///   - NO length framing in either direction.
    ///   - Client sends raw `Input` protobuf bytes (via `encode_command`), then
    ///     `shutdown(SHUT_WR)`.  The shutdown signals end-of-request; without it
    ///     the server blocks waiting for more data and the call times out.
    ///   - Server reads until EOF, processes, writes raw `Output` protobuf bytes,
    ///     then closes the connection.
    fn send_recv(&self, input: &super::proto::EncodedInput) -> Result<DecodedOutput, MozcError> {
        let mut stream = self.new_connection()?;
        let cmd_bytes = encode_command(input);

        stream.write_all(&cmd_bytes)?;
        // Half-close the write side so the server knows the request is complete.
        stream.shutdown(std::net::Shutdown::Write)?;

        // Read raw response bytes until the server closes the connection.
        // Bail out early if the accumulated length would exceed MAX_RESPONSE_BYTES
        // to avoid buffering an unboundedly large Vec on a misbehaving server.
        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf)? {
                0 => break,
                n => {
                    if resp.len() + n > MAX_RESPONSE_BYTES {
                        return Err(MozcError::Protocol(format!(
                            "Mozc response length {} exceeds {MAX_RESPONSE_BYTES} bytes",
                            resp.len() + n
                        )));
                    }
                    resp.extend_from_slice(&buf[..n]);
                }
            }
        }

        if resp.is_empty() {
            return Err(MozcError::Protocol("empty response from Mozc server".into()));
        }
        Ok(decode_response(&resp)?)
    }

    fn create_session(&mut self) -> Result<(), MozcError> {
        let out = self.send_recv(&input_create_session())?;
        self.session_id = Some(
            out.session_id
                .ok_or_else(|| MozcError::Protocol("CREATE_SESSION returned no id".into()))?,
        );
        Ok(())
    }

    fn sid(&self) -> Result<u64, MozcError> {
        self.session_id.ok_or(MozcError::NotConnected)
    }

    fn dispatch(&self, input: &super::proto::EncodedInput) -> Result<MozcOutput, MozcError> {
        Ok(MozcOutput::from_decoded(self.send_recv(input)?))
    }

    fn send_special_key(&self, code: i32) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        self.dispatch(&input_send_special(sid, code))
    }

    pub fn send_kana(&self, kana: &str) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        self.dispatch(&input_send_kana(sid, kana))
    }

    pub fn send_backspace(&self) -> Result<MozcOutput, MozcError> {
        self.send_special_key(special_key::BACKSPACE)
    }

    pub fn submit(&self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        self.dispatch(&input_submit(sid))
    }

    pub fn revert(&self) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        self.dispatch(&input_revert(sid))
    }

    pub fn send_space(&self) -> Result<MozcOutput, MozcError> {
        self.send_special_key(special_key::SPACE)
    }

    pub fn send_enter(&self) -> Result<MozcOutput, MozcError> {
        self.send_special_key(special_key::ENTER)
    }

    pub fn send_function_key(&self, name: &str) -> Result<MozcOutput, MozcError> {
        let code = xkb_name_to_mozc_special(name)
            .ok_or_else(|| MozcError::Protocol(format!("no Mozc SpecialKey for: {name}")))?;
        self.send_special_key(code)
    }

    pub fn send_modified_key(&self, key: &str, mods: u8) -> Result<MozcOutput, MozcError> {
        let sid = self.sid()?;
        if let Some(special) = xkb_name_to_mozc_special(key) {
            self.dispatch(&input_send_special_with_mods(sid, special, mods))
        } else if key.len() == 1 && key.is_ascii() {
            let code = key.as_bytes()[0] as u32;
            self.dispatch(&input_send_key_code_with_mods(sid, code, mods))
        } else {
            Err(MozcError::Protocol(format!("no Mozc encoding for key: {key}")))
        }
    }
}

/// Map an XKB keysym name to a Mozc SpecialKey value.
pub fn xkb_name_to_mozc_special(name: &str) -> Option<i32> {
    match name {
        "Return"            => Some(special_key::ENTER),
        "Tab"               => Some(special_key::TAB),
        "Escape"            => Some(special_key::ESCAPE),
        "BackSpace"         => Some(special_key::BACKSPACE),
        "Delete"            => Some(special_key::DEL),
        "Insert"            => Some(special_key::INSERT),
        "Home"              => Some(special_key::HOME),
        "End"               => Some(special_key::END),
        "Up"                => Some(special_key::UP),
        "Down"              => Some(special_key::DOWN),
        "Left"              => Some(special_key::LEFT),
        "Right"             => Some(special_key::RIGHT),
        "Prior" | "PageUp"  => Some(special_key::PAGE_UP),
        "Next"  | "PageDown" => Some(special_key::PAGE_DOWN),
        "space"             => Some(special_key::SPACE),
        "Henkan"            => Some(special_key::HENKAN),
        "Hiragana_Katakana" => Some(special_key::KANA),
        // F1–F12: SpecialKey values 19–30
        s if s.starts_with('F') => s[1..].parse::<i32>().ok()
            .filter(|&n| (1..=12).contains(&n))
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
