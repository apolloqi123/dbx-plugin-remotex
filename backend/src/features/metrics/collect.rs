//! Metrics support.

use super::*;

/// Seconds between the two network counter snapshots (the `sleep 1` in the
/// collector script); used as the denominator for B/s rates.
pub(crate) const NET_SAMPLE_INTERVAL_SECS: f64 = 1.0;

pub(crate) const METRICS_SCRIPT: &str = concat!(
    "echo \"hostname=$(hostname 2>/dev/null)\"; ",
    "echo \"kernel=$(uname -r 2>/dev/null)\"; ",
    // Linux /proc first; macOS/BSD falls back to sysctl (braces stripped so
    // the parser sees three plain floats).
    "echo \"loadavg=$(cat /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg 2>/dev/null | tr -d '{}')\"; ",
    // /proc/uptime is two floats; kern.boottime is `{ sec = N, usec = … }` —
    // capture the first `=` value (greedy `.*sec` would grab `usec`).
    "if [ -r /proc/uptime ]; then echo \"uptime=$(cat /proc/uptime 2>/dev/null)\"; ",
    "else boot=$(sysctl -n kern.boottime 2>/dev/null | sed -n 's/^[^=]*= *\\([0-9][0-9]*\\).*/\\1/p'); ",
    "[ -n \"$boot\" ] && echo \"uptime=$(( $(date +%s) - boot ))\"; fi; ",
    "echo \"nproc=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null)\"; ",
    // Network + processes run before the --mem--/--cpu--/--df-- sections so
    // the base parser safely skips these lines (its section state is still
    // empty here and none of the key=value prefixes can match).
    "echo '--net--'; ",
    "if [ -r /proc/net/dev ]; then cat /proc/net/dev 2>/dev/null; ",
    "elif command -v netstat >/dev/null 2>&1; then netstat -ibn 2>/dev/null; fi; ",
    "echo '--net2--'; sleep 1; ",
    "if [ -r /proc/net/dev ]; then cat /proc/net/dev 2>/dev/null; ",
    "elif command -v netstat >/dev/null 2>&1; then netstat -ibn 2>/dev/null; fi; ",
    "echo '--ps--'; ",
    // 兼容性：CentOS 7（procps-ng 3.3.10）的 `-o pid=,user=,…` 空表头+逗号写法
    // 有 bug——退出码 0 但只输出第一列，进程段于是被解析器整段丢弃；busybox
    // （Alpine/OpenWrt）则根本没有 pcpu/pmem。统一改用可重复的 `-o 字段=`，
    // 并在首行校验「字段数 + CPU% 数字」形状：形状不对（老 procps/busybox）
    // 退回 pid,user,rss,args，并按 /proc/meminfo 合成同一行形的
    // 「0.0 CPU% + 真实 MEM%」行（按内存排序）。LC_ALL=C 固定小数点。
    "psrows() { if LC_ALL=C ps -eo pid= -o user= -o pcpu= -o pmem= -o args= 2>/dev/null | ",
    "awk 'NR==1{ if (NF>=5 && $3 ~ /^[0-9]+(\\.[0-9]+)?$/) ok=1; exit } END{exit ok?0:1}'; then ",
    "LC_ALL=C ps -eo pid= -o user= -o pcpu= -o pmem= -o args= 2>/dev/null; ",
    "else LC_ALL=C ps -eo pid= -o user= -o rss= -o args= 2>/dev/null | LC_ALL=C sort -k3,3nr | head -n 8 | ",
    "awk -v total=\"$(awk '/^MemTotal:/{print $2; exit}' /proc/meminfo 2>/dev/null)\" ",
    "'{ pid=$1; user=$2; rss=$3; $1=$2=$3=\"\"; sub(/^ +/, \"\"); printf \"%s %s 0.0 %.1f %s\\n\", pid, user, total ? (rss+0)*100/total : 0, $0 }'; fi; }; ",
    "psrows | LC_ALL=C sort -k3,3nr | head -n 8; ",
    "echo '--psm--'; ",
    "psrows | LC_ALL=C sort -k4,4nr | head -n 8; ",
    "echo '--dfi--'; ",
    "if [ \"$(uname -s 2>/dev/null)\" = Darwin ]; then df -i 2>/dev/null | tail -n +2 | head -n 24; ",
    "else df -iP 2>/dev/null | tail -n +2 | head -n 24; fi; ",
    "echo '--mem--'; ",
    // MemFree/Buffers/Cached are captured alongside MemAvailable so the
    // parser's classic fallback (exec.rs) has real inputs on pre-3.14
    // kernels and the MSYS /proc shim (Git-Bash sh on Windows) — neither
    // carries MemAvailable, and without MemFree the fallback computes 0,
    // which pins the local dock band's MEM at 100%.
    "if [ -r /proc/meminfo ]; then grep -E '^(MemTotal|MemAvailable|MemFree|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null; ",
    // macOS: total from hw.memsize (bytes -> kB); available = free + inactive
    // + speculative pages (vm_stat values carry a trailing dot); swap from
    // vm.swapusage in MB — all re-emitted in meminfo `Key: value kB` shape
    // so the shared parser stays unchanged.
    "else echo \"MemTotal: $(($(sysctl -n hw.memsize 2>/dev/null)/1024)) kB\"; ",
    "pskb=$(($(sysctl -n vm.pagesize 2>/dev/null || sysctl -n hw.pagesize 2>/dev/null)/1024)); ",
    "vm_stat 2>/dev/null | awk -v pskb=\"$pskb\" '/Pages free:/{f=$NF} /Pages inactive:/{ia=$NF} /Pages speculative:/{sp=$NF} END{gsub(/\\./,\"\",f); gsub(/\\./,\"\",ia); gsub(/\\./,\"\",sp); if (f!=\"\") printf \"MemAvailable: %d kB\\n\", (f+ia+sp)*pskb}'; ",
    "sysctl -n vm.swapusage 2>/dev/null | awk -F'[ =M]+' '{if ($2 != \"\") printf \"SwapTotal: %d kB\\nSwapFree: %d kB\\n\", $2*1024, $6*1024}'; fi; ",
    "echo '--cpu--'; ",
    // Aggregate `cpu` line first, then up to 64 per-core `cpuN` lines; the
    // parser uses the aggregate line as a sample boundary, so both passes
    // keep the same order.
    "if [ -r /proc/stat ]; then head -n 1 /proc/stat; grep -E '^cpu[0-9]+ ' /proc/stat 2>/dev/null | head -n 64; sleep 0.4; head -n 1 /proc/stat; grep -E '^cpu[0-9]+ ' /proc/stat 2>/dev/null | head -n 64; ",
    // macOS: `iostat -c 2` takes two samples 1s apart; the second is a real
    // delta. The us/sy/id columns sit at a variable offset (per-disk columns
    // precede them), so anchor on the header row and re-emit the percentages
    // as two synthetic tick rows for the shared delta parser.
    "else iostat -c 2 2>/dev/null | awk 'NR==2{for(i=1;i<=NF;i++) if($i==\"id\") c=i} END{if(c){us=$(c-2); sy=$(c-1); id=$c; if (us ~ /^[0-9]+$/ && sy ~ /^[0-9]+$/ && id ~ /^[0-9]+$/) printf \"cpu  0 0 0 0 0\\ncpu  %d 0 0 %d 0\\n\", us+sy, id}}'; fi; ",
    "echo '--df--'; df -kP 2>/dev/null | tail -n +2 | head -n 24; ",
    // Distribution identification (IMPL_PLAN §1.5): both standard paths,
    // first present wins; neither existing (BSD, busybox minimal) leaves the
    // section empty and the payload fields omitted entirely.
    "echo '--os--'; cat /etc/os-release 2>/dev/null || cat /usr/lib/os-release 2>/dev/null"
);

