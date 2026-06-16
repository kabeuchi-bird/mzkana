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

fn fill_buf(dst: &mut [u8], src: &str) -> u32 {
    let max = dst.len().saturating_sub(1);
    let mut len = src.len().min(max);
    while !src.is_char_boundary(len) {
        len -= 1;
    }
    dst[..len].copy_from_slice(&src.as_bytes()[..len]);
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

/// Select and commit the candidate with the given Mozc-internal `candidate_id`
/// (e.g. when the user presses a number key or clicks a candidate). The returned
/// `MzkanaResult.commit` holds the committed text; the candidate window is closed.
///
/// # Safety
/// `engine` must be a valid non-null pointer returned by `mzkana_engine_create`.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_select_candidate(
    engine: *mut MzkanaEngine,
    candidate_id: i32,
) -> MzkanaResult {
    // candidate_id < 0 is the "not selectable" sentinel; treat it as a no-op so it
    // never reaches Mozc as a bogus SELECT_CANDIDATE or clears the UI caches.
    if engine.is_null() || candidate_id < 0 {
        return MzkanaResult::default();
    }
    let engine = &mut *engine;
    result_from(engine.0.select_candidate(candidate_id))
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

/// Handle focus loss, honoring the layout's `on_focus_change` setting
/// (preserve / reset). Call from the C++ `deactivate` handler.
///
/// Returns 1 if the composition state was preserved (caller should NOT clear
/// preedit), 0 if it was reset (caller should clear preedit).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_focus_out(engine: *mut MzkanaEngine) -> u8 {
    match engine.as_mut() {
        Some(e) => {
            let preserved = e.0.focus_out_preserved();
            preserved as u8
        }
        None => 0,
    }
}

/// `sensitive_field_behavior` setting (0 = passthrough, 1 = buffer).
/// Returns 0 for a NULL engine.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_sensitive_field_behavior(engine: *const MzkanaEngine) -> i32 {
    engine.as_ref().map_or(0, |e| e.0.sensitive_field_behavior())
}

/// `preedit_fallback` setting (0 = client, 1 = panel, 2 = buffer, 3 = auto).
/// Returns 1 (panel) for a NULL engine.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_preedit_fallback(engine: *const MzkanaEngine) -> i32 {
    engine.as_ref().map_or(1, |e| e.0.preedit_fallback())
}

/// Resolved `candidate_page_size` (configtool override > TOML > default 5).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_candidate_page_size(engine: *const MzkanaEngine) -> i32 {
    engine.as_ref().map_or(5, |e| e.0.candidate_page_size())
}

/// Resolved `show_prediction` (1 = show, 0 = hide).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_show_prediction(engine: *const MzkanaEngine) -> u8 {
    engine.as_ref().map_or(1, |e| e.0.show_prediction() as u8)
}

/// Set the configtool override for `preedit_fallback`.
/// Pass -1 to clear the override (use TOML value).
/// 0 = client, 1 = panel, 2 = buffer, 3 = auto.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_set_preedit_fallback(engine: *mut MzkanaEngine, value: i32) {
    if let Some(e) = engine.as_mut() {
        e.0.set_preedit_fallback_override(value);
    }
}

/// Set the configtool override for `on_focus_change`.
/// Pass -1 to clear the override (use TOML value).
/// 0 = preserve, 1 = reset.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_set_on_focus_change(engine: *mut MzkanaEngine, value: i32) {
    if let Some(e) = engine.as_mut() {
        e.0.set_on_focus_change_override(value);
    }
}

/// Set the configtool override for `candidate_page_size`.
/// Pass 0 to clear the override (use TOML value).
/// Valid range: 1–9.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_set_candidate_page_size(engine: *mut MzkanaEngine, value: i32) {
    if let Some(e) = engine.as_mut() {
        e.0.set_candidate_page_size_override(value);
    }
}

