//! AI providers. Each provider turns a system instruction and a prompt into
//! the complete response text; the app decides where that text goes.

pub mod gemini;
pub mod ollama;
pub mod openai_compat;
pub mod subscription;

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::obfuscate;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// A one-shot request, or a follow-up conversation (without the system
/// instruction, which every provider receives separately).
#[derive(Clone, Debug)]
pub enum Prompt {
    Single(String),
    Chat(Vec<Message>),
}

impl Prompt {
    /// The conversation as user/assistant turns.
    pub fn messages(&self) -> Vec<Message> {
        match self {
            Prompt::Single(text) => vec![Message::user(text.clone())],
            Prompt::Chat(messages) => messages.clone(),
        }
    }
}

/// A failure to show the user, as a dialog title and message.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{title}: {message}")]
pub struct ProviderError {
    pub title: String,
    pub message: String,
}

impl ProviderError {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self { title: title.into(), message: message.into() }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn respond(&self, system_instruction: &str, prompt: Prompt) -> Result<String, ProviderError>;
}

/// Shared HTTP client for all HTTP providers.
pub fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            // Local models can take minutes to load and generate.
            .timeout(Duration::from_secs(600))
            .build()
            .expect("failed to build the HTTP client")
    })
}

/// Extract an API error message from a JSON error body, or fall back to the raw text.
pub fn error_detail(status: reqwest::StatusCode, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let message = parsed.as_ref().and_then(|v| {
        v.pointer("/error/message").or_else(|| v.get("error")).and_then(Value::as_str).map(str::to_owned)
    });
    match message {
        Some(message) => format!("{message} (HTTP {})", status.as_u16()),
        None if body.trim().is_empty() => format!("HTTP {status}"),
        None => format!("HTTP {status}: {}", body.trim().chars().take(300).collect::<String>()),
    }
}

