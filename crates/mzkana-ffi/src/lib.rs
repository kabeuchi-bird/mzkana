//! C ABI for the MzKana input method engine.
//!
//! All strings are NUL-terminated UTF-8.  The caller must not free any
//! returned string pointers — they are owned by the engine or copied into
//! stack-allocated arrays within `MzkanaResult`.
mod engine;

use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::Path;

use engine::Engine;

// ── Public opaque type ────────────────────────────────────────────────────────

/// Opaque engine handle returned by `mzkana_engine_create`.
pub struct MzkanaEngine(Engine);

// ── Result struct ─────────────────────────────────────────────────────────────

/// Result of processing a key event.
///
/// String fields are NUL-terminated UTF-8 byte arrays.  `*_len` gives the
/// byte count **excluding** the NUL terminator, so callers can construct a
/// `std::string` with `std::string(field, field_len)`.
#[repr(C)]
pub struct MzkanaResult {
    /// 1 if the key event was consumed by the IME; 0 if it should be
    /// forwarded to the application unchanged.
    pub consumed: u8,

    /// Current preedit / composition string (may be empty).
    pub preedit: [u8; 512],
    pub preedit_len: u32,

    /// Text to commit to the application (may be empty).
    pub commit: [u8; 512],
    pub commit_len: u32,

    /// If `consumed == 0`, the key name to pass through (may be empty).
    pub passthrough_key: [u8; 64],
    pub passthrough_key_len: u32,

    /// XKB keysym name of a key to forward to the application with modifier synthesis.
    /// Non-empty only when a modifier+key token was not consumed by Mozc.
    /// The C++ layer should call ic->forwardKey() with this key and `forward_modifiers`.
    pub forward_key: [u8; 64],
    pub forward_key_len: u32,
    /// Modifier bitmask for `forward_key`: bit0=Shift, bit1=Ctrl, bit2=Alt, bit3=Super.
    pub forward_modifiers: u8,
}

impl Default for MzkanaResult {
    fn default() -> Self {
        Self {
            consumed: 0,
            preedit: [0u8; 512],
            preedit_len: 0,
            commit: [0u8; 512],
            commit_len: 0,
            passthrough_key: [0u8; 64],
            passthrough_key_len: 0,
            forward_key: [0u8; 64],
            forward_key_len: 0,
            forward_modifiers: 0,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Write `src` into `dst` as NUL-terminated bytes, truncating if needed.
/// Returns the number of bytes written (excluding NUL).
fn fill_buf(dst: &mut [u8], src: &str) -> u32 {
    let bytes = src.as_bytes();
    let len = bytes.len().min(dst.len().saturating_sub(1));
    dst[..len].copy_from_slice(&bytes[..len]);
    dst[len] = 0;
    len as u32
}

fn result_from(out: engine::ProcessResult) -> MzkanaResult {
    let mut r = MzkanaResult::default();
    r.consumed = out.consumed as u8;
    r.preedit_len = fill_buf(&mut r.preedit, &out.preedit);
    if let Some(ref c) = out.commit {
        r.commit_len = fill_buf(&mut r.commit, c);
    }
    if let Some(ref p) = out.passthrough_key {
        r.passthrough_key_len = fill_buf(&mut r.passthrough_key, p);
    }
    if let Some(ref fk) = out.forward_key {
        r.forward_key_len = fill_buf(&mut r.forward_key, fk);
        r.forward_modifiers = out.forward_mods;
    }
    r
}

// ── C API ─────────────────────────────────────────────────────────────────────

/// Create a new engine.
///
/// `config_path`  — NUL-terminated UTF-8 path to the layout TOML file.
/// `socket_path`  — NUL-terminated UTF-8 path to the Mozc UDS socket, or
///                  NULL to use the default (`~/.mozc/session.sock`).
///
/// Returns NULL on failure (logs the reason to stderr).
///
/// # Safety
/// `config_path` must be a valid NUL-terminated C string.
/// `socket_path` must be a valid NUL-terminated C string or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_create(
    config_path: *const c_char,
    socket_path: *const c_char,
) -> *mut MzkanaEngine {
    if config_path.is_null() {
        return std::ptr::null_mut();
    }
    let config_str = match CStr::from_ptr(config_path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let socket: Option<std::path::PathBuf> = if socket_path.is_null() {
        None
    } else {
        CStr::from_ptr(socket_path)
            .to_str()
            .ok()
            .map(std::path::PathBuf::from)
    };

    match Engine::new(Path::new(config_str), socket.as_deref()) {
        Ok(inner) => Box::into_raw(Box::new(MzkanaEngine(inner))),
        Err(e) => {
            eprintln!("mzkana_engine_create: {e}");
            std::ptr::null_mut()
        }
    }
}

/// Destroy an engine previously created with `mzkana_engine_create`.
/// Passing NULL is safe and does nothing.
///
/// # Safety
/// `engine` must be a pointer returned by `mzkana_engine_create`, or NULL.
/// After this call the pointer is invalid and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_destroy(engine: *mut MzkanaEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// Process a key-down event.
///
/// `key_name` — XKB keysym name at level 0, e.g. `"a"`, `"comma"`, `"space"`,
///              `"henkan"`.
/// `shift`    — 1 if the Shift key is held, 0 otherwise.
///
/// # Safety
/// `engine` must be a valid non-null pointer returned by `mzkana_engine_create`.
/// `key_name` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_key_down(
    engine: *mut MzkanaEngine,
    key_name: *const c_char,
    shift: u8,
) -> MzkanaResult {
    if engine.is_null() || key_name.is_null() {
        return MzkanaResult::default();
    }
    let engine = &mut *engine;
    let key = match CStr::from_ptr(key_name).to_str() {
        Ok(s) => s,
        Err(_) => return MzkanaResult::default(),
    };
    result_from(engine.0.key_event(key, true, shift != 0))
}

/// Process a key-up event.
///
/// # Safety
/// `engine` must be a valid non-null pointer returned by `mzkana_engine_create`.
/// `key_name` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_key_up(
    engine: *mut MzkanaEngine,
    key_name: *const c_char,
) -> MzkanaResult {
    if engine.is_null() || key_name.is_null() {
        return MzkanaResult::default();
    }
    let engine = &mut *engine;
    let key = match CStr::from_ptr(key_name).to_str() {
        Ok(s) => s,
        Err(_) => return MzkanaResult::default(),
    };
    result_from(engine.0.key_event(key, false, false))
}

/// Advance internal timers.  Call from an fcitx5 timer callback with the
/// chord-window interval (e.g. every 10 ms while preedit is active).
///
/// # Safety
/// `engine` must be a valid non-null pointer returned by `mzkana_engine_create`.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_tick(engine: *mut MzkanaEngine) -> MzkanaResult {
    if engine.is_null() {
        return MzkanaResult::default();
    }
    let engine = &mut *engine;
    result_from(engine.0.tick())
}

/// Reset engine state (call on focus loss or IM deactivation).
/// Any pending preedit is discarded.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_reset(engine: *mut MzkanaEngine) {
    if let Some(engine) = engine.as_mut() {
        engine.0.reset();
    }
}

/// Check whether the config file changed and reload it if so.
///
/// Returns 1 if the config was reloaded, 0 otherwise.
/// Call this periodically (e.g. once per key event or from a timer).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_check_reload(engine: *mut MzkanaEngine) -> u8 {
    engine
        .as_mut()
        .map(|e| e.0.check_reload() as u8)
        .unwrap_or(0)
}
