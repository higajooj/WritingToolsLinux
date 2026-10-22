//! User-editable popup buttons (`options.json`).

use std::io;
use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::paths;

pub const DEFAULT_OPTIONS: &str = include_str!("../assets/default_options.json");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OptionEntry {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub open_in_window: bool,
    /// Popup-local shortcut; absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
    /// Provider name to use instead of the default one; absent means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model to use instead of the one set for the provider in Settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A reusable base: not shown as a button, its instruction is added to
    /// the buttons that name it as their `base`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_base: bool,
    /// Name of the base action whose instruction comes before this one's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Fields written by other versions are kept as they are.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Buttons keyed by name, in display order.
pub type Options = IndexMap<String, OptionEntry>;

pub fn defaults() -> Options {
    serde_json::from_str(DEFAULT_OPTIONS).expect("bundled default_options.json is valid")
}

/// The instruction of `entry`'s base action. A base that no longer exists,
/// is no longer a base, or is empty is ignored.
pub fn base_instruction<'a>(options: &'a Options, entry: &OptionEntry) -> Option<&'a str> {
    let base = options.get(entry.base.as_deref()?)?;
    let instruction = base.instruction.trim();
    (base.is_base && !instruction.is_empty()).then_some(instruction)
}

/// Load user options, seeding them from the bundled defaults if absent.
pub fn load() -> io::Result<Options> {
    load_from(&paths::options_path())
}

pub fn save(options: &Options) -> io::Result<()> {
    save_to(&paths::options_path(), options)
}

fn load_from(path: &Path) -> io::Result<Options> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_OPTIONS)?;
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn save_to(path: &Path, options: &Options) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(options).map_err(io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_defaults_then_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/options.json");
        let mut options = load_from(&path).unwrap();
        assert_eq!(options, defaults());
        assert_eq!(options.keys().next().map(String::as_str), Some("Proofread"));

        options.shift_remove("Rewrite");
        options.get_mut("Proofread").unwrap().hotkey = Some("ctrl+1".into());
        save_to(&path, &options).unwrap();
        let reloaded = load_from(&path).unwrap();
        assert_eq!(reloaded, options);
        assert!(!reloaded.contains_key("Rewrite"));
    }

    #[test]
    fn keeps_unknown_fields_and_omits_empty_hotkey() {
        let parsed: Options = serde_json::from_str(
            r#"{"A": {"prefix": "p", "instruction": "i", "icon": "x", "open_in_window": true, "future": 1}}"#,
        )
        .unwrap();
        let text = serde_json::to_string(&parsed).unwrap();
        assert!(text.contains(r#""future":1"#));
        assert!(!text.contains("hotkey"));
        assert!(!text.contains("provider") && !text.contains("model"));
        assert!(!text.contains("is_base") && !text.contains("base"));
    }

    #[test]
    fn base_round_trips_and_resolves() {
        let mut parsed: Options = serde_json::from_str(
            r#"{"B": {"instruction": " No em-dashes. ", "is_base": true}, "A": {"instruction": "i", "base": "B"}}"#,
        )
        .unwrap();
        assert!(parsed["A"].extra.is_empty() && parsed["B"].extra.is_empty());
        let text = serde_json::to_string(&parsed).unwrap();
        assert!(text.contains(r#""is_base":true"#) && text.contains(r#""base":"B""#));
        assert_eq!(base_instruction(&parsed, &parsed["A"]), Some("No em-dashes."));
        assert_eq!(base_instruction(&parsed, &parsed["B"]), None);

        parsed.get_mut("B").unwrap().is_base = false;
        assert_eq!(base_instruction(&parsed, &parsed["A"]), None);
        parsed.shift_remove("B");
        assert_eq!(base_instruction(&parsed, &parsed["A"]), None);
    }

    #[test]
    fn provider_and_model_round_trip() {
        let parsed: Options = serde_json::from_str(r#"{"A": {"provider": "OpenRouter", "model": "x/y"}}"#).unwrap();
        assert_eq!(parsed["A"].provider.as_deref(), Some("OpenRouter"));
        assert_eq!(parsed["A"].model.as_deref(), Some("x/y"));
        assert!(parsed["A"].extra.is_empty());
        let text = serde_json::to_string(&parsed).unwrap();
        assert!(text.contains(r#""provider":"OpenRouter","model":"x/y""#));
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("options.json");
        std::fs::write(&path, "{").unwrap();
        assert_eq!(load_from(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