/// Collects the extended metrics sample over a new exec channel. Read-only
/// commands only; the extra `sleep 1` is bounded by the timeout below. The
/// GPU/NPU accelerator overviews ride the same document (best-effort probe,
/// see [`merge_accelerator_sections`]).
/// Collector with the accelerator probes switchable. The dock panel samples
/// every 5s for as long as a connection is open; probing GPU/NPU each time
/// costs two extra exec channels per sample even on hosts that have neither.
/// Callers that already know the host has no accelerator pass `false` and get
/// a document without the `gpu`/`npu` keys (the frontend hides both sections
/// when the keys are absent).
pub async fn collect_metrics_with_accelerators(
    handle: &Handle<SshClient>,
    probe_accelerators: bool,
) -> Result<serde_json::Value, String> {
    // Plugin-internal collector: no client setEnv so locale overrides on the
    // connection cannot reshape the output this parser expects.
    let outcome = exec_plain(
        handle,
        METRICS_SCRIPT,
        std::time::Duration::from_secs(30),
        &[],
    )
    .await?;
    if outcome.exit_code != 0 && outcome.output.is_empty() {
        // output 被条件保证为空，格式化它只会产出 "failed: " 空因；退出码
        // 是这里唯一可用的线索。
        return Err(format!(
            "metrics collection failed (exit {})",
            outcome.exit_code
        ));
    }
    let mut metrics = parse_metrics_output(&outcome.output);
    if probe_accelerators {
        merge_accelerator_sections(handle, &mut metrics).await;
    }
    Ok(metrics)
}

