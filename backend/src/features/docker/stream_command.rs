//! Remote command rendering for the POSIX `docker logs -f` supervisor.
//!
//! Kept out of `stream.rs` so the registry/forwarder file stays inside the
//! source-size budget; the public path is re-exported by `stream`.
use super::*;

/// POSIX supervisor around [`logs_follow_command`].
///
/// Some sshd builds keep a channel counted against `MaxSessions` until its
/// child exits, and the SSH `signal` request does not always reach the real
/// `docker` process (forking login shells, wrapper scripts, servers that
/// ignore signals for non-PTY sessions). The supervisor makes the child die
/// without the server's help:
///
/// * the docker CLI is a background child of `sh`;
/// * `HUP`/`TERM` (our abort path) kill that child explicitly;
/// * a `cat` watcher also kills it on **stdin EOF** — the client's
///   `CHANNEL_EOF`/`CHANNEL_CLOSE` closes the session's stdin pipe, which is
///   guaranteed by the protocol even when signal requests are dropped;
/// * a `sleep` watchdog kills it after [`STREAM_TTL`], so even a client that
///   outlived every other cleanup path cannot follow logs forever.
///
/// The docker child deliberately stays in the SSH session's process group
/// (no `setsid`): when the connection dies without a signal request — DBX
/// quitting, sidecar SIGKILL, network loss — sshd's session cleanup reaches
/// the whole group and reaps `docker logs -f`. Detaching with `setsid` was
/// the root cause of orphaned followers (16 accumulated clients kept dockerd
/// busy-spinning at EOF, burning ~3.5 cores).
///
/// `exec sh -c` makes the supervisor the session's direct child regardless of
/// the login shell's exec/ fork behaviour, while docker's stdout still streams
/// straight into the channel. The supervisor itself runs without a TTY and
/// never reads the log output.
pub fn supervised_logs_follow_command(
    engine: &EngineConfig,
    container_id: &str,
    tail: u64,
) -> String {
    // `env VAR=…` instead of the plain command's `export …;` prefix keeps the
    // launch one external command, so the supervisor can exec it directly and
    // the child stays the session's process-group member (no `setsid`).
    let assignments = match engine.endpoint.as_deref() {
        None => String::new(),
        Some(endpoint) => format!("DOCKER_HOST={endpoint} CONTAINER_HOST={endpoint} "),
    };
    let body = format!(
        "{}{} logs -f --tail {} {}",
        assignments,
        engine.cli,
        clamp_tail(tail),
        container_id
    );
    let launch = if assignments.is_empty() {
        body
    } else {
        format!("env {body}")
    };
    // The launch command rides in an environment variable: `ps` otherwise
    // shows the supervisor and its watcher subshells with the full
    // `docker logs ...` argv (forked shells inherit their parent's argv), so a
    // single stream looked like three log clients. Only the real child now
    // matches a `grep docker logs`.
    //
    // Watchers get their own short `sh -c` argv for the same reason. The trap
    // kills the whole process group (after the specific pids), so watcher
    // children (`cat`, `sleep`) cannot outlive a container switch and keep
    // looking like the previous container's log process.
    //
    // `$c`/`$w`/`$d` are assigned before the trap is defined, so the
    // double-quoted trap expands their pids at definition time (the values we
    // want to kill later). fd 3 preserves the channel's stdin for the watcher:
    // POSIX gives background commands /dev/null on stdin when job control is
    // off, which would otherwise make `cat` see instant EOF and kill the
    // stream at once.
    //
    // The watchdog polls instead of sleeping for the whole TTL: it exits as
    // soon as the supervisor or the docker child is gone, so a stop that
    // SIGKILLs the supervisor (uncatchable, trap cannot run) still leaves the
    // watchdog to reap docker within a second instead of keeping a process
    // alive for 30 minutes.
    //
    // The trap kills the three specific pids, never the process group: not
    // every sshd puts each exec channel in its own session/process group, and
    // `kill -KILL 0` could then take out sibling channels on the same
    // connection. `kill -0` guards avoid signalling a recycled pid after a
    // long stream.
    //
    // `st=$?` after `wait` propagates docker's own exit status: the former
    // trailing `kill` made every supervisor exit 0, which hid the
    // daemon-permission signature from the backend's `needsSudo` detection.
    //
    // No `setsid`: the child must stay reachable by sshd's session cleanup.
    let ttl_secs = STREAM_TTL.as_secs();
    format!(
        "DBX_DOCKER_LOGS_CMD='{launch}' exec sh -c 'exec 3<&0; $DBX_DOCKER_LOGS_CMD & c=$!; sh -c \"cat <&3 >/dev/null; kill -KILL \\$0 2>/dev/null\" \"$c\" & w=$!; sh -c \"p=\\$PPID; n=0; while kill -0 \\$p 2>/dev/null && kill -0 \\$0 2>/dev/null && [ \\$n -lt {ttl_secs} ]; do sleep 1; n=\\$((n+1)); done; kill -0 \\$0 2>/dev/null && kill -KILL \\$0 2>/dev/null\" \"$c\" & d=$!; trap \"kill -0 $c 2>/dev/null && kill -KILL $c 2>/dev/null; kill -0 $w 2>/dev/null && kill -KILL $w 2>/dev/null; kill -0 $d 2>/dev/null && kill -KILL $d 2>/dev/null; exit 0\" HUP TERM; wait $c; st=$?; kill $w $d 2>/dev/null; exit $st'"
    )
}
