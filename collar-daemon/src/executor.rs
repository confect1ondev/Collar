//! Command execution engine.

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, error, instrument};

/// Result of script execution.
#[derive(Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ExecutionResult {
    /// Get combined output, preferring stdout.
    pub fn output(&self) -> &str {
        if !self.stdout.is_empty() {
            &self.stdout
        } else {
            &self.stderr
        }
    }
}

/// Execute a shell command.
#[instrument(skip_all, fields(command = %command))]
pub async fn execute(command: &str, args: Option<&[String]>) -> Result<ExecutionResult> {
    debug!("Executing command");

    let mut cmd = Command::new("sh");
    cmd.arg("-c");

    // Build full command with args if provided
    let full_command = if let Some(args) = args {
        format!("{} {}", command, shell_escape_args(args))
    } else {
        command.to_string()
    };

    cmd.arg(&full_command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd
        .output()
        .await
        .context("Failed to execute command")?;

    let result = ExecutionResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    };

    if result.success {
        debug!(stdout = %result.stdout, "Command succeeded");
    } else {
        error!(
            exit_code = ?result.exit_code,
            stderr = %result.stderr,
            "Command failed"
        );
    }

    Ok(result)
}

/// Escape arguments for shell.
fn shell_escape_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') || arg.contains('"') || arg.contains('\'') {
                format!("'{}'", arg.replace('\'', "'\\''"))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_simple() {
        let result = execute("echo hello", None).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, "hello");
    }

    #[tokio::test]
    async fn test_execute_with_args() {
        let args = vec!["world".to_string()];
        let result = execute("echo", Some(&args)).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, "world");
    }

    #[tokio::test]
    async fn test_execute_failure() {
        let result = execute("exit 1", None).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }
}
