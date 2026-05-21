use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

// ── Top-level layout file ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LayoutFile {
    pub meta: Meta,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub modifier: Vec<ModifierDef>,
    pub direct_trigger: Option<DirectTriggerDef>,
    #[serde(default)]
    pub layer: Vec<LayerDef>,
    #[serde(default)]
    pub chord: Vec<ChordRule>,
    #[serde(default)]
    pub direct: Vec<DirectRule>,
    /// Named output sequences, e.g. `ku_ret = "、 !Enter"`.
    /// Each `[[alias]]` table contributes zero or more name→sequence pairs.
    #[serde(default)]
    pub alias: Vec<HashMap<String, String>>,
}

// ── Meta ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Meta {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub author: String,
    pub mode: LayoutMode,
    #[serde(default = "default_schema")]
    pub schema: u32,
}

fn default_version() -> String {
    "1.0".to_string()
}
fn default_schema() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    Kana,
    Kanchoku,
    Hybrid,
}

// ── Settings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    pub chord_window_ms: u32,
    pub mutual_window_ms: u32,
    pub caps_lock_behavior: CapsLockBehavior,
    pub on_focus_change: OnFocusChange,
    pub roll_over: bool,
    pub preedit_fallback: PreeditFallback,
    pub sensitive_field_behavior: SensitiveFieldBehavior,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            chord_window_ms: 50,
            mutual_window_ms: 80,
            caps_lock_behavior: CapsLockBehavior::Shift,
            on_focus_change: OnFocusChange::Preserve,
            roll_over: true,
            preedit_fallback: PreeditFallback::Panel,
            sensitive_field_behavior: SensitiveFieldBehavior::Passthrough,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CapsLockBehavior {
    Shift,
    Ignore,
    Passthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OnFocusChange {
    Preserve,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PreeditFallback {
    Client,
    Panel,
    Buffer,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SensitiveFieldBehavior {
    Passthrough,
    Buffer,
}

// ── Modifier ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ModifierDef {
    pub id: String,
    pub key: String,
    #[serde(default = "default_hold")]
    pub kind: ModifierKind,
    #[serde(default = "default_interrupt")]
    pub hold_detection: HoldDetection,
    #[serde(default = "default_hold_timeout")]
    pub hold_timeout_ms: u32,
    #[serde(default = "default_tap_passthrough")]
    pub tap_action: TapAction,
    /// Output string emitted on tap when `tap_action = "output"`.
    /// Supports alias names, `!FunctionKey`, and space-separated multi-token sequences.
    #[serde(default)]
    pub tap_output: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModifierKind {
    Hold,
    Oneshot,
    /// Tap once to activate the layer; tap again to deactivate (sticky/Caps-Lock style).
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HoldDetection {
    Interrupt,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TapAction {
    /// Pass the key through to the application unchanged (formerly also "send_key").
    #[serde(alias = "send_key")]
    Passthrough,
    /// Emit the base-layer kana for this key on tap (dual-role key).
    BaseKana,
    /// Emit the string in the `tap_output` field on tap.
    /// Supports alias names, `!FunctionKey`, and multi-token sequences.
    Output,
    /// Do nothing on tap.
    None,
}

fn default_hold() -> ModifierKind {
    ModifierKind::Hold
}
fn default_interrupt() -> HoldDetection {
    HoldDetection::Interrupt
}
fn default_hold_timeout() -> u32 {
    150
}
fn default_tap_passthrough() -> TapAction {
    TapAction::Passthrough
}

// ── DirectTrigger ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DirectTriggerDef {
    pub keys: Vec<String>,
    #[serde(default = "default_trigger_kind")]
    pub kind: TriggerKind,
    #[serde(default = "default_interrupt")]
    pub hold_detection: HoldDetection,
    #[serde(default = "default_hold_timeout")]
    pub hold_timeout_ms: u32,
    #[serde(default = "default_tap_passthrough")]
    pub tap_action: TapAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    Hold,
    Toggle,
}

fn default_trigger_kind() -> TriggerKind {
    TriggerKind::Hold
}

// ── Layer ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LayerDef {
    pub id: String,
    pub kind: LayerKind,
    pub trigger: Option<String>,
    pub modifier: Option<String>,
    pub grid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    Single,
    Prefix,
    Postfix,
    Modified,
}

// ── ChordRule ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ChordRule {
    pub keys: Vec<String>,
    pub output: String,
    #[serde(default)]
    pub symmetric: bool,
    pub window_ms: Option<u32>,
}

// ── DirectRule ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DirectRule {
    pub sequence: Vec<String>,
    pub output: String,
}

// ── Function key registry ─────────────────────────────────────────────────────

/// XKB keysym names accepted after the `!` prefix in output strings.
/// These are passed through to the fcitx5 layer as-is.
pub static VALID_FUNCTION_KEYS: phf::Set<&'static str> = phf::phf_set! {
    // Navigation
    "Return", "Tab", "Escape", "BackSpace", "Delete",
    "Home", "End", "Insert",
    "Up", "Down", "Left", "Right",
    "Prior", "Next",          // PageUp / PageDown
    "PageUp", "PageDown",     // common aliases
    // Function keys
    "F1",  "F2",  "F3",  "F4",  "F5",  "F6",
    "F7",  "F8",  "F9",  "F10", "F11", "F12",
    // Editing / IME
    "space", "Henkan", "Muhenkan", "Hiragana_Katakana",
};

