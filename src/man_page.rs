use std::fmt;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::mcp;

#[derive(Clone, Copy, Debug)]
pub struct ManLookupConfig {
    pub max_output_bytes: usize,
    pub timeout: Duration,
}

impl Default for ManLookupConfig {
    fn default() -> Self {
        Self {
            max_output_bytes: 8192,
            timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ManPageResult {
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum ManError {
    InvalidInput {
        message: String,
    },
    NotFound,
    SpawnError {
        message: String,
    },
    SubprocessError {
        exit_code: Option<i32>,
        stderr: String,
    },
    Timeout,
}

impl fmt::Display for ManError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManError::InvalidInput { message } => write!(f, "invalid input: {}", message),
            ManError::NotFound => write!(f, "page not found"),
            ManError::SpawnError { message } => write!(f, "failed to spawn man: {}", message),
            ManError::SubprocessError { exit_code, stderr } => match exit_code {
                Some(code) => write!(f, "subprocess failed (exit {}): {}", code, stderr),
                None => write!(f, "subprocess failed: {}", stderr),
            },
            ManError::Timeout => write!(f, "subprocess timed out"),
        }
    }
}

fn validate_topic(topic: &str) -> Result<(), ManError> {
    let bytes = topic.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(ManError::InvalidInput {
            message: "topic must be 1-64 characters".into(),
        });
    }
    if !bytes[0].is_ascii_alphabetic() {
        return Err(ManError::InvalidInput {
            message: "topic must start with a letter".into(),
        });
    }
    if !bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(ManError::InvalidInput {
            message: "topic contains invalid characters".into(),
        });
    }
    Ok(())
}

fn validate_section(section: &str) -> Result<(), ManError> {
    let bytes = section.as_bytes();
    if bytes.is_empty() || bytes.len() > 1 {
        return Err(ManError::InvalidInput {
            message: "section must be one ASCII character".into(),
        });
    }

    let c = section.chars().next().unwrap().to_ascii_lowercase();

    if !matches!(c, '1'..='8' | 'n' | 'p' | 'l') {
        return Err(ManError::InvalidInput {
            message: "section must be 1-8, n, p, or l".into(),
        });
    }

    Ok(())
}

pub fn lookup_man_page(
    topic: &str,
    section: Option<&str>,
    config: &ManLookupConfig,
) -> Result<ManPageResult, ManError> {
    validate_topic(topic)?;

    let mut cmd = Command::new("man");
    cmd.arg("-P").arg("cat");

    if let Some(section) = section {
        validate_section(section)?;
        cmd.arg("-s").arg(section);
    }
    cmd.arg("--");
    cmd.arg(topic);

    let output = cmd.output().map_err(|e| ManError::SpawnError {
        message: e.to_string(),
    })?;

    match output.status.code() {
        Some(0) => {
            let (content, truncated) = if output.stdout.len() > config.max_output_bytes {
                let mut truncated_content = output.stdout[..config.max_output_bytes].to_vec();
                truncated_content.extend_from_slice(b"[\n... truncated ...\n]");
                (truncated_content, true)
            } else {
                (output.stdout, false)
            };

            return Ok(ManPageResult {
                content: String::from_utf8(content).expect("not valid utf8"),
                truncated,
            });
        }
        Some(16) => Err(ManError::NotFound),
        other => {
            let stderr_str = String::from_utf8(output.stderr).expect("not valid utf8");

            return Err(ManError::SubprocessError {
                exit_code: other,
                stderr: stderr_str,
            });
        }
    }
}

// --- MCP adapter ---

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "man_page",
        "description": "Look up a man page",
        "inputSchema": {
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "topic (e.g. 'ls')"},
                "section": { "type": "string", "description": "section (1-8, n, l or p)"}
            },
            "required": ["topic"]
        }
    })
}

pub fn handle_call(
    args: Value,
) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let topic = args["topic"].as_str().unwrap_or("");
    let section = args["section"].as_str();

    let man_page_res =
        lookup_man_page(topic, section, &ManLookupConfig::default());

    match man_page_res {
        Ok(res) => Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": res.content },
            ],
            "isError": false,
            "truncated": res.truncated,
        })),
        Err(err) => Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": err.to_string() },
            ],
            "isError": true,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_is_ok() {
        assert!(
            !lookup_man_page("ls", None, &ManLookupConfig::default())
                .unwrap()
                .content
                .is_empty()
        );
    }

    #[test]
    fn ls_with_section() {
        assert!(
            !lookup_man_page("ls", Some("1"), &ManLookupConfig::default())
                .unwrap()
                .content
                .is_empty()
        );
    }

    #[test]
    fn nonexistent_is_err() {
        let err = lookup_man_page("nonexistent_topic_xyz", None, &ManLookupConfig::default())
            .unwrap_err();

        assert!(matches!(err, ManError::NotFound))
    }

    #[test]
    fn validate_topic_valid() {
        assert!(validate_topic("ls").is_ok());
        assert!(validate_topic("systemctl").is_ok());
        assert!(validate_topic("foo-bar_baz.qux").is_ok());
    }

    #[test]
    fn validate_topic_empty() {
        assert!(matches!(
            validate_topic(""),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_topic_starts_with_digit() {
        assert!(matches!(
            validate_topic("1abc"),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_topic_invalid_char() {
        assert!(matches!(
            validate_topic("ls!"),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_topic_too_long() {
        let topic = "a".repeat(65);
        assert!(matches!(
            validate_topic(&topic),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_section_valid() {
        assert!(validate_section("1").is_ok());
        assert!(validate_section("8").is_ok());
        assert!(validate_section("n").is_ok());
        assert!(validate_section("N").is_ok());
        assert!(validate_section("p").is_ok());
        assert!(validate_section("l").is_ok());
    }

    #[test]
    fn validate_section_empty() {
        assert!(matches!(
            validate_section(""),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_section_too_long() {
        assert!(matches!(
            validate_section("12"),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn validate_section_invalid() {
        assert!(matches!(
            validate_section("9"),
            Err(ManError::InvalidInput { .. })
        ));
    }

    #[test]
    fn test_man_error_display() {
        assert_eq!(ManError::NotFound.to_string(), "page not found");
        assert_eq!(
            ManError::SubprocessError {
                exit_code: Some(1),
                stderr: String::from("error message")
            }
            .to_string(),
            "subprocess failed (exit 1): error message"
        );
        assert_eq!(
            ManError::SubprocessError {
                exit_code: None,
                stderr: String::from("error message")
            }
            .to_string(),
            "subprocess failed: error message"
        );
    }
}
