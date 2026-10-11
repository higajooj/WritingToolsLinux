//! OpenRouter: one key for many models, over its OpenAI-compatible API.

use serde_json::{Map, Value};

use super::openai_compat::OpenAiCompatible;
use super::{SPEED_OPTIONS, SettingKind, SettingSpec};

pub const API_BASE: &str = "https://openrouter.ai/api/v1";

pub fn settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec { key: "api_key", label: "API Key", default: "", placeholder: "", kind: SettingKind::Secret },
        SettingSpec::text(
            "api_model",
            "Default Model",
            "openai/gpt-4o-mini",
            "A model ID from openrouter.ai/models, e.g. openai/gpt-4o-mini",
        ),
    ]
}

pub fn build(config: &Map<String, Value>) -> OpenAiCompatible {
    let get = |key: &str| {
        settings().into_iter().find(|spec| spec.key == key).map(|spec| spec.value(config)).unwrap_or_default()
    };
    OpenAiCompatible {
        api_key: get("api_key").trim().to_owned(),
        api_base: API_BASE.to_owned(),
        organisation: String::new(),
        project: String::new(),
        model: get("api_model").trim().to_owned(),
        service_tier: SPEED_OPTIONS[0].1.to_owned(),
    }
}
