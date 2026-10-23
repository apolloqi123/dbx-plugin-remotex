//! Metrics support.

use super::*;

/// Cap for the process command column so the payload stays small.
pub(crate) const PROCESS_COMMAND_MAX_CHARS: usize = 120;

pub(crate) const PROCESS_TOP_LIMIT: usize = 8;

/// A row of the top-process table (sorted by CPU, [`PROCESS_TOP_LIMIT`] max).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetrics {
    pub pid: u64,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub command: String,
}

/// Parses the `ps` top-process section of the collector output.
pub fn parse_processes(output: &str) -> Vec<ProcessMetrics> {
    parse_process_lines(section_of(output, "--ps--"))
}

/// Parses `ps -eo pid=,user=,pcpu=,pmem=,args=` lines (already sorted and
/// limited by the collector pipeline).
pub fn parse_process_lines(text: &str) -> Vec<ProcessMetrics> {
    let mut processes = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // pid user cpu% mem% + at least one command token.
        if fields.len() < 5 {
            continue;
        }
        let Ok(pid) = fields[0].parse::<u64>() else {
            continue;
        };
        let Some(cpu_percent) = parse_percent(fields[2]) else {
            continue;
        };
        let Some(mem_percent) = parse_percent(fields[3]) else {
            continue;
        };
        let command = truncate_chars(&fields[4..].join(" "), PROCESS_COMMAND_MAX_CHARS);
        processes.push(ProcessMetrics {
            pid,
            user: fields[1].to_string(),
            cpu_percent,
            mem_percent,
            command,
        });
        if processes.len() >= PROCESS_TOP_LIMIT {
            break;
        }
    }
    processes
}

pub(crate) fn parse_percent(value: &str) -> Option<f64> {
    value.trim_end_matches('%').parse::<f64>().ok()
}

// —— Process management (ssh/processes/list, ssh/processes/kill) ———

/// Row cap for the full process list so the payload stays bounded.
pub const PROCESS_LIST_LIMIT: usize = 500;

/// Max chars kept of one process command line.
pub(crate) const PROCESS_LIST_COMMAND_MAX_CHARS: usize = 200;

/// Max listening ports reported per process (payload stays bounded even for
/// multi-socket daemons); the frontend shows the truncated list as-is.
pub const PROCESS_PORTS_MAX: usize = 16;

/// Sorted by CPU (busiest first) and capped server-side; the frontend
/// re-sorts client-side without refetching.
///
/// After the `ps` rows the same single read-only command appends three
/// best-effort sections (iShell-style fd counts and listening ports):
/// - `--fds--`: open fd count per pid. Pure shell builtins over
///   `/proc/<pid>/fd` — no extra process is spawned. Unreadable fd
///   directories (other users' processes without root) are skipped, so the
///   column is best-effort by design.
/// - `--sock--`: every `/proc/<pid>/fd/*` symlink pointing at a socket, as
///   `<path>\tsocket:[<inode>]`. `find -lname/-printf` only enumerates the
///   procfs symlinks (a single spawn); all mapping (inode -> port -> pid)
///   happens in the Rust parsers below.
/// - `--netp--`: raw `/proc/net/tcp` + `/proc/net/tcp6` dumps, parsed for
///   listening (state `0A`) sockets.
///
/// No `sudo`/privileged downgrade is used: `/proc/net/tcp{,6}` is
/// world-readable and the fd sections simply stay empty where procfs is
/// unreadable (macOS/BSD), matching the "best effort, blank when unknown"
/// contract. `ss -tlnp` was deliberately not added as a fallback — it cannot
/// attribute ports to pids without the same root rights, so it would add an
/// execution surface without recovering data (see parse functions).
pub(crate) const PROCESS_LIST_SCRIPT: &str = concat!(
    "(ps -eo pid=,ppid=,user=,pcpu=,pmem=,etime=,state=,args= 2>/dev/null || true) ",
    "| sort -k3,3nr | head -n 500; ",
    "echo '--fds--'; ",
    "if [ -d /proc/1 ]; then for d in /proc/[0-9]*; do set -- \"$d\"/fd/*; ",
    "{ [ -L \"$1\" ] || [ -e \"$1\" ]; } || continue; c=0; ",
    "for f in \"$d\"/fd/*; do c=$((c+1)); done; echo \"${d##*/} $c\"; done; fi; ",
    "echo '--sock--'; ",
    "find /proc/[0-9]*/fd -lname 'socket:*' -printf '%p\\t%l\\n' 2>/dev/null || true; ",
    "echo '--netp--'; ",
    "cat /proc/net/tcp /proc/net/tcp6 2>/dev/null || true"
);