// ── Grid parsing ──────────────────────────────────────────────────────────────

/// Sentinel stored in layer maps for grid cells marked `XX` (key passthrough).
/// Must not appear in normal kana/alias output.
pub const PASSTHROUGH_CELL: &str = "\x00pt";

/// QWERTY keyboard rows used for implicit row-to-key mapping.
///   row 0 → number row (1 2 3 4 5 6 7 8 9 0 minus equal yen)
///   row 1 → top row    (tab q w e r t y u i o p bracketleft bracketright backslash)
///   row 2 → home row   (caps_lock a s d f g h j k l semicolon quote)
///   row 3 → bottom row (lshift z x c v b n m comma period slash intlro)
const QWERTY_ROW0: &[&str] = &["1","2","3","4","5","6","7","8","9","0","minus","equal","yen"];
const QWERTY_ROW1: &[&str] = &["tab","q","w","e","r","t","y","u","i","o","p","bracketleft","bracketright","backslash"];
const QWERTY_ROW2: &[&str] = &["caps_lock","a","s","d","f","g","h","j","k","l","semicolon","quote"];
const QWERTY_ROW3: &[&str] = &["lshift","z","x","c","v","b","n","m","comma","period","slash","intlro"];

/// Parse a grid string into a map of key_id → kana.
///
/// The header line (`. col1 col2 ...`) defines column positions (visual only).
/// Numbered rows map those column positions to physical keys by keyboard row:
///   row 0 → 1 2 3 4 5 6 7 8 9 0 minus equal   (number row)
///   row 1 → q w e r t y u i o p               (top row)
///   row 2 → a s d f g h j k l ;               (home row)
///   row 3 → z x c v b n m , . /               (bottom row)
///
/// A mid-grid line starting with `.` switches to an explicit key list for
/// subsequent rows.
pub fn parse_grid(grid: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let lines: Vec<&str> = grid
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return Ok(map);
    }

    // First non-empty line is the column-count header: `. col1 col2 ...`
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    if header.is_empty() || header[0] != "." {
        return Err(ConfigError::GridParse(
            "grid header must start with '.'".to_string(),
        ));
    }
    let col_count = header.len() - 1;

    // Current explicit key row (used when a `. key...` line appears mid-grid)
    let mut explicit_keys: Option<Vec<String>> = None;

    for line in &lines[1..] {
        let cols = tokenize_grid_row(line);
        if cols.len() < 2 {
            continue;
        }

        let row_label = cols[0].as_str();

        // A line starting with `.` is a new explicit key header
        if row_label == "." {
            explicit_keys = Some(cols[1..].to_vec());
            continue;
        }

        // Determine the key row for this data row
        let key_row: Vec<&str> = if let Some(ref ek) = explicit_keys {
            ek.iter().map(String::as_str).collect()
        } else {
            match row_label {
                "0" => QWERTY_ROW0.to_vec(),
                "1" => QWERTY_ROW1.to_vec(),
                "2" => QWERTY_ROW2.to_vec(),
                "3" => QWERTY_ROW3.to_vec(),
                other => {
                    return Err(ConfigError::GridParse(format!(
                        "unknown row label '{other}' (expected 0-3 or an explicit '.' header)"
                    )));
                }
            }
        };

        let values = &cols[1..];
        for (i, value) in values.iter().enumerate() {
            if i >= key_row.len() || i >= col_count {
                break;
            }
            // TODO: ASCII "." は後方互換のため残存。将来のスキーマ改訂で削除予定。
            if *value == "＿" || *value == "." || value.is_empty() {
                continue;
            }
            let stored = if *value == "XX" { PASSTHROUGH_CELL } else { value.as_str() };
            map.insert(key_row[i].to_string(), stored.to_string());
        }
    }

    Ok(map)
}

