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
    #[serde(default = "default_tap_send_key")]
    pub tap_action: TapAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModifierKind {
    Hold,
    Oneshot,
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
    SendKey,
    Passthrough,
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
fn default_tap_send_key() -> TapAction {
    TapAction::SendKey
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
fn default_tap_passthrough() -> TapAction {
    TapAction::Passthrough
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

// ── Grid parsing ──────────────────────────────────────────────────────────────

/// QWERTY keyboard rows used for implicit row-to-key mapping.
///   row 0 → number row (1 2 3 4 5 6 7 8 9 0 minus equal)
///   row 1 → top row    (q w e r t y u i o p)
///   row 2 → home row   (a s d f g h j k l ;)
///   row 3 → bottom row (z x c v b n m , . /)
const QWERTY_ROW0: &[&str] = &["1","2","3","4","5","6","7","8","9","0","minus","equal","yen"];
const QWERTY_ROW1: &[&str] = &["q","w","e","r","t","y","u","i","o","p"];
const QWERTY_ROW2: &[&str] = &["a","s","d","f","g","h","j","k","l","semicolon"];
const QWERTY_ROW3: &[&str] = &["z","x","c","v","b","n","m","comma","period","slash"];

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
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }

        let row_label = cols[0];

        // A line starting with `.` is a new explicit key header
        if row_label == "." {
            explicit_keys = Some(cols[1..].iter().map(|s| s.to_string()).collect());
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
        for (i, kana) in values.iter().enumerate() {
            if i >= key_row.len() || i >= col_count {
                break;
            }
            if *kana == "." || kana.is_empty() {
                continue;
            }
            map.insert(key_row[i].to_string(), kana.to_string());
        }
    }

    Ok(map)
}

// ── Layout (compiled) ─────────────────────────────────────────────────────────

/// Compiled layout ready for use by the state machine.
#[derive(Debug, Clone)]
pub struct Layout {
    pub meta: Meta,
    pub settings: Settings,
    pub modifiers: Vec<ModifierDef>,
    pub direct_trigger: Option<DirectTriggerDef>,
    /// base layer grid: key_id → kana
    pub base_layer: HashMap<String, String>,
    /// prefix layers: trigger_key → (key_id → kana)
    pub prefix_layers: Vec<(String, HashMap<String, String>)>,
    /// postfix layers: trigger_key → (key_id → kana)
    pub postfix_layers: Vec<(String, HashMap<String, String>)>,
    /// modified layers: modifier_id → (key_id → kana)
    pub modified_layers: Vec<(String, HashMap<String, String>)>,
    pub chords: Vec<ChordRule>,
    pub directs: Vec<DirectRule>,
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
        };

        layout.validate()?;
        Ok(layout)
    }

    fn validate(&self) -> Result<()> {
        // Check for same-context conflicts
        self.check_direct_conflicts()?;
        self.check_modifier_key_overlap()?;
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
