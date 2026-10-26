//! Docker feature layer.

use super::*;

// —— Scripts ——————————————————————————————————————————————

/// Raw POSIX collector body (PATH hardening + probe + `<cli> ps`), shared
/// by the heredoc wrapper ([`list_script`]) and the Quick Sudo elevated
/// retry — the latter passes the body through `sanitize_sudo_command`
/// (`sudo -S -p '' sh -c '<escaped>'`) so sudo's stdin stays free for the
/// password while the script travels as argv. `engine.env_prefix()` bakes
/// the endpoint export in as part of the body text, so the elevated path
/// keeps it too (sudo's env_reset cannot strip what is in the script).
pub fn list_body(engine: &EngineConfig) -> String {
    format!(
        "{PATH_PREFIX}{export}\
         echo DBXDOCKER_PROBE\n\
         if command -v {cli} >/dev/null 2>&1; then echo docker-found; else echo docker-missing; fi\n\
         if {cli} info >/dev/null 2>&1; then echo daemon-ok; else echo daemon-denied; fi\n\
         echo DBXDOCKER_PS\n\
         ps_out=$({cli} ps -a --no-trunc --format '{PS_FORMAT}' 2>/dev/null)\n\
         if [ -n \"$ps_out\" ]; then printf '%s\\n' \"$ps_out\"; else\n\
           echo DBXDOCKER_PS_LEGACY\n\
           {cli} ps -a --no-trunc --format '{PS_FORMAT_LEGACY}' 2>/dev/null\n\
         fi\n",
        export = engine.env_prefix(),
        cli = engine.cli,
    )
}

/// Container list collector. The probe section distinguishes "CLI not
/// installed" (available: false) from "daemon socket denied" (available:
/// false plus needsSudo: true, the docker-group hint) so the panel can
/// explain *why* a host shows an empty list instead of a bare empty state.
///
/// The PATH line is load-bearing: non-interactive shells (sshd exec, macOS
/// GUI children) carry a skeletal PATH where Homebrew (`/opt/homebrew/bin`,
/// `/usr/local/bin`), Docker Desktop and OrbStack installs are invisible,
/// so a bare `command -v docker` reports "not installed" on healthy hosts.
pub fn list_script(engine: &EngineConfig) -> String {
    format!("sh -s <<'DBXDOCKER'\n{}DBXDOCKER\n", list_body(engine))
}

/// cmd.exe fallback collector for Windows OpenSSH hosts, where the default
/// shell has no `sh` and the heredoc above produces no markers at all. The
/// trigger is the missing `DBXDOCKER_PROBE` marker (POSIX hosts always emit
/// it, even on a docker-missing verdict), so the retry costs one extra
/// round-trip only on hosts that garbled the first collector. cmd emits
/// CRLF, which the parsers already trim field by field. A configured
/// endpoint travels as `set` lines (the charset gate keeps the values
/// cmd-inert).
pub fn win_list_script(engine: &EngineConfig) -> String {
    format!(
        "echo DBXDOCKER_PROBE\r\n{env}\
         where /q {cli} 2>nul && echo docker-found || echo docker-missing\r\n\
         {cli} info >nul 2>&1 && echo daemon-ok || echo daemon-denied\r\n\
         echo DBXDOCKER_PS\r\n\
         {cli} ps -a --no-trunc --format \"{PS_FORMAT}\" 2>nul\r\n",
        env = engine.cmd_env_lines(),
        cli = engine.cli,
    )
}

/// True when the collector output carries no `DBXDOCKER_PROBE` marker at
/// all — the signature of a remote shell that cannot run the heredoc
/// (Windows cmd/PowerShell). POSIX hosts always emit the marker, even on a
/// docker-missing verdict, so the [`WIN_LIST_SCRIPT`] retry costs one
/// extra round-trip only on hosts that garbled the first collector.
pub fn needs_windows_fallback(output: &str) -> bool {
    !output.contains("DBXDOCKER_PROBE")
}

/// Elevated logs body for the Quick Sudo retry of `docker/logs`: same
/// inspect→logs order as [`logs_script`], passed as argv (never stdin, the
/// password owns it) through `sanitize_sudo_command` by the callers.
pub fn sudo_logs_body(
    engine: &EngineConfig,
    container_id: &str,
    tail: u64,
) -> Result<String, String> {
    validate_container_id(container_id)?;
    let tail = clamp_tail(tail);
    Ok(format!(
        "{PATH_PREFIX}{export}\
         echo DBXDOCKER_INSPECT\n\
         {cli} inspect {container_id} 2>/dev/null\n\
         echo DBXDOCKER_LOGS\n\
         {cli} logs --tail {tail} {container_id} 2>&1\n",
        export = engine.env_prefix(),
        cli = engine.cli,
    ))
}

/// Builds the logs + inspect collector for one container. Inspect runs
/// first so the (potentially large) log tail cannot bury the marker scan,
/// and the log section is last by construction (everything after its
/// marker is the log text, `2>&1` merged so permission errors surface).
pub fn logs_script(engine: &EngineConfig, container_id: &str, tail: u64) -> Result<String, String> {
    validate_container_id(container_id)?;
    let tail = clamp_tail(tail);
    Ok(format!(
        "sh -s <<'DBXDOCKER'\n\
         {PATH_PREFIX}{export}\
         echo DBXDOCKER_INSPECT\n\
         {cli} inspect {container_id} 2>/dev/null\n\
         echo DBXDOCKER_LOGS\n\
         {cli} logs --tail {tail} {container_id} 2>&1\n\
         DBXDOCKER\n",
        export = engine.env_prefix(),
        cli = engine.cli,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_follow_command_is_fixed_and_clamped() {
        let engine = crate::features::docker::test_engine("docker", None);
        assert_eq!(
            logs_follow_command(&engine, "d4a7c9f1e2b3", 200),
            "docker logs -f --tail 200 d4a7c9f1e2b3"
        );
    }

    #[test]
    fn logs_follow_command_keeps_engine_endpoint() {
        let engine =
            crate::features::docker::test_engine("podman", Some("unix:///tmp/podman.sock"));
        let command = logs_follow_command(&engine, "d4a7c9f1e2b3", 9999);
        assert!(command.contains("DOCKER_HOST=unix:///tmp/podman.sock"));
        assert!(command.ends_with("podman logs -f --tail 2000 d4a7c9f1e2b3"));
    }

    #[test]
    fn stream_registry_caps_active_streams() {
        let registry = DockerStreams::new();
        for index in 0..MAX_ACTIVE_STREAMS {
            registry.insert_test(format!("s{index}"));
        }
        assert!(registry.start_guard().is_err());
    }
}