/// Split a grid data row into cell tokens, respecting `"..."` quoted strings
/// (which may contain spaces and represent multi-token output sequences).
///
/// Example: `1 あ "、 !Enter" か` → `["1", "あ", "、 !Enter", "か"]`
pub(crate) fn tokenize_grid_row(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();

    loop {
        // Skip whitespace between tokens
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        match chars.peek() {
            None => break,
            Some('"') => {
                chars.next(); // consume opening quote
                let mut tok = String::new();
                for c in chars.by_ref() {
                    if c == '"' { break; }
                    tok.push(c);
                }
                tokens.push(tok);
            }
            _ => {
                let mut tok = String::new();
                while chars.peek().is_some_and(|c| !c.is_ascii_whitespace()) {
                    tok.push(chars.next().unwrap());
                }
                tokens.push(tok);
            }
        }
    }
    tokens
}

// ── Layout (compiled) ─────────────────────────────────────────────────────────

/// Compiled layout ready for use by the state machine.
#[derive(Debug, Clone)]
pub struct Layout {
    pub meta: Meta,
    pub settings: Settings,
    pub modifiers: Vec<ModifierDef>,
    pub direct_trigger: Option<DirectTriggerDef>,
    /// base layer grid: key_id → raw output string
    pub base_layer: HashMap<String, String>,
    /// prefix layers: trigger_key → (key_id → raw output string)
    pub prefix_layers: Vec<(String, HashMap<String, String>)>,
    /// postfix layers: trigger_key → (key_id → raw output string)
    pub postfix_layers: Vec<(String, HashMap<String, String>)>,
    /// modified layers: modifier_id → (key_id → raw output string)
    pub modified_layers: Vec<(String, HashMap<String, String>)>,
    pub chords: Vec<ChordRule>,
    pub directs: Vec<DirectRule>,
    /// Named output sequences: alias_name → Vec of output tokens
    pub aliases: HashMap<String, Vec<String>>,
}

impl Layout {
    pub fn from_file(file: &LayoutFile) -> Result<Self> {
        let mut base_layer = HashMap::new();
        let mut prefix_layers = Vec::new();
        let mut postfix_layers = Vec::new();
        let mut modified_layers = Vec::new();

        for layer in &file.layer {
            let grid = parse_grid(&layer.grid)?;
            match layer.kind {
                LayerKind::Single => {
                    base_layer = grid;
                }
                LayerKind::Prefix => {
                    let trigger = layer.trigger.clone().ok_or_else(|| {
                        ConfigError::MissingField(format!(
                            "layer '{}' kind=prefix requires trigger",
                            layer.id
                        ))
                    })?;
                    prefix_layers.push((trigger, grid));
                }
                LayerKind::Postfix => {
                    let trigger = layer.trigger.clone().ok_or_else(|| {
                        ConfigError::MissingField(format!(
                            "layer '{}' kind=postfix requires trigger",
                            layer.id
                        ))
                    })?;
                    postfix_layers.push((trigger, grid));
                }
                LayerKind::Modified => {
                    let modifier = layer.modifier.clone().ok_or_else(|| {
                        ConfigError::MissingField(format!(
                            "layer '{}' kind=modified requires modifier",
                            layer.id
                        ))
                    })?;
                    modified_layers.push((modifier, grid));
                }
            }
        }

        // Flatten all [[alias]] tables into a single map
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        for alias_table in &file.alias {
            for (name, seq_str) in alias_table {
                let tokens: Vec<String> =
                    seq_str.split_whitespace().map(|s| s.to_string()).collect();
                if tokens.is_empty() {
                    return Err(ConfigError::MissingField(format!(
                        "alias '{name}' has an empty sequence; \
                         at least one token is required"
                    )));
                }
                aliases.insert(name.clone(), tokens);
            }
        }

        let layout = Layout {
            meta: file.meta.clone(),
            settings: file.settings.clone(),
            modifiers: file.modifier.clone(),
            direct_trigger: file.direct_trigger.clone(),
            base_layer,
            prefix_layers,
            postfix_layers,
            modified_layers,
            chords: file.chord.clone(),
            directs: file.direct.clone(),
            aliases,
        };

        layout.validate()?;
        Ok(layout)
    }

