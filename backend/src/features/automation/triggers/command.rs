//! Trigger command layer.

use super::*;

/// Spawns the substituted command through the platform shell and waits for
/// completion. Injectable so command execution is unit-testable without a
/// real shell (the production implementation is [`ShellExecutor`]).
pub trait CommandExecutor: Send + Sync {
    fn run(
        &self,
        command: &str,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<Output>> + Send + '_>>;
}

/// Production executor: unix `sh -c` / windows `cmd /C` (contract D4).
pub struct ShellExecutor;

impl CommandExecutor for ShellExecutor {
    fn run(
        &self,
        command: &str,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<Output>> + Send + '_>> {
        let command = command.to_string();
        Box::pin(async move {
            #[cfg(unix)]
            {
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .output()
                    .await
            }
            #[cfg(windows)]
            {
                tokio::process::Command::new("cmd")
                    .args(["/C"])
                    .arg(&command)
                    .output()
                    .await
            }
        })
    }
}

/// Runs one local command (placeholders substituted, fixed timeout, exit-code
/// check) and returns its stdout with a single trailing newline stripped,
/// zeroize-wrapped so the credential buffer is wiped when dropped. Errors
/// carry only the failure mode (timeout / spawn error / exit code) — never
/// the command output.
pub fn execute_command<'a>(
    executor: &'a dyn CommandExecutor,
    command: &str,
    placeholders: &CommandPlaceholders,
    timeout: Duration,
) -> Pin<Box<dyn Future<Output = Result<Zeroizing<String>, String>> + Send + 'a>> {
    let substituted = apply_command_placeholders(command, placeholders);
    Box::pin(async move {
        let output = tokio::time::timeout(timeout, executor.run(&substituted))
            .await
            .map_err(|_| {
                format!(
                    "local command timed out after {}s",
                    timeout.as_secs().max(1)
                )
            })?
            .map_err(|error| format!("local command could not be spawned: {error}"))?;
        if !output.status.success() {
            return Err(match output.status.code() {
                Some(code) => format!("local command exited with code {code}"),
                None => "local command was terminated by a signal".to_string(),
            });
        }
        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        // Strip exactly one trailing newline: `echo pw` emits "pw\n", a
        // password genuinely ending in "\n" would emit "pw\n\n" and keep it.
        if stdout.ends_with('\n') {
            stdout.pop();
            if stdout.ends_with('\r') {
                stdout.pop();
            }
        }
        if stdout.len() > MAX_COMMAND_OUTPUT_BYTES {
            return Err(format!(
                "local command produced more than {MAX_COMMAND_OUTPUT_BYTES} bytes of output"
            ));
        }
        Ok(Zeroizing::new(stdout))
    })
}

/// Convenience wrapper used by the SSH auth/trigger paths: production shell
/// executor plus the fixed 10s timeout (contract D4).
pub fn run_credential_command<'a>(
    command: &'a str,
    placeholders: &'a CommandPlaceholders,
) -> Pin<Box<dyn Future<Output = Result<Zeroizing<String>, String>> + Send + 'a>> {
    execute_command(&ShellExecutor, command, placeholders, COMMAND_TIMEOUT)
}
