use reqwest;
use serde;
use std::time::Duration;

pub struct LlmConfig {
    base_url: String,
    model: String,
    timeout: Duration,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: std::env::var("LLAMA_URL")
                .unwrap_or_else(|_| String::from("http://172.17.0.1:8081/v1")),
            model: std::env::var("LLAMA_MODEL")
                .unwrap_or_else(|_| String::from("qwen3.8-27b-q4xl")),
            timeout: Duration::from_secs(60),
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

pub async fn chat(
    cfg: &LlmConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user},
        ],
        "temperature": 0,
        "max_tokens": max_tokens,
        "chat_template_kwargs": { "enable_thinking": false }
    });

    match tokio::time::timeout(cfg.timeout, async {
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
}