/// One row of the full process list (same `ps` dialect as the top-process
/// section, plus ppid/etime/state for the management panel). `fd_count` and
/// `listen_ports` are best-effort extras: `None`/empty means the data was not
/// readable (other user's process, or non-Linux host) and the frontend shows
/// a placeholder instead of a zero.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListRow {
    pub pid: u64,
    pub ppid: u64,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub etime: String,
    pub state: String,
    pub command: String,
    /// Open file descriptors (`/proc/<pid>/fd` entries); `None` when unknown.
    pub fd_count: Option<u64>,
    /// Listening TCP ports owned by this pid (sorted, deduped, capped at
    /// [`PROCESS_PORTS_MAX`]); empty when unknown.
    pub listen_ports: Vec<u16>,
}

/// Collects the full process list over a new exec channel. Read-only.
pub async fn collect_process_list(handle: &Handle<SshClient>) -> Result<serde_json::Value, String> {
    let outcome = exec_plain(
        handle,
        PROCESS_LIST_SCRIPT,
        std::time::Duration::from_secs(15),
        &[],
    )
    .await?;
    Ok(serde_json::json!({ "processes": parse_full_process_list(&outcome.output) }))
}

/// Splits the collector output into its sections: the `ps` rows first, then
/// `--fds--` (pid + fd count), `--sock--` (procfs fd symlink dump) and
/// `--netp--` (/proc/net/tcp{,6} dump). Anything before the first marker is
/// the `ps` part.
pub(crate) fn split_process_sections(text: &str) -> (String, String, String, String) {
    let mut ps = String::new();
    let mut fds = String::new();
    let mut sock = String::new();
    let mut netp = String::new();
    let mut target = &mut ps;
    for line in text.lines() {
        match line.trim_end() {
            "--fds--" => {
                target = &mut fds;
                continue;
            }
            "--sock--" => {
                target = &mut sock;
                continue;
            }
            "--netp--" => {
                target = &mut netp;
                continue;
            }
            _ => {}
        }
        target.push_str(line);
        target.push('\n');
    }
    (ps, fds, sock, netp)
}

/// Parses `--fds--` rows (`<pid> <count>`) into a pid -> fd-count map.
pub fn parse_fd_counts(text: &str) -> BTreeMap<u64, u64> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(count)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(pid), Ok(count)) = (pid.parse::<u64>(), count.parse::<u64>()) else {
            continue;
        };
        map.insert(pid, count);
    }
    map
}

/// Parses `--netp--` (/proc/net/tcp + /proc/net/tcp6) rows into an
/// inode -> local-port map for listening sockets only (state `0A`). Header
/// rows fail the hex port parse and are skipped; both the IPv4 and IPv6
/// tables share the column layout.
pub fn parse_listening_inodes(text: &str) -> BTreeMap<u64, u16> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // sl local_address rem_address st tx:rx tr:when retrnsmt uid timeout inode ...
        if fields.len() < 10 || fields[3] != "0A" {
            continue;
        }
        let Some(port) = fields[1]
            .rsplit(':')
            .next()
            .and_then(|port| u16::from_str_radix(port, 16).ok())
        else {
            continue;
        };
        let Ok(inode) = fields[9].parse::<u64>() else {
            continue;
        };
        map.insert(inode, port);
    }
    map
}

/// Parses the `--sock--` dump (`/proc/<pid>/fd/<n>\tsocket:[<inode>]` rows
/// emitted by find(1)) into pid -> owned socket inodes.
pub fn parse_fd_socket_inodes(text: &str) -> BTreeMap<u64, Vec<u64>> {
    let mut map: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for line in text.lines() {
        let Some((path, target)) = line.split_once('\t') else {
            continue;
        };
        let Some(pid_part) = path.strip_prefix("/proc/") else {
            continue;
        };
        let Some((pid, _)) = pid_part.split_once('/') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u64>() else {
            continue;
        };
        let Some(inode) = target
            .strip_prefix("socket:[")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        map.entry(pid).or_default().push(inode);
    }
    map
}

