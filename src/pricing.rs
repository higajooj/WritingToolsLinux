//! List prices per token, from OpenRouter's public model list, to estimate
//! the cost of requests to providers that report only token counts.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::OnceCell;

use crate::providers::http;
use crate::runtime;

const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a finished response waits for the list before going unpriced.
const ESTIMATE_TIMEOUT: Duration = Duration::from_secs(3);

/// Dollars per (input, output) token, by lowercase model ID.
type Prices = HashMap<String, (f64, f64)>;

pub fn parse_prices(body: &Value) -> Prices {
    let price = |model: &Value, key: &str| {
        let price: f64 = model.pointer(&format!("/pricing/{key}"))?.as_str()?.parse().ok()?;
        // Routers such as openrouter/auto list -1: the price varies.
        (price >= 0.0).then_some(price)
    };
    let models = body.get("data").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    models
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?.to_lowercase();
            Some((id, (price(model, "prompt")?, price(model, "completion")?)))
        })
        .collect()
}

/// The list, downloaded once per run. A failed download is tried again by
/// the next request.
async fn prices() -> Option<&'static Prices> {
    static PRICES: OnceCell<Prices> = OnceCell::const_new();
    let fetch = || async {
        let response = http().get(MODELS_URL).timeout(FETCH_TIMEOUT).send().await.ok()?.error_for_status().ok()?;
        let prices = parse_prices(&response.json().await.ok()?);
        (!prices.is_empty()).then_some(prices)
    };
    PRICES.get_or_try_init(|| async { fetch().await.ok_or(()) }).await.ok()
}

/// Start downloading the list, so it is ready when the response arrives.
pub fn prefetch() {
    runtime::spawn(async {
        prices().await;
    });
}

/// The cost in dollars of a request to `model`, an OpenRouter model ID, or
/// `None` when the model is not listed or the list is unavailable.
pub async fn estimate(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let prices = tokio::time::timeout(ESTIMATE_TIMEOUT, prices()).await.ok()??;
    let (input, output) = prices.get(&model.to_lowercase())?;
    Some(input * input_tokens as f64 + output * output_tokens as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_prices_per_token() {
        let body = json!({"data": [
            {"id": "OpenAI/gpt-4o-mini", "pricing": {"prompt": "0.00000015", "completion": "0.0000006"}},
            {"id": "openrouter/auto", "pricing": {"prompt": "-1", "completion": "-1"}},
            {"id": "broken", "pricing": {"prompt": "free"}},
        ]});
        let prices = parse_prices(&body);
        assert_eq!(prices.len(), 1);
        assert_eq!(prices["openai/gpt-4o-mini"], (0.00000015, 0.0000006));
        assert!(parse_prices(&json!({"error": "down"})).is_empty());
    }
}