/// Set the configtool override for `show_prediction`.
/// Pass -1 to clear the override (use TOML value).
/// 0 = hide, 1 = show.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_set_show_prediction(engine: *mut MzkanaEngine, value: i32) {
    if let Some(e) = engine.as_mut() {
        e.0.set_show_prediction_override(value);
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

/// Switch to a different layout file (configtool selection, §13.5) and reload it
/// immediately. `path` is a NUL-terminated UTF-8 path to a layout TOML file.
///
/// Returns 1 on success, 0 on failure (the current layout is kept). On success
/// the hot-reload watch is moved to the new file's directory.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
/// `path` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_reload_layout(
    engine: *mut MzkanaEngine,
    path: *const c_char,
) -> u8 {
    if engine.is_null() || path.is_null() {
        return 0;
    }
    let engine = &mut *engine;
    let path = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    engine.0.reload_layout_from(Path::new(path)) as u8
}

/// Returns 1 if the Mozc conversion engine is connected, 0 otherwise.
///
/// Use this to determine the current engine state for UI display (e.g. status
/// area labels).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_mozc_available(engine: *const MzkanaEngine) -> u8 {
    engine
        .as_ref()
        .map(|e| e.0.mozc_available() as u8)
        .unwrap_or(0)
}

// ── Candidate window accessors (Phase 4) ───────────────────────────────────────
//
// The candidate list is variable-length, so it is exposed separately from the
// fixed-size `MzkanaResult`. After each key event the C++ layer queries the
// count and fetches each entry to (re)build the fcitx5 candidate list. All
// returned string pointers borrow engine-owned memory that stays valid until the
// next key event mutates the engine — the caller must copy, not retain them.

/// One candidate entry. `value` / `annotation` are NUL-terminated UTF-8 borrowed
/// from the engine; `annotation` is NULL when absent. `*_len` excludes the NUL.
#[repr(C)]
pub struct MzkanaCandidate {
    pub value: *const u8,
    pub value_len: u32,
    pub annotation: *const u8,
    pub annotation_len: u32,
    /// Mozc-internal candidate id (for SELECT_CANDIDATE), or -1 when the candidate
    /// has no id and therefore cannot be selected by id.
    pub id: i32,
}

impl Default for MzkanaCandidate {
    fn default() -> Self {
        Self {
            value: std::ptr::null(),
            value_len: 0,
            annotation: std::ptr::null(),
            annotation_len: 0,
            id: -1,
        }
    }
}

/// Number of candidates in the most recent Mozc output (current page).
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_candidate_count(engine: *const MzkanaEngine) -> u32 {
    engine.as_ref().map_or(0, |e| e.0.candidate_count() as u32)
}

/// Fetch the `i`-th candidate. Returns an all-NULL/zero struct if `engine` is
/// NULL or `i` is out of range. Strings are NUL-terminated and engine-owned;
/// valid only until the next key event.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_candidate(
    engine: *const MzkanaEngine,
    i: u32,
) -> MzkanaCandidate {
    let Some(e) = engine.as_ref() else { return MzkanaCandidate::default() };
    let Some(c) = e.0.candidate(i as usize) else { return MzkanaCandidate::default() };
    // value/annotation buffers carry a trailing NUL; report length without it.
    let (annotation, annotation_len) = match &c.annotation {
        Some(a) => (a.as_ptr(), (a.len() - 1) as u32),
        None => (std::ptr::null(), 0),
    };
    MzkanaCandidate {
        value: c.value.as_ptr(),
        value_len: (c.value.len() - 1) as u32,
        annotation,
        annotation_len,
        // -1 signals "no selectable id"; the C++ layer must not call
        // mzkana_engine_select_candidate for such an entry.
        id: c.id.unwrap_or(-1),
    }
}

/// Focused candidate index during conversion, or -1 for a suggestion window /
/// no window / NULL engine.
///
/// # Safety
/// `engine` must be a valid pointer returned by `mzkana_engine_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mzkana_engine_focused_index(engine: *const MzkanaEngine) -> i32 {
    engine
        .as_ref()
        .and_then(|e| e.0.focused_index())
        .map_or(-1, |i| i as i32)
}
