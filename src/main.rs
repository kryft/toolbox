use std::fmt;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
struct ManLookupConfig {
    max_output_bytes: usize,
    timeout: Duration,
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
struct ManPageResult {
    content: String,
    truncated: bool,
}

#[derive(Debug)]
enum ManError {
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
            ManError::NotFound => write!(f, "page not found"),
            ManError::SpawnError { message } => write!(f, "failed to spawn man: {}", message),
            ManError::SubprocessError { exit_code, stderr } => {
                match exit_code {
                    Some(code) => write!(f, "subprocess failed (exit {}): {}", code, stderr),
                    None => write!(f, "subprocess failed: {}", stderr),
                }
            }
            ManError::Timeout => write!(f, "subprocess timed out")
        }
    }
}

fn lookup_man_page(
    topic: &str,
    section: Option<&str>,
    config: &ManLookupConfig,
) -> Result<ManPageResult, ManError> {
    let mut cmd = Command::new("man");
    cmd.arg("-P").arg("cat");

    if let Some(section) = section {
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
                truncated: truncated,
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

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use crate::ManLookupConfig;

    use super::lookup_man_page;
    use super::ManError;

    #[test]
    fn ls_is_ok() {
        assert!(!lookup_man_page("ls", None, &ManLookupConfig::default())
            .unwrap()
            .content
            .is_empty());
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
        let err =
            lookup_man_page("nonexistent_topic_xyz", None, &ManLookupConfig::default()).unwrap_err();

        assert!(matches!(err, ManError::NotFound))
    }

    #[test]
    fn test_man_error_display() {
        assert_eq!(ManError::NotFound.to_string(), "page not found");
        assert_eq!(ManError::SubprocessError { exit_code: Some(1), stderr: String::from("error message")}.to_string(), "subprocess failed (exit 1): error message");
        assert_eq!(ManError::SubprocessError { exit_code: None, stderr: String::from("error message")}.to_string(), "subprocess failed: error message");
    }
}
