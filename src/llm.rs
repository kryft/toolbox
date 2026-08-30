use reqwest;
use serde;
use std::time::Duration;

pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
    /// Sampling temperature (0.0 = deterministic).
    pub temperature: f32,
    /// None → thinking explicitly off via `chat_template_kwargs` (the
    /// server thinks by default); Some(effort) → send `reasoning_effort`
    /// and let it own thinking.
    pub reasoning_effort: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: std::env::var("LLAMA_URL")
                .unwrap_or_else(|_| String::from("http://172.17.0.1:8081/v1")),
            model: std::env::var("LLAMA_MODEL")
                .unwrap_or_else(|_| String::from("qwen3.8-27b-q4xl")),
            timeout: Duration::from_secs(60),
            temperature: std::env::var("LLAMA_TEMPERATURE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            reasoning_effort: std::env::var("LLAMA_REASONING_EFFORT").ok(),
        }
    }
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(serde::Deserialize)]
struct Choice {
    message: Message,
}

#[derive(serde::Deserialize)]
struct Message {
    content: Option<String>,
}

/// Builds the chat-completion request body. `reasoning_effort` and
/// `chat_template_kwargs` are mutually exclusive: None pins thinking off,
/// Some(e) hands control to the knob.
fn chat_request(
    cfg: &LlmConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user},
        ],
        "temperature": cfg.temperature,
        "max_tokens": max_tokens,
    });
    match &cfg.reasoning_effort {
        Some(effort) => body["reasoning_effort"] = serde_json::json!(effort),
        None => body["chat_template_kwargs"] =
            serde_json::json!({ "enable_thinking": false }),
    }
    body
}

pub async fn chat(
    cfg: &LlmConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let body = chat_request(cfg, system, user, max_tokens);

    let timeout = cfg.timeout + Duration::from_millis(u64::from(max_tokens) * 10);

    match tokio::time::timeout(timeout, async {
        let resp = client
            .post(&format!("{}/chat/completions", cfg.base_url))
            .json(&body)
            .send()
            .await?;

        let resp = resp.error_for_status()?;
        let data: ChatResponse = resp.json().await?;

        Ok::<_, reqwest::Error>(data)
    })
    .await
    .map_err(|_| "llm call timed out".to_string())?
    {
        Ok(data) => {
            let choice = data
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| "llm returned no choices".to_string())?;

            choice
                .message
                .content
                .ok_or_else(|| "llm returned no content".to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn extract_json(text: &str) -> Result<serde_json::Value, String> {
    let candidate = match (text.find("```"), text.rfind("```")) {
        (Some(open), Some(close)) if close > open => &text[open + 3..close],
        _ => text,
    };

    let Some(start) = candidate.find('{') else {
        return Err("no JSON object found".into());
    };
    let Some(end) = candidate.rfind('}') else {
        return Err("no closing brace found".into());
    };
    serde_json::from_str(&candidate[start..end + 1]).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_json_parses() {
        let v = extract_json(r#"{"a": 1}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn nested_object_is_captured_whole() {
        let v = extract_json(r#"{"a": {"b": 2}}"#).unwrap();
        assert_eq!(v["a"]["b"], 2);
    }

    #[test]
    fn fenced_json_with_language_tag() {
        let text = r#"```json
{"regions": [], "continuation_note": "n/a"}
```"#;
        let v = extract_json(text).unwrap();
        assert!(v["regions"].is_array());
        assert_eq!(v["continuation_note"], "n/a");
    }

    #[test]
    fn fenced_json_without_language_tag() {
        let text = r#"```
{"a": 1}
```"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn prose_around_json_is_ignored() {
        let text = "Here you go:\n  {\"a\": 1}\nLet me know if you need more.";
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn fenced_prose_then_json() {
        let text = r#"```
Sure, here it is:
{"a": 1}
```"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn unclosed_single_fence_falls_back_to_whole_text() {
        let text = r#"```json
{"a": 1}"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn no_braces_is_error() {
        let err = extract_json("no json here").unwrap_err();
        assert!(err.contains("no JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn empty_input_is_error() {
        let err = extract_json("").unwrap_err();
        assert!(err.contains("no JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn opening_without_closing_brace_is_error() {
        let err = extract_json(r#"{"a": 1"#).unwrap_err();
        assert!(err.contains("no closing brace"), "unexpected error: {err}");
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let err = extract_json(r#"{"a":}"#).unwrap_err();
        // Braces were found; the failure must come from serde, not the
        // "no JSON object" path.
        assert!(!err.contains("no JSON object"), "unexpected error: {err}");
    }

    fn cfg_with(reasoning_effort: Option<&str>) -> LlmConfig {
        LlmConfig {
            base_url: "http://mock".to_string(),
            model: "mock".to_string(),
            timeout: Duration::from_secs(5),
            temperature: 0.0,
            reasoning_effort: reasoning_effort.map(String::from),
        }
    }

    #[test]
    fn request_body_pins_thinking_off_when_no_reasoning_effort() {
        let body = chat_request(&cfg_with(None), "sys", "usr", 100);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["temperature"], serde_json::json!(0.0));
    }

    #[test]
    fn request_body_reasoning_effort_replaces_template_kwargs() {
        let body = chat_request(&cfg_with(Some("low")), "sys", "usr", 100);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("chat_template_kwargs").is_none());
    }
}
