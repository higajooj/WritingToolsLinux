//! Any OpenAI-compatible `/chat/completions` server.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Prompt, Provider, ProviderError, SPEED_OPTIONS, SettingKind, SettingSpec, error_detail, http};

pub fn settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec {
            key: "api_key",
            label: "API Key",
            default: "",
            placeholder: "Leave blank if your server does not require authentication.",
            kind: SettingKind::Text,
        },
        SettingSpec::text("api_base", "API Base URL", "https://api.openai.com/v1", "E.g. https://api.openai.com/v1"),
        SettingSpec::text("api_organisation", "API Organisation", "", "Leave blank if not applicable."),
        SettingSpec::text("api_project", "API Project", "", "Leave blank if not applicable."),
        SettingSpec::text("api_model", "API Model", "gpt-4o-mini", "E.g. gpt-4o-mini"),
        SettingSpec {
            key: "service_tier",
            label: "Speed",
            default: "default",
            placeholder: "",
            kind: SettingKind::Speed {
                description: "Speed selection is available for the official OpenAI API. Fast costs more and depends on model and account availability.",
            },
        },
    ]
}

/// Speed options exist only for the official OpenAI API.
pub fn is_openai_api(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.openai.com")
        && url.port().is_none()
        && matches!(url.path(), "/v1" | "/v1/")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

pub struct OpenAiCompatible {
    pub api_key: String,
    pub api_base: String,
    pub organisation: String,
    pub project: String,
    pub model: String,
    pub service_tier: String,
}

impl OpenAiCompatible {
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.api_base.trim().trim_end_matches('/'))
    }

    pub fn request_body(&self, system_instruction: &str, prompt: &Prompt) -> Value {
        let mut messages = vec![json!({"role": "system", "content": system_instruction})];
        messages.extend(prompt.messages().into_iter().map(|m| json!(m)));
        let mut body = json!({"model": self.model, "messages": messages, "stream": false});
        // Standard sends nothing, so the tier configured in the OpenAI Project
        // still applies; only Fast is requested explicitly.
        if self.service_tier == SPEED_OPTIONS[1].1 && is_openai_api(&self.api_base) {
            body["service_tier"] = json!("priority");
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    async fn respond(&self, system_instruction: &str, prompt: Prompt) -> Result<String, ProviderError> {
        let error = |message: String| {
            let lower = message.to_lowercase();
            if lower.contains("exceeded") || lower.contains("rate limit") {
                ProviderError::new(
                    "Rate Limit Hit",
                    "It appears you have hit an API rate/usage limit. Please try again later or adjust your settings.",
                )
            } else {
                ProviderError::new("Error", format!("An error occurred: {message}"))
            }
        };
        let mut request = http().post(self.endpoint()).json(&self.request_body(system_instruction, &prompt));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        if !self.organisation.trim().is_empty() {
            request = request.header("OpenAI-Organization", self.organisation.trim());
        }
        if !self.project.trim().is_empty() {
            request = request.header("OpenAI-Project", self.project.trim());
        }
        let response = request.send().await.map_err(|e| error(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| error(e.to_string()))?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(error(format!("rate limit: {}", error_detail(status, &text))));
        }
        if !status.is_success() {
            return Err(error(error_detail(status, &text)));
        }
        let body: Value = serde_json::from_str(&text).map_err(|e| error(format!("Invalid response: {e}")))?;
        body.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|content| content.trim().to_owned())
            .ok_or_else(|| error("The server returned no message.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Message;

    fn provider(base: &str, tier: &str) -> OpenAiCompatible {
        OpenAiCompatible {
            api_key: String::new(),
            api_base: base.into(),
            organisation: String::new(),
            project: String::new(),
            model: "m".into(),
            service_tier: tier.into(),
        }
    }

    #[test]
    fn recognises_only_the_official_api() {
        for ok in ["https://api.openai.com/v1", "https://api.openai.com/v1/", " https://api.openai.com:443/v1 "] {
            assert!(is_openai_api(ok), "{ok}");
        }
        for bad in [
            "http://api.openai.com/v1",
            "https://api.openai.com/v2",
            "https://api.openai.com:8443/v1",
            "https://user:pw@api.openai.com/v1",
            "https://api.openai.com/v1?x=1",
            "https://api.openai.com/v1#f",
            "https://api.openai.com.evil/v1",
            "http://localhost:11434/v1",
            "",
        ] {
            assert!(!is_openai_api(bad), "{bad}");
        }
    }

    #[test]
    fn priority_only_for_official_api() {
        let prompt = Prompt::Single("hi".into());
        assert_eq!(
            provider("https://api.openai.com/v1", "priority").request_body("s", &prompt)["service_tier"],
            "priority"
        );
        assert!(
            provider("https://api.openai.com/v1", "default").request_body("s", &prompt).get("service_tier").is_none()
        );
        assert!(
            provider("http://localhost:8080/v1", "priority").request_body("s", &prompt).get("service_tier").is_none()
        );
    }

    #[test]
    fn messages_and_endpoint() {
        let p = provider("http://localhost:11434/v1/", "default");
        assert_eq!(p.endpoint(), "http://localhost:11434/v1/chat/completions");
        let body = p.request_body("sys", &Prompt::Chat(vec![Message::user("q"), Message::assistant("a")]));
        assert_eq!(
            body["messages"],
            json!([{"role": "system", "content": "sys"}, {"role": "user", "content": "q"}, {"role": "assistant", "content": "a"}])
        );
        assert_eq!(body["stream"], false);
    }
}
