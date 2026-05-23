pub(crate) mod codec;
pub(crate) mod proto;
pub mod client;

pub use client::{default_socket_path, find_abstract_socket_name, MozcClient, MozcError};
pub use proto::{composition_mode, DecodedOutput};

/// Decoded Mozc server response exposed to callers.
#[derive(Debug, Clone, Default)]
pub struct MozcOutput {
    /// Current preedit string (text being composed, not yet committed).
    pub preedit: String,
    /// Text committed by Mozc (e.g. after conversion + selection).
    pub result: Option<String>,
    /// True if preedit has a highlighted segment, indicating CONVERSION mode.
    pub is_converting: bool,
    /// Mozc CompositionMode (see `composition_mode` constants).
    pub mode: i32,
    /// Whether the key event was consumed by Mozc.
    pub consumed: bool,
}

impl MozcOutput {
    pub(crate) fn from_decoded(d: DecodedOutput) -> Self {
        Self {
            preedit: d.preedit_text,
            result: d.result_value,
            is_converting: d.preedit_has_highlight,
            mode: d.mode,
            consumed: d.consumed,
        }
    }
}