    fn validate(&self) -> Result<()> {
        self.check_direct_conflicts()?;
        self.check_modifier_key_overlap()?;
        self.check_function_key_names()?;
        self.check_tap_output()?;
        Ok(())
    }

    fn check_tap_output(&self) -> Result<()> {
        for m in &self.modifiers {
            if m.tap_action == TapAction::Output && m.tap_output.is_none() {
                return Err(ConfigError::MissingField(format!(
                    "modifier '{}': tap_action = \"output\" requires a tap_output field",
                    m.id
                )));
            }
            if m.tap_output.is_some() && m.tap_action != TapAction::Output {
                return Err(ConfigError::GridParse(format!(
                    "modifier '{}': tap_output is set but tap_action is not \"output\"; \
                     remove tap_output or set tap_action = \"output\"",
                    m.id
                )));
            }
        }
        // BaseKana and Output have no effect on direct triggers (statemachine silently
        // ignores them). Reject them early so authors get a clear error.
        if let Some(ref dt) = self.direct_trigger {
            if matches!(dt.tap_action, TapAction::BaseKana | TapAction::Output) {
                return Err(ConfigError::GridParse(format!(
                    "direct_trigger: tap_action = {:?} is not supported; \
                     use \"passthrough\" or \"none\"",
                    dt.tap_action
                )));
            }
        }
        Ok(())
    }

    fn check_function_key_names(&self) -> Result<()> {
        // Collect every raw output string from all rule sources.
        let all_outputs = self
            .chords
            .iter()
            .map(|c| c.output.as_str())
            .chain(self.directs.iter().map(|d| d.output.as_str()))
            .chain(self.base_layer.values().map(String::as_str))
            .chain(
                self.prefix_layers
                    .iter()
                    .flat_map(|(_, g)| g.values().map(String::as_str)),
            )
            .chain(
                self.postfix_layers
                    .iter()
                    .flat_map(|(_, g)| g.values().map(String::as_str)),
            )
            .chain(
                self.modified_layers
                    .iter()
                    .flat_map(|(_, g)| g.values().map(String::as_str)),
            );

        // Each raw output may be a multi-token sequence (space-separated).
        // Alias values are also validated (they may contain !-prefixed tokens).
        let alias_tokens = self
            .aliases
            .values()
            .flat_map(|tokens| tokens.iter().map(String::as_str));

        for token in all_outputs
            .flat_map(|s| s.split_whitespace())
            .chain(alias_tokens)
        {
            if let Some(key_name) = token.strip_prefix('!') {
                if !VALID_FUNCTION_KEYS.contains(key_name) {
                    return Err(ConfigError::GridParse(format!(
                        "unknown function key '!{key_name}'; \
                         see the layout documentation for valid names"
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_direct_conflicts(&self) -> Result<()> {
        // Detect duplicate direct sequences with different outputs
        let mut seen: HashMap<Vec<String>, &str> = HashMap::new();
        for rule in &self.directs {
            if let Some(existing) = seen.get(&rule.sequence) {
                if *existing != rule.output {
                    return Err(ConfigError::Conflict(format!(
                        "duplicate [[direct]] sequence {:?} with different outputs: '{}' vs '{}'",
                        rule.sequence, existing, rule.output
                    )));
                }
            } else {
                seen.insert(rule.sequence.clone(), &rule.output);
            }
        }
        Ok(())
    }

    fn check_modifier_key_overlap(&self) -> Result<()> {
        let modifier_keys: HashSet<&str> =
            self.modifiers.iter().map(|m| m.key.as_str()).collect();

        // Warn if direct_trigger key is also a modifier key
        if let Some(dt) = &self.direct_trigger {
            for key in &dt.keys {
                if modifier_keys.contains(key.as_str()) {
                    tracing::warn!(
                        key = %key,
                        "direct_trigger key is also defined as a modifier key"
                    );
                }
            }
        }

        // Warn if modifier key appears in base layer
        for key in modifier_keys {
            if self.base_layer.contains_key(key) {
                tracing::warn!(
                    key = %key,
                    "modifier key also appears in base layer — base layer entry unreachable"
                );
            }
        }
        Ok(())
    }
}

// ── Loader ────────────────────────────────────────────────────────────────────

pub fn load_layout(toml_str: &str) -> Result<Layout> {
    let file: LayoutFile = toml::from_str(toml_str).map_err(|e| ConfigError::Toml(e.to_string()))?;
    Layout::from_file(&file)
}