/// Upper bound for one local collector run: the script itself sleeps ~1.4s
/// and runs a handful of POSIX tools; 20s leaves room for a loaded host while
/// still failing long before a stuck shell can wedge the caller.
pub(crate) const LOCAL_METRICS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// `local/metrics`: the same collector run through the sidecar host's own
/// shell. Local terminals have no SSH session, so the dock panel's system
/// band samples the local machine instead of a remote one. POSIX sh only:
/// macOS/Linux have one; on Windows it comes from Git-Bash (MSYS), whose
/// /proc shim lacks MemAvailable — the parser's classic fallback (exec.rs)
/// covers that shape. GPU/NPU probes stay SSH-only — the base document omits
/// both sections and [`project_metrics_sections`] treats missing keys like
/// any other absent section.
///
/// The collector runs as a real child process (`tokio::process`) and is
/// explicitly killed on timeout: `spawn_blocking(Command::output())` could not
/// be cancelled, so a hung shell kept a blocking thread alive forever and
/// repeated timeouts could exhaust the blocking pool.
pub async fn collect_local_metrics_async() -> Result<serde_json::Value, String> {
    collect_local_metrics_with_shell_async("sh").await
}

pub(crate) async fn collect_local_metrics_with_shell_async(
    shell: &str,
) -> Result<serde_json::Value, String> {
    run_local_metrics_script_async(shell, METRICS_SCRIPT, LOCAL_METRICS_TIMEOUT).await
}

/// Test hook: run an arbitrary script with an explicit timeout so the
/// timeout-kill behavior can be proven without waiting 20s.
#[cfg(test)]
pub(crate) async fn collect_local_metrics_script_async(
    shell: &str,
    script: &str,
    limit: std::time::Duration,
) -> Result<serde_json::Value, String> {
    run_local_metrics_script_async(shell, script, limit).await
}

async fn run_local_metrics_script_async(
    shell: &str,
    script: &str,
    limit: std::time::Duration,
) -> Result<serde_json::Value, String> {
    let mut child = tokio::process::Command::new(shell)
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Belt and braces: if the task is dropped before the explicit kill
        // path runs, the child is still killed instead of leaking.
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("local metrics unavailable: {error}"))?;
    let stdout_reader = tokio::spawn(read_all(child.stdout.take()));
    let stderr_reader = tokio::spawn(read_all(child.stderr.take()));
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            stdout_reader.abort();
            stderr_reader.abort();
            return Err(format!("local metrics collection failed: {error}"));
        }
        Err(_) => {
            // Explicit kill + reap: dropping a `spawn_blocking` handle used to
            // leave the shell running. Readers are aborted so a grandchild
            // holding the pipes cannot keep us waiting.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            stdout_reader.abort();
            stderr_reader.abort();
            return Err(format!(
                "local metrics collection timed out after {}",
                format_timeout(limit)
            ));
        }
    };
    // Bounded joins: the shell is already reaped, but a grandchild that
    // inherited the pipes can keep EOF from ever arriving. Joining both
    // concurrently caps the total wait at one READER_JOIN_TIMEOUT instead of
    // one per stream. A timed-out stream degrades to empty output (same as a
    // panicked reader), so the acceptance rule below still applies.
    let (stdout, _stderr) = tokio::join!(
        join_reader(stdout_reader, READER_JOIN_TIMEOUT),
        join_reader(stderr_reader, READER_JOIN_TIMEOUT),
    );
    let text = String::from_utf8_lossy(&stdout).into_owned();
    // Same acceptance rule as [`collect_metrics_with_accelerators`]: a non-zero exit with no
    // output is a failure; defensive `|| true` fallbacks normally keep the
    // script at 0 with parseable output even on partial /proc holes.
    if !status.success() && text.trim().is_empty() {
        return Err(format!("local metrics collection failed: {status}"));
    }
    Ok(parse_metrics_output(&text))
}

