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
                .unwrap_or_else(|_| String::from("qwen3.8-27b")),
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

/// Builds the chat-completion request body. The resolved
/// `reasoning_effort` (a per-call override if one was given, else the
/// config's knob) and `chat_template_kwargs` are mutually exclusive:
/// None pins thinking off, Some(e) hands control to the knob.
fn chat_request(
    cfg: &LlmConfig,
    reasoning_effort: &Option<String>,
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
    match reasoning_effort {
        Some(effort) => body["reasoning_effort"] = serde_json::json!(effort),
        None => body["chat_template_kwargs"] =
            serde_json::json!({ "enable_thinking": false }),
    }
    body
}

/// One-shot chat completion (returns the assistant's content); thinking
/// follows the config's `reasoning_effort` knob (None = off).
pub async fn chat(
    cfg: &LlmConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    chat_reasoning(cfg, None, system, user, max_tokens).await
}

/// `chat` with a per-call thinking override: `Some(e)` sends
/// `reasoning_effort: e` for this call only — the config's (global env)
/// knob is ignored, so one call can think without flipping every other
/// caller (triage's calibration corpus included); `None` falls through
/// to the config. Thinking tokens count against `max_tokens`.
pub async fn chat_reasoning(
    cfg: &LlmConfig,
    reasoning_effort: Option<&str>,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let effective: Option<String> = reasoning_effort
        .map(String::from)
        .or_else(|| cfg.reasoning_effort.clone());
    let client = reqwest::Client::new();

    let body = chat_request(cfg, &effective, system, user, max_tokens);

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

/// Extracts the first complete JSON object from model output.
/// The object is located by a string/escape-aware brace-depth scan, so
/// it works whether the model wrote a bare object, prose before it
/// (preambles observed live), or extra text after it — a duplicated
/// object after the first would break a "first { .. last }" slice,
/// whose serde parse fails with "trailing characters" (observed live:
/// KJV no-query run, 2026-09-26).
pub fn extract_json(text: &str) -> Result<serde_json::Value, String> {
    let candidate = match (text.find("```"), text.rfind("```")) {
        (Some(open), Some(close)) if close > open => &text[open + 3..close],
        _ => text,
    };
    let slice = first_complete_object(candidate).ok_or("no JSON object found")?;
    serde_json::from_str(slice).map_err(|e| e.to_string())
}

/// The span of the first brace-balanced top-level object: from the
/// first `{` at depth 0 to the `}` that returns the depth to 0. Braces
/// inside JSON strings (and escaped characters) do not count; a stray
/// `}` at depth 0 is ignored. None if no balanced span exists (e.g. a
/// truncated object).
fn first_complete_object(candidate: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in candidate.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        let s = start?;
                        return Some(&candidate[s..i + ch.len_utf8()]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Serves canned chat-completion responses, one per connection.
/// `chat()` builds a fresh reqwest::Client per call, so each call is
/// one TCP connection; the mock loops until the bodies run out.
#[cfg(test)]
pub(crate) fn start_mock_llm(bodies: Vec<String>) -> String {
    use std::io::{BufRead, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("failed to bind mock llm");
    let url = format!("http://{}", listener.local_addr().unwrap());

    std::thread::spawn(move || {
        for body in bodies {
            let Ok((mut socket, _)) = listener.accept() else { break };
            let mut reader = std::io::BufReader::new(&socket);
            // Read request headers until the blank line; the small
            // test bodies ride along in the same buffer fills.
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
            }
            let content = serde_json::json!({
                "choices": [{ "message": { "content": body } }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                content.len(),
                content
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });

    url
}

#[cfg(test)]
pub(crate) fn mock_config(url: &str) -> LlmConfig {
    LlmConfig {
        base_url: url.to_string(),
        model: "mock".to_string(),
        timeout: std::time::Duration::from_secs(5),
        temperature: 0.0,
        reasoning_effort: None,
    }
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
    fn unclosed_object_is_error() {
        // A truncated object has no balanced span -> the extraction
        // error (the caller names the chunk), not a serde error.
        let err = extract_json(r#"{"a": 1"#).unwrap_err();
        assert!(err.contains("no JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let err = extract_json(r#"{"a":}"#).unwrap_err();
        // Braces were found; the failure must come from serde, not the
        // "no JSON object" path.
        assert!(!err.contains("no JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn object_then_duplicate_object_parses_the_first() {
        // The live failure (2026-09-26 KJV no-query run, map call at
        // bytes 1147022..1409188): a complete object followed by more
        // text containing braces. A first-{..last-} slice spans both
        // and serde rejects the trailing characters; the first
        // complete object is the answer.
        let text = r#"{"summary": "s", "pointers": []}
{"summary": "s again", "pointers": []}"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["summary"], "s");
    }

    #[test]
    fn object_then_trailing_prose_with_brace_parses() {
        let text = r#"{"a": 1} trailing prose with a brace } here"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn braces_inside_strings_do_not_end_the_object() {
        let text = r#"here you go: {"summary": "he said \"}\" and \"{\"", "n": 1}"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["summary"], "he said \"}\" and \"{\"");
        assert_eq!(v["n"], 1);
    }

    #[test]
    fn escaped_backslash_then_close_parses() {
        // The string ends with an escaped backslash (\\ in the JSON); the
        // next quote closes the string and the object closes after it.
        let text = r#"{"s": "ends with backslash \\"}"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["s"], "ends with backslash \\");
    }

    #[test]
    fn escaped_quote_swallows_the_closing_brace() {
        // The \" is an escaped quote (the backslash belongs to the
        // string), so the `}` is inside the string and the object never
        // closes: an extraction error, not a parse error.
        let text = r#"{"s": "ends with a \" quote}"#;
        let err = extract_json(text).unwrap_err();
        assert!(err.contains("no JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn stray_closing_brace_before_the_object_is_ignored() {
        let text = r#"garbage } then {"a": 1}"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"], 1);
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
        let cfg = cfg_with(None);
        let body = chat_request(&cfg, &cfg.reasoning_effort, "sys", "usr", 100);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["temperature"], serde_json::json!(0.0));
    }

    #[test]
    fn request_body_reasoning_effort_replaces_template_kwargs() {
        let cfg = cfg_with(Some("low"));
        let body = chat_request(&cfg, &cfg.reasoning_effort, "sys", "usr", 100);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn request_body_per_call_override_wins_over_config() {
        let cfg = cfg_with(Some("low"));
        // A per-call override wins over the config knob...
        let body = chat_request(&cfg, &Some("medium".to_string()), "sys", "usr", 100);
        assert_eq!(body["reasoning_effort"], "medium");
        assert!(body.get("chat_template_kwargs").is_none());
        // ...and a None override falls through to the config.
        let body = chat_request(&cfg, &cfg.reasoning_effort, "sys", "usr", 100);
        assert_eq!(body["reasoning_effort"], "low");
    }
}
