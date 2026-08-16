use std::fmt;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;

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

#[derive(serde::Deserialize)]
struct ManPageArgs {
    topic: String,
    section: Option<String>,
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

pub async fn lookup_man_page(
    topic: &str,
    section: Option<&str>,
    config: &ManLookupConfig,
) -> Result<ManPageResult, ManError> {
    validate_topic(topic)?;

    let mut cmd = tokio::process::Command::new("man");
    cmd.arg("-P").arg("cat");

    if let Some(section) = section {
        validate_section(section)?;
        cmd.arg("-s").arg(section);
    }
    cmd.arg("--");
    cmd.arg(topic);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| ManError::SpawnError {
        message: e.to_string(),
    })?;
    let mut out_pipe = child.stdout.take().unwrap();
    let mut err_pipe = child.stderr.take().unwrap();

    let (status, std_out, stderr) = match tokio::time::timeout(config.timeout, async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (status, _, _) = tokio::try_join!(
            child.wait(),
            out_pipe.read_to_end(&mut stdout),
            err_pipe.read_to_end(&mut stderr),
        )?;
        Ok::<_, std::io::Error>((status, stdout, stderr))
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return Err(ManError::SpawnError {
                message: e.to_string(),
            });
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            return Err(ManError::Timeout);
        }
    };

    match status.code() {
        Some(0) => {
            let (content, truncated) = if std_out.len() > config.max_output_bytes {
                let mut truncated_content = std_out[..config.max_output_bytes].to_vec();
                truncated_content.extend_from_slice(b"[\n... truncated ...\n]");
                (truncated_content, true)
            } else {
                (std_out, false)
            };

            return Ok(ManPageResult {
                content: String::from_utf8(content).expect("not valid utf8"),
                truncated,
            });
        }

        Some(16) => Err(ManError::NotFound),
        other => {
            let stderr_str = String::from_utf8(stderr).expect("not valid utf8");

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

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let parsed_args: ManPageArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    let topic = parsed_args.topic.as_str();
    let section = parsed_args.section.as_deref();

    let man_page_res = lookup_man_page(topic, section, &ManLookupConfig::default()).await;

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

    #[tokio::test]
    async fn ls_is_ok() {
        assert!(
            !lookup_man_page("ls", None, &ManLookupConfig::default())
                .await
                .unwrap()
                .content
                .is_empty()
        );
    }

    #[tokio::test]
    async fn ls_with_section() {
        assert!(
            !lookup_man_page("ls", Some("1"), &ManLookupConfig::default())
                .await
                .unwrap()
                .content
                .is_empty()
        );
    }

    #[tokio::test]
    async fn nonexistent_is_err() {
        let err = lookup_man_page("nonexistent_topic_xyz", None, &ManLookupConfig::default())
            .await
            .unwrap_err();

        assert!(matches!(err, ManError::NotFound))
    }

    #[tokio::test]
    async fn lookup_times_out() {
        let config = ManLookupConfig {
            timeout: Duration::from_millis(1),
            ..ManLookupConfig::default()
        };

        let err = lookup_man_page("ls", None, &config).await.unwrap_err();

        assert!(matches!(err, ManError::Timeout));
    }

    #[tokio::test]
    async fn large_page_is_ok() {
        // The `curl` page is ~260 KB on this system, well above the 64 KB OS
        // pipe buffer. A wait-then-read design would deadlock on it.
        let res = lookup_man_page("curl", None, &ManLookupConfig::default())
            .await
            .unwrap();

        assert!(res.truncated);
        assert!(!res.content.is_empty());
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