/// Upper bound for draining one collector stream after the child exited.
/// Two seconds is ample for bytes the kernel already buffered, while a
/// grandchild holding the pipe open must not wedge the caller forever.
pub(crate) const READER_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Joins one pipe-reader task with a bound. On timeout the task is aborted
/// and the stream degrades to empty output (the same rule as a panicked
/// reader task): the call returns promptly and consistently instead of
/// waiting on a grandchild that outlives the shell.
pub(crate) async fn join_reader(
    mut reader: tokio::task::JoinHandle<Vec<u8>>,
    limit: std::time::Duration,
) -> Vec<u8> {
    match tokio::time::timeout(limit, &mut reader).await {
        Ok(Ok(data)) => data,
        Ok(Err(_)) => Vec::new(),
        Err(_) => {
            reader.abort();
            Vec::new()
        }
    }
}

async fn read_all<R: tokio::io::AsyncRead + Unpin>(mut reader: Option<R>) -> Vec<u8> {
    let mut data = Vec::new();
    if let Some(reader) = reader.as_mut() {
        let _ = tokio::io::AsyncReadExt::read_to_end(reader, &mut data).await;
    }
    data
}

fn format_timeout(limit: std::time::Duration) -> String {
    if limit.subsec_millis() > 0 && limit.as_secs() == 0 {
        format!("{}ms", limit.as_millis())
    } else {
        format!("{}s", limit.as_secs())
    }
}

#[cfg(test)]
pub fn collect_local_metrics() -> Result<serde_json::Value, String> {
    collect_local_metrics_with_shell("sh")
}

#[cfg(test)]
pub(crate) fn collect_local_metrics_with_shell(shell: &str) -> Result<serde_json::Value, String> {
    let output = std::process::Command::new(shell)
        .arg("-c")
        .arg(METRICS_SCRIPT)
        .output()
        .map_err(|error| format!("local metrics unavailable: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    // Same acceptance rule as [`collect_metrics_with_accelerators`]: a non-zero exit with no
    // output is a failure; defensive `|| true` fallbacks normally keep the
    // script at 0 with parseable output even on partial /proc holes.
    if !output.status.success() && text.trim().is_empty() {
        return Err(format!(
            "local metrics collection failed: {}",
            output.status
        ));
    }
    Ok(parse_metrics_output(&text))
}

/// Runs the GPU and NPU probes concurrently and merges the results under the
/// `gpu` / `npu` top-level keys. Best-effort by design: a failed probe (no
/// accelerator, restricted driver, exec hiccup) degrades to
/// `{"available": false}` so it can never fail the base metrics collection,
/// and [`project_metrics_sections`] treats the keys like any other section.
pub(crate) async fn merge_accelerator_sections(
    handle: &Handle<SshClient>,
    metrics: &mut serde_json::Value,
) {
    let (gpu, npu) = tokio::join!(
        metrics_gpu::collect_gpu_overview(handle),
        metrics_gpu::collect_npu_overview(handle)
    );
    if let Some(object) = metrics.as_object_mut() {
        object.insert(
            "gpu".to_string(),
            gpu.unwrap_or_else(|_| serde_json::json!({ "available": false, "gpus": [] })),
        );
        object.insert(
            "npu".to_string(),
            npu.unwrap_or_else(
                |_| serde_json::json!({ "available": false, "cann": null, "devices": [] }),
            ),
        );
    }
}
