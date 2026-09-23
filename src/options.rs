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
    /// Fields written by other versions are kept as they are.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Buttons keyed by name, in display order.
pub type Options = IndexMap<String, OptionEntry>;

pub fn defaults() -> Options {
    serde_json::from_str(DEFAULT_OPTIONS).expect("bundled default_options.json is valid")
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
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("options.json");
        std::fs::write(&path, "{").unwrap();
        assert_eq!(load_from(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