// ---------------------------------------------------------------------------
// Provider catalogue and the settings each one exposes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Gemini,
    Subscription,
    OpenAiCompatible,
    Ollama,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 4] =
        [ProviderKind::Gemini, ProviderKind::Subscription, ProviderKind::OpenAiCompatible, ProviderKind::Ollama];

    /// The name shown in Settings, also the key under `providers` in config.json.
    pub fn name(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "Gemini (Recommended)",
            ProviderKind::Subscription => "OpenAI Subscription (ChatGPT)",
            ProviderKind::OpenAiCompatible => "OpenAI Compatible (For Experts)",
            ProviderKind::Ollama => "Ollama (For Experts)",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn description(self) -> &'static str {
        match self {
            ProviderKind::Gemini => {
                "• Google's Gemini is a powerful AI model available for free!\n\
                 • An API key is required to connect to Gemini on your behalf.\n\
                 • Click the button below to get your API key."
            }
            ProviderKind::Subscription => {
                "• Use models included with your ChatGPT plan.\n\
                 • Sign in securely in your browser through the official Codex CLI.\n\
                 • This login is kept separate from your normal Codex CLI account."
            }
            ProviderKind::OpenAiCompatible => {
                "• Connect to ANY OpenAI-compatible API (v1/chat/completions).\n\
                 • You must abide by the service's Terms of Service."
            }
            ProviderKind::Ollama => "• Connect to an Ollama server (local LLM).",
        }
    }

    /// Bundled logo icon name.
    pub fn logo(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "provider_gemini",
            ProviderKind::Subscription | ProviderKind::OpenAiCompatible => "provider_openai",
            ProviderKind::Ollama => "provider_ollama",
        }
    }

    /// Optional help button: (label, URL).
    pub fn link(self) -> Option<(&'static str, &'static str)> {
        match self {
            ProviderKind::Gemini => Some(("Get API Key", "https://aistudio.google.com/app/apikey")),
            ProviderKind::Subscription => None,
            ProviderKind::OpenAiCompatible => {
                Some(("Get OpenAI API Key", "https://platform.openai.com/account/api-keys"))
            }
            ProviderKind::Ollama => Some((
                "Ollama Set-up Instructions",
                "https://github.com/higajooj/WritingToolsLinux#features-and-configuration",
            )),
        }
    }

    /// Generic settings rows. The subscription provider has its own UI.
    pub fn settings(self) -> Vec<SettingSpec> {
        match self {
            ProviderKind::Gemini => gemini::settings(),
            ProviderKind::Subscription => Vec::new(),
            ProviderKind::OpenAiCompatible => openai_compat::settings(),
            ProviderKind::Ollama => ollama::settings(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SettingKind {
    Text,
    /// Text stored obfuscated in config.json.
    Secret,
    /// Preset (label, value) choices; `custom_placeholder` adds a "Custom" entry.
    Dropdown {
        options: Vec<(&'static str, &'static str)>,
        custom_placeholder: Option<&'static str>,
    },
    /// Standard / Fast processing, with an explanation under the row.
    Speed {
        description: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct SettingSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub placeholder: &'static str,
    pub kind: SettingKind,
}

impl SettingSpec {
    pub fn text(key: &'static str, label: &'static str, default: &'static str, placeholder: &'static str) -> Self {
        Self { key, label, default, placeholder, kind: SettingKind::Text }
    }

    /// The effective value from a provider's stored config.
    pub fn value(&self, config: &Map<String, Value>) -> String {
        let stored = match config.get(self.key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => self.default.to_owned(),
            Some(other) => other.to_string(),
        };
        match self.kind {
            SettingKind::Secret => obfuscate::deobfuscate(&stored),
            SettingKind::Speed { .. } => normalize_service_tier(&stored).to_owned(),
            _ => stored,
        }
    }

    /// The value to store in config.json.
    pub fn stored(&self, value: &str) -> Value {
        match self.kind {
            SettingKind::Secret => Value::String(obfuscate::obfuscate(value.trim())),
            SettingKind::Speed { .. } => Value::String(normalize_service_tier(value).to_owned()),
            _ => Value::String(value.to_owned()),
        }
    }
}

pub const SPEED_OPTIONS: [(&str, &str); 2] = [("Standard", "default"), ("Fast (priority)", "priority")];

pub fn normalize_service_tier(value: &str) -> &'static str {
    if value == "priority" { "priority" } else { "default" }
}

/// Build a request-ready provider from its stored settings.
pub fn build(kind: ProviderKind, config: &Map<String, Value>) -> Option<std::sync::Arc<dyn Provider>> {
    let get = |key: &str| {
        kind.settings().into_iter().find(|spec| spec.key == key).map(|spec| spec.value(config)).unwrap_or_default()
    };
    Some(match kind {
        ProviderKind::Gemini => {
            std::sync::Arc::new(gemini::Gemini { api_key: get("api_key"), model: get("model_name") })
        }
        ProviderKind::OpenAiCompatible => std::sync::Arc::new(openai_compat::OpenAiCompatible {
            api_key: get("api_key").trim().to_owned(),
            api_base: get("api_base"),
            organisation: get("api_organisation"),
            project: get("api_project"),
            model: get("api_model"),
            service_tier: get("service_tier"),
        }),
        ProviderKind::Ollama => std::sync::Arc::new(ollama::Ollama {
            api_base: get("api_base"),
            model: get("api_model"),
            keep_alive: get("keep_alive"),
        }),
        // Long-lived; owned by the app rather than rebuilt from settings.
        ProviderKind::Subscription => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_settings_are_obfuscated_at_rest() {
        let spec = gemini::settings().into_iter().find(|s| s.key == "api_key").unwrap();
        let stored = spec.stored(" key ");
        assert_eq!(stored, json!(crate::obfuscate::obfuscate("key")));
        let config = Map::from_iter([("api_key".to_owned(), stored)]);
        assert_eq!(spec.value(&config), "key");
        // Plaintext keys from very old configs still load.
        let config = Map::from_iter([("api_key".to_owned(), json!("plain"))]);
        assert_eq!(spec.value(&config), "plain");
    }

    #[test]
    fn defaults_and_speed_normalisation() {
        let settings = openai_compat::settings();
        let base = settings.iter().find(|s| s.key == "api_base").unwrap();
        assert_eq!(base.value(&Map::new()), "https://api.openai.com/v1");
        let speed = settings.iter().find(|s| s.key == "service_tier").unwrap();
        assert_eq!(speed.value(&Map::new()), "default");
        let config = Map::from_iter([("service_tier".to_owned(), json!("turbo"))]);
        assert_eq!(speed.value(&config), "default");
        let config = Map::from_iter([("service_tier".to_owned(), json!("priority"))]);
        assert_eq!(speed.value(&config), "priority");
    }

    #[test]
    fn error_detail_prefers_api_message() {
        let status = reqwest::StatusCode::TOO_MANY_REQUESTS;
        assert_eq!(error_detail(status, r#"{"error": {"message": "slow down"}}"#), "slow down (HTTP 429)");
        assert_eq!(error_detail(status, r#"{"error": "model not found"}"#), "model not found (HTTP 429)");
        assert_eq!(error_detail(status, ""), "HTTP 429 Too Many Requests");
    }
}
