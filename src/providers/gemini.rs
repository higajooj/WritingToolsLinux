//! Google Gemini through the Generative Language REST API.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Prompt, Provider, ProviderError, Role, SettingKind, SettingSpec, error_detail, http};

pub const DEFAULT_MODEL: &str = "gemini-flash-latest";

const RATE_LIMIT_MESSAGE: &str = "Whoops! You've hit the per-minute rate limit of the Gemini API. Please try again in a few moments.\n\nIf this happens often, simply switch to a Gemini model with a higher usage limit in Settings.";

pub fn settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec {
            key: "api_key",
            label: "API Key",
            default: "",
            placeholder: "Paste your Gemini API key here",
            kind: SettingKind::Secret,
        },
        SettingSpec {
            key: "model_name",
            label: "Model",
            default: DEFAULT_MODEL,
            placeholder: "Select Gemini model to use",
            kind: SettingKind::Dropdown {
                options: vec![
                    // A Google-managed alias for the current Flash model: fast,
                    // but capped at 20 free requests/day.
                    ("⭐ Gemini Flash Latest (very fast | only 20 free uses/day)", DEFAULT_MODEL),
                    // Unlimited on the free tier but noticeably slower.
                    ("Gemma 4 31B (slow | unlimited free use)", "gemma-4-31b-it"),
                    ("Gemma 4 26B A4B (slow | unlimited free use)", "gemma-4-26b-a4b-it"),
                ],
                custom_placeholder: Some("e.g., gemini-3.1-pro-preview"),
            },
        },
    ]
}

pub struct Gemini {
    pub api_key: String,
    pub model: String,
}

impl Gemini {
    /// Request body. Temperature stays at the default (Gemini 3 docs advise
    /// against lowering it). Thinking is kept minimal on Gemini models, where
    /// it cannot be fully disabled; Gemma models think only when asked, and
    /// `thinkingConfig` is omitted for them.
    pub fn request_body(&self, system_instruction: &str, prompt: &Prompt) -> Value {
        let contents: Vec<Value> = prompt
            .messages()
            .into_iter()
            .map(|message| {
                let role = if message.role == Role::Assistant { "model" } else { "user" };
                json!({"role": role, "parts": [{"text": message.content}]})
            })
            .collect();
        let safety: Vec<Value> = [
            "HARM_CATEGORY_HARASSMENT",
            "HARM_CATEGORY_HATE_SPEECH",
            "HARM_CATEGORY_SEXUALLY_EXPLICIT",
            "HARM_CATEGORY_DANGEROUS_CONTENT",
        ]
        .into_iter()
        .map(|category| json!({"category": category, "threshold": "BLOCK_NONE"}))
        .collect();
        let mut generation = json!({"maxOutputTokens": 1000});
        if !self.model.to_lowercase().contains("gemma") {
            generation["thinkingConfig"] = json!({"thinkingLevel": "minimal"});
        }
        json!({
            "systemInstruction": {"parts": [{"text": system_instruction}]},
            "contents": contents,
            "safetySettings": safety,
            "generationConfig": generation,
        })
    }
}

/// Concatenate the answer's text parts, skipping thought summaries.
pub fn response_text(body: &Value) -> Option<String> {
    let parts = body.pointer("/candidates/0/content/parts")?.as_array()?;
    let text: String = parts
        .iter()
        .filter(|part| !part.get("thought").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    Some(text.trim_end_matches('\n').to_owned())
}

#[async_trait]
impl Provider for Gemini {
    async fn respond(&self, system_instruction: &str, prompt: Prompt) -> Result<String, ProviderError> {
        let error = |message: String| ProviderError::new("Gemini Error", message);
        if self.api_key.is_empty() {
            return Err(error("Add your Gemini API key in Settings.".into()));
        }
        let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", self.model);
        let response = http()
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&self.request_body(system_instruction, &prompt))
            .send()
            .await
            .map_err(|e| error(format!("Could not reach Gemini: {e}")))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| error(e.to_string()))?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || text.contains("RESOURCE_EXHAUSTED") {
            return Err(ProviderError::new("Error - Rate Limit Hit", RATE_LIMIT_MESSAGE));
        }
        if !status.is_success() {
            return Err(error(error_detail(status, &text)));
        }
        let body: Value = serde_json::from_str(&text).map_err(|e| error(format!("Invalid response: {e}")))?;
        match response_text(&body) {
            Some(text) => Ok(text),
            None => {
                let reason = body
                    .pointer("/promptFeedback/blockReason")
                    .or_else(|| body.pointer("/candidates/0/finishReason"))
                    .and_then(Value::as_str)
                    .unwrap_or("no content");
                Err(error(format!("Gemini returned no text ({reason}).")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Message;

    #[test]
    fn body_for_gemini_model() {
        let provider = Gemini { api_key: "k".into(), model: DEFAULT_MODEL.into() };
        let body = provider.request_body("sys", &Prompt::Single("hello".into()));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(body["contents"], json!([{"role": "user", "parts": [{"text": "hello"}]}]));
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 1000);
        assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"], "minimal");
        assert_eq!(body["safetySettings"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn body_for_gemma_chat() {
        let provider = Gemini { api_key: "k".into(), model: "gemma-4-31b-it".into() };
        let prompt = Prompt::Chat(vec![Message::user("q"), Message::assistant("a"), Message::user("q2")]);
        let body = provider.request_body("sys", &prompt);
        assert!(body["generationConfig"].get("thinkingConfig").is_none());
        let roles: Vec<&str> =
            body["contents"].as_array().unwrap().iter().map(|c| c["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "model", "user"]);
    }

    #[test]
    fn parses_text_without_thoughts() {
        let body = json!({"candidates": [{"content": {"parts": [
            {"text": "thinking", "thought": true}, {"text": "Hello "}, {"text": "world\n"}
        ]}}]});
        assert_eq!(response_text(&body).as_deref(), Some("Hello world"));
        assert_eq!(response_text(&json!({"promptFeedback": {"blockReason": "SAFETY"}})), None);
    }
}
