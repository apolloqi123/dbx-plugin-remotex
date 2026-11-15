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

/// Elevated stats body for the Quick Sudo retry of `docker/stats`: same
/// single `stats --no-stream` call as [`stats_script`], passed as argv
/// (never stdin, the password owns it) through `sanitize_sudo_command`.
pub fn sudo_stats_body(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!(
        "{PATH_PREFIX}{export}\
         echo {STATS_MARKER}\n\
         {cli} stats --no-stream --format '{STATS_FORMAT}' {container_id} 2>&1\n",
        export = engine.env_prefix(),
        cli = engine.cli,
    ))
}

/// Builds the one-shot `docker stats` collector for one container. stderr
/// is merged into the marked section so the parser can tell "container not
/// running" (daemon reachable) from "permission denied" (needs Quick
/// Sudo); a successful run prints one JSON object per line.
pub fn stats_script(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!(
        "sh -s <<'DBXDOCKER'\n\
         {PATH_PREFIX}{export}\
         echo {STATS_MARKER}\n\
         {cli} stats --no-stream --format '{STATS_FORMAT}' {container_id} 2>&1\n\
         DBXDOCKER\n",
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
        assert!(command.ends_with("podman logs -f --tail 1000 d4a7c9f1e2b3"));
    }

    /// The follow command must stay flat: it also runs on Windows OpenSSH
    /// hosts whose default shell is cmd.exe. Reaping the remote process is the
    /// SSH `signal` request's job (see `abort_exec_channel`), not a POSIX
    /// shell wrapper's — a wrapper here would break those hosts.
    #[test]
    fn logs_follow_command_stays_flat_for_non_posix_hosts() {
        let engine = crate::features::docker::test_engine("docker", None);
        let command = logs_follow_command(&engine, "d4a7c9f1e2b3", 200);
        assert!(!command.contains("sh -c"), "{command}");
        assert!(!command.contains('\''), "{command}");
        assert_eq!(command, "docker logs -f --tail 200 d4a7c9f1e2b3");
    }

    /// The POSIX supervisor must stay one flat, single-quoted argument (no
    /// embedded single quotes from the endpoint/id), keep the docker child in
    /// the session's process group (no `setsid`, or sshd cleanup cannot reach
    /// it), and kill it on HUP/TERM, stdin EOF and the TTL watchdog.
    #[test]
    fn supervised_follow_command_reaps_child_on_signals_stdin_eof_and_ttl() {
        let engine = crate::features::docker::test_engine("docker", None);
        let command =
            crate::features::docker::supervised_logs_follow_command(&engine, "d4a7c9f1e2b3", 200);
        assert!(
            command.starts_with("DBX_DOCKER_LOGS_CMD='docker logs -f --tail 200 d4a7c9f1e2b3' exec sh -c 'exec 3<&0; $DBX_DOCKER_LOGS_CMD &"),
            "{command}"
        );
        assert!(!command.contains("setsid"), "{command}");
        // The supervisor and watchers must not carry the docker argv (a
        // forked shell inherits it and `ps | grep docker logs` looked like
        // several clients). Only the launched child may show the command.
        assert!(command.contains("$DBX_DOCKER_LOGS_CMD &"), "{command}");
        assert!(command.contains("kill -KILL \\$0"), "{command}");
        // Never broadcast to the process group: not every sshd isolates an
        // exec channel in its own session, so `kill -KILL 0` could reach
        // sibling channels. The trap kills the three known pids instead.
        assert!(!command.contains("kill -KILL 0"), "{command}");
        assert!(
            command.contains("kill -0 $c 2>/dev/null && kill -KILL $c"),
            "{command}"
        );
        assert!(
            command.contains("kill -0 $w 2>/dev/null && kill -KILL $w"),
            "{command}"
        );
        assert!(
            command.contains("kill -0 $d 2>/dev/null && kill -KILL $d"),
            "{command}"
        );
        // Watchdog polls the supervisor and the docker child instead of
        // sleeping for the whole TTL: a SIGKILLed supervisor (trap cannot run)
        // still loses its docker child within a second.
        assert!(command.contains("p=\\$PPID"), "{command}");
        assert!(
            command.contains(&format!("[ \\$n -lt {} ]", STREAM_TTL.as_secs())),
            "{command}"
        );
        assert!(command.contains("sleep 1; n=\\$((n+1))"), "{command}");
        // Do not kill a pid that is already gone (it may be recycled).
        assert!(
            command.contains("kill -0 \\$0 2>/dev/null && kill -KILL \\$0"),
            "{command}"
        );
        assert!(
            command.contains("cat <&3 >/dev/null; kill -KILL \\$0"),
            "{command}"
        );
        // Propagate docker's exit status: the backend's `needsSudo`
        // detection keys on a non-zero exit plus the permission signature.
        assert!(
            command.contains("wait $c; st=$?; kill $w $d 2>/dev/null; exit $st"),
            "{command}"
        );
        assert_eq!(
            command.matches('\'').count(),
            4,
            "the env assignment and sh -c argument are single-quoted: {command}"
        );
    }

    #[test]
    fn supervised_follow_command_keeps_the_endpoint_inside_the_script() {
        let engine =
            crate::features::docker::test_engine("podman", Some("unix:///tmp/podman.sock"));
        let command =
            crate::features::docker::supervised_logs_follow_command(&engine, "d4a7c9f1e2b3", 500);
        assert!(
            command.contains("env DOCKER_HOST=unix:///tmp/podman.sock CONTAINER_HOST=unix:///tmp/podman.sock podman logs -f --tail 500 d4a7c9f1e2b3"),
            "{command}"
        );
    }

    #[test]
    fn windows_platform_is_tracked_per_session() {
        let session = "windows-platform-test-session";
        assert!(!crate::features::docker::remote_is_windows(session));
        crate::features::docker::mark_remote_platform(session, true);
        assert!(crate::features::docker::remote_is_windows(session));
        crate::features::docker::mark_remote_platform(session, false);
        assert!(!crate::features::docker::remote_is_windows(session));
    }

    #[test]
    fn stream_registry_caps_active_streams() {
        let registry = DockerStreams::new();
        for index in 0..MAX_ACTIVE_STREAMS {
            registry.insert_test(format!("s{index}"));
        }
        assert!(registry.try_register_test("pane-x").is_err());
    }
}
