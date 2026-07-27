use std::process::Command;

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
    Timeout
}

fn lookup_man_page(topic: &str, section: Option<&str>) -> Result<String, ManError> {
    let mut cmd = Command::new("man");

    if let Some(section) = section {
        cmd.arg(section);
    }
    cmd.arg(topic);

    let output = cmd.output().map_err(|e| ManError::SpawnError { message: e.to_string() })?;

    let stderr_str = String::from_utf8(output.stderr).expect("not valid utf8");
    if stderr_str.contains("No manual entry") {
        return Err(ManError::NotFound);
    }

    if output.status.success() {
        Ok(String::from_utf8(output.stdout).expect("not valid utf8"))
    } else {
        Err(ManError::SubprocessError {
            exit_code: output.status.code(),
            stderr: stderr_str,
        })
    }
}

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use super::lookup_man_page;
    use super::ManError;

    #[test]
    fn ls_is_ok() {
        assert!(!lookup_man_page("ls", None).unwrap().is_empty());
    }

    #[test]
    fn nonexistent_is_err() {
        let err = lookup_man_page("nonexistent_topic_xyz", None).unwrap_err();

        assert!(matches!(err, ManError::NotFound))
    }
}
