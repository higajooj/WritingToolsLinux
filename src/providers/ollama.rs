//! A local or remote Ollama server.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Prompt, Provider, ProviderError, SettingSpec, error_detail, http};

pub fn settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec::text("api_base", "API Base URL", "http://localhost:11434", "E.g. http://localhost:11434"),
        SettingSpec::text("api_model", "API Model", "llama3.1:8b", "E.g. llama3.1:8b"),
        SettingSpec::text("keep_alive", "Time to keep the model loaded in memory in minutes", "5", "E.g. 5"),
    ]
}

pub struct Ollama {
    pub api_base: String,
    pub model: String,
    pub keep_alive: String,
}

impl Ollama {
    pub fn request_body(&self, system_instruction: &str, prompt: &Prompt) -> Value {
        let mut messages = vec![json!({"role": "system", "content": system_instruction})];
        messages.extend(prompt.messages().into_iter().map(|m| json!(m)));
        let mut body = json!({"model": self.model, "messages": messages, "stream": false});
        let keep_alive = self.keep_alive.trim();
        if !keep_alive.is_empty() {
            // The setting is in minutes; anything else is passed through as an
            // Ollama duration (e.g. "1h" or "-1").
            body["keep_alive"] =
                if keep_alive.parse::<f64>().is_ok() { json!(format!("{keep_alive}m")) } else { json!(keep_alive) };
        }
        body
    }
}

#[async_trait]
impl Provider for Ollama {
    async fn respond(&self, system_instruction: &str, prompt: Prompt) -> Result<String, ProviderError> {
        let error = |message: String| {
            ProviderError::new("Ollama Error", format!("An error occurred during Ollama chat: {message}"))
        };
        let url = format!("{}/api/chat", self.api_base.trim().trim_end_matches('/'));
        let response = http()
            .post(url)
            .json(&self.request_body(system_instruction, &prompt))
            .send()
            .await
            .map_err(|e| error(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| error(e.to_string()))?;
        if !status.is_success() {
            return Err(error(error_detail(status, &text)));
        }
        let body: Value = serde_json::from_str(&text).map_err(|e| error(format!("Invalid response: {e}")))?;
        body.pointer("/message/content")
            .and_then(Value::as_str)
            .map(|content| content.trim().to_owned())
            .ok_or_else(|| error("The server returned no message.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_alive_in_minutes() {
        let mut provider = Ollama { api_base: "http://h".into(), model: "m".into(), keep_alive: "5".into() };
        let prompt = Prompt::Single("hi".into());
        let body = provider.request_body("sys", &prompt);
        assert_eq!(body["keep_alive"], "5m");
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "sys"}));
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "hi"}));
        provider.keep_alive = "-1".into();
        assert_eq!(provider.request_body("s", &prompt)["keep_alive"], "-1m");
        provider.keep_alive = "1h".into();
        assert_eq!(provider.request_body("s", &prompt)["keep_alive"], "1h");
        provider.keep_alive = " ".into();
        assert!(provider.request_body("s", &prompt).get("keep_alive").is_none());
    }
}