/// Fills `fd_count` / `listen_ports` from the parsed /proc sections.
pub(crate) fn attach_process_extras(
    mut rows: Vec<ProcessListRow>,
    fds: &BTreeMap<u64, u64>,
    inodes_by_pid: &BTreeMap<u64, Vec<u64>>,
    inode_ports: &BTreeMap<u64, u16>,
) -> Vec<ProcessListRow> {
    for row in &mut rows {
        row.fd_count = fds.get(&row.pid).copied();
        if let Some(inodes) = inodes_by_pid.get(&row.pid) {
            let mut ports: Vec<u16> = inodes
                .iter()
                .filter_map(|inode| inode_ports.get(inode).copied())
                .collect();
            ports.sort_unstable();
            ports.dedup();
            ports.truncate(PROCESS_PORTS_MAX);
            row.listen_ports = ports;
        }
    }
    rows
}

/// Parses the whole collector output (ps rows + the /proc sections) into the
/// final row list. Pure so the Linux/macOS shapes stay unit-testable without
/// a server.
pub fn parse_full_process_list(text: &str) -> Vec<ProcessListRow> {
    let (ps, fds, sock, netp) = split_process_sections(text);
    let rows = parse_process_list(&ps);
    attach_process_extras(
        rows,
        &parse_fd_counts(&fds),
        &parse_fd_socket_inodes(&sock),
        &parse_listening_inodes(&netp),
    )
}

/// Parses the process-list `ps` output. Pure so it can be unit-tested
/// without a server.
pub fn parse_process_list(text: &str) -> Vec<ProcessListRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // pid ppid user cpu% mem% etime state + at least one command token.
        if fields.len() < 8 {
            continue;
        }
        let Ok(pid) = fields[0].parse::<u64>() else {
            continue;
        };
        let Ok(ppid) = fields[1].parse::<u64>() else {
            continue;
        };
        let Some(cpu_percent) = parse_percent(fields[3]) else {
            continue;
        };
        let Some(mem_percent) = parse_percent(fields[4]) else {
            continue;
        };
        rows.push(ProcessListRow {
            pid,
            ppid,
            user: fields[2].to_string(),
            cpu_percent,
            mem_percent,
            etime: fields[5].to_string(),
            state: fields[6].to_string(),
            command: truncate_chars(&fields[7..].join(" "), PROCESS_LIST_COMMAND_MAX_CHARS),
            // Best-effort extras are attached later by parse_full_process_list.
            fd_count: None,
            listen_ports: Vec::new(),
        });
        if rows.len() >= PROCESS_LIST_LIMIT {
            break;
        }
    }
    rows
}

/// Validates a kill request and renders the remote command. Pid 0/1 are
/// refused outright (init / the process group escape hatch); only a small
/// safe signal set is accepted.
pub fn kill_command(pid: u64, signal: u32) -> Result<String, String> {
    if pid <= 1 {
        return Err(format!(
            "Refusing to signal pid {pid}: init and pid 0 are protected"
        ));
    }
    let name = match signal {
        1 => "HUP",
        2 => "INT",
        9 => "KILL",
        15 => "TERM",
        _ => return Err(format!("Unsupported signal {signal}; use 1, 2, 9 or 15")),
    };
    Ok(format!("kill -{name} {pid}"))
}

/// Sends one signal to a remote process. Executed through the same
/// plugin-internal exec path as the metrics collector.
pub async fn kill_process(handle: &Handle<SshClient>, pid: u64, signal: u32) -> Result<(), String> {
    let command = kill_command(pid, signal)?;
    let outcome = exec_plain(handle, &command, std::time::Duration::from_secs(10), &[]).await?;
    if outcome.exit_code != 0 {
        return Err(format!(
            "kill failed (exit {}): {}",
            outcome.exit_code, outcome.output
        ));
    }
    Ok(())
}
