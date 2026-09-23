//! Application settings (`config.json`).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths;

pub const DEFAULT_SHORTCUT: &str = "ctrl+space";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Per-provider settings, keyed by provider name.
    #[serde(default)]
    pub providers: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_window_zoom: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Config {
    pub fn shortcut(&self) -> &str {
        match self.shortcut.as_deref() {
            Some(s) if !s.trim().is_empty() => s,
            _ => DEFAULT_SHORTCUT,
        }
    }

    pub fn provider_config(&self, name: &str) -> Map<String, Value> {
        match self.providers.get(name) {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        }
    }
}

/// Load the config, or `None` for a first run.
pub fn load() -> io::Result<Option<Config>> {
    load_from(&paths::config_path())
}

pub fn save(config: &Config) -> io::Result<()> {
    save_to(&paths::config_path(), config)
}

fn load_from(path: &Path) -> io::Result<Option<Config>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn save_to(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    // The file holds provider credentials.
    write_private(path, text.as_bytes())
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // `mode` only applies when the file is created; tighten an existing one too.
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_first_run() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from(&dir.path().join("config.json")).unwrap().is_none());
    }

    #[test]
    fn reads_existing_python_config_and_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"shortcut": "super+p", "provider": "Gemini (Recommended)", "streaming": false,
                "providers": {"Gemini (Recommended)": {"api_key": "enc:Ozg5", "model_name": "gemini-flash-latest"}},
                "response_window_zoom": 1.5}"#,
        )
        .unwrap();
        let config = load_from(&path).unwrap().unwrap();
        assert_eq!(config.shortcut(), "super+p");
        assert_eq!(config.response_window_zoom, Some(1.5));
        assert_eq!(config.provider_config("Gemini (Recommended)")["model_name"], "gemini-flash-latest");
        assert!(config.provider_config("missing").is_empty());

        save_to(&path, &config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("streaming"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn blank_shortcut_uses_default() {
        let config = Config { shortcut: Some(" ".into()), ..Default::default() };
        assert_eq!(config.shortcut(), DEFAULT_SHORTCUT);
    }
}
