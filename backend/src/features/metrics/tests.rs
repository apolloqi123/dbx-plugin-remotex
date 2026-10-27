use super::*;
use serde_json::json;

#[cfg(unix)]
#[test]
fn local_collector_samples_the_sidecar_host() {
    // 真跑本机 sh（macOS/Linux 开发机即可，CI Windows 不编译该用例）：
    // 断言基础三段齐全即可，数值随机器漂移不作断言。
    let sample = collect_local_metrics().expect("local collector should run on unix hosts");
    assert!(sample.get("cpu").is_some(), "cpu section missing: {sample}");
    assert!(
        sample.get("memory").is_some(),
        "memory section missing: {sample}"
    );
    assert!(
        sample.get("network").is_some(),
        "network section missing: {sample}"
    );
    assert!(
        sample.get("gpu").is_none(),
        "gpu probe must stay ssh-only: {sample}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_collector_async_runs_without_blocking_workers() {
    // `local/metrics` runs a real child process with a timeout+kill so a hung
    // `sh` can never occupy an SDK worker; the successful path must still
    // return the same parsed document.
    let sample = collect_local_metrics_async()
        .await
        .expect("async local collector should run on unix hosts");
    assert!(sample.get("cpu").is_some(), "cpu section missing: {sample}");
    assert!(sample.get("memory").is_some(), "memory section missing");
    assert!(sample.get("network").is_some(), "network section missing");
}

#[cfg(unix)]
#[tokio::test]
async fn local_collector_timeout_kills_the_shell_and_returns_promptly() {
    // A hung shell must be KILLED, not abandoned on the blocking pool: the
    // script records its pid then execs `sleep` (same pid), so after the
    // timeout the recorded process must be gone and the call must have
    // returned well before the sleep would have ended.
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let script = format!("echo $$ > '{}'; exec sleep 30", pid_file.display());
    let started = std::time::Instant::now();
    let error =
        super::collect_local_metrics_script_async("sh", &script, std::time::Duration::from_secs(1))
            .await
            .expect_err("a sleeping collector must time out");
    let elapsed = started.elapsed();
    assert!(
        error.contains("timed out"),
        "unexpected timeout error: {error}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "timeout must return promptly even though the shell slept 30s: {elapsed:?}"
    );
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("script must write its pid before sleeping")
        .trim()
        .parse()
        .expect("pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !alive {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed-out collector shell {pid} is still alive"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn local_collector_grandchild_holding_stdout_cannot_wedge_the_reader() {
    // The shell exits immediately, but its background `sleep` inherits the
    // stdout pipe, so the reader never sees EOF until that grandchild dies.
    // Without the bounded join the collector would block for the full sleep;
    // with it the call must return within READER_JOIN_TIMEOUT and degrade the
    // stream to empty output.
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("grandchild.pid");
    let script = format!(
        "sleep 30 & echo $! > '{}'; echo collected",
        pid_file.display()
    );
    let started = std::time::Instant::now();
    let result = super::collect_local_metrics_script_async(
        "sh",
        &script,
        std::time::Duration::from_secs(10),
    )
    .await;
    let elapsed = started.elapsed();
    // Clean up the grandchild before asserting so a failure cannot leak it.
    if let Ok(raw) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = raw.trim().parse::<u32>() {
            let _ = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status();
        }
    }
    assert!(
        result.is_ok(),
        "an exited shell must still return a document: {result:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(8),
        "reader join must be bounded by READER_JOIN_TIMEOUT, took {elapsed:?}"
    );
}

/// Sends on drop so a test can prove the reader task was actually aborted
/// (not merely detached) by [`join_reader`]'s timeout path.
struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn bounded_reader_join_times_out_aborts_and_returns_empty() {
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let reader = tokio::spawn(async move {
        let _guard = NotifyOnDrop(Some(dropped_tx));
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        vec![b'x']
    });
    let started = std::time::Instant::now();
    let data = join_reader(reader, std::time::Duration::from_millis(50)).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the join must be bounded by the supplied limit"
    );
    assert!(data.is_empty(), "a timed-out stream degrades to empty output");
    tokio::time::timeout(std::time::Duration::from_secs(2), dropped_rx)
        .await
        .expect("the timed-out reader task must be aborted, not detached")
        .expect("drop notification");
}

#[test]
fn collector_grep_captures_memfree_for_the_available_fallback() {
    // 解析端（exec.rs）缺 MemAvailable 时回退 free+buffers+cached；采集端
    // 的 grep 必须把这三个键带出来，否则回退算出 0——MSYS（Windows 本地
    // 终端经 Git-Bash sh）与 pre-3.14 内核都没有 MemAvailable，MEM 带会
    // 恒定钉在 100%。整段断言精确钉住 grep 模式，不做字符串切片手术
    // （切片法在脚本新增第二个 grep -E 时会静默取错片段）。
    assert!(
        METRICS_SCRIPT.contains(
            "grep -E '^(MemTotal|MemAvailable|MemFree|Buffers|Cached|SwapTotal|SwapFree):'",
        ),
        "meminfo grep must capture every key the parser's MemAvailable fallback reads"
    );
}

#[test]
fn msys_meminfo_shape_parses_with_nonzero_available() {
    // 真实 MSYS（Git-Bash sh）/proc/meminfo 经采集 grep 后的准确形状：
    // 只有 MemTotal/MemFree/SwapTotal/SwapFree，没有 MemAvailable。
    // availableBytes 必须落在 MemFree 上，used 不得退化为 total。
    let output = "\
hostname=win-pc
--mem--
MemTotal:       16772124 kB
MemFree:         9774672 kB
SwapTotal:       2490368 kB
SwapFree:        2452336 kB
";
    let sample = parse_metrics_output(output);
    assert_eq!(sample["memory"]["totalBytes"], 16_772_124_u64 * 1024);
    assert_eq!(sample["memory"]["availableBytes"], 9_774_672_u64 * 1024);
    assert_eq!(
        sample["memory"]["usedBytes"],
        (16_772_124_u64 - 9_774_672) * 1024
    );
}

#[cfg(windows)]
#[test]
fn local_collector_end_to_end_on_windows() {
    // Windows 端到端：MEM 带的三个数值必须满足 available > 0 且 used <
    // total——这正是 MSYS 缺 MemAvailable 时曾退化为 MEM=100% 的现场。
    //
    // sh 探测按候选顺序：PATH 之外再试 Git-Bash 标准安装路径——runner 与
    // 常规开发机的 PATH 通常只有 Git\cmd（git.exe），sh.exe 所在的
    // usr\bin 不在 PATH，且不能把整个 usr\bin 挂上去（coreutils 的
    // link.exe 会抢占 MSVC 链接器，见 dbx-ssh-dev 技能的 Windows 故障
    // 表）。CI（windows-regression）里 sh 必须可达，缺失即失败——静默
    // 跳过会让这个双端修复的回归防线变成纸面；本地开发机允许跳过。
    const SH_CANDIDATES: [&str; 3] = [
        "sh",
        "C:\\Program Files\\Git\\bin\\sh.exe",
        "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
    ];
    let sh = SH_CANDIDATES.into_iter().find(|sh| {
        std::process::Command::new(sh)
            .arg("-c")
            .arg("true")
            .output()
            .is_ok()
    });
    let Some(sh) = sh else {
        if std::env::var_os("CI").is_some() {
            panic!(
                "CI must provide a POSIX sh (Git-Bash); fix the environment, not this assertion"
            );
        }
        eprintln!("skipped: no sh on PATH or standard Git-Bash install");
        return;
    };
    let sample =
        collect_local_metrics_with_shell(sh).expect("local collector should run with sh present");
    let memory = &sample["memory"];
    let total = memory["totalBytes"].as_u64().expect("totalBytes");
    let available = memory["availableBytes"].as_u64().expect("availableBytes");
    let used = memory["usedBytes"].as_u64().expect("usedBytes");
    assert!(total > 0, "totalBytes must be positive: {sample}");
    assert!(available > 0, "availableBytes degenerated to 0: {sample}");
    assert!(
        used < total,
        "usedBytes degenerated to total (MEM band pins at 100%): {sample}"
    );
}

/// Full collector output in Linux shape: /proc/net/dev twice around the
/// process table, followed by the base sections.
const LINUX_FIXTURE: &str = "\
hostname=web-01
kernel=6.1.0-18-amd64
loadavg=0.28 0.42 0.35 1/887 23456
uptime=987654.32 456789.01
nproc=8
--net--
Inter-|   Receive                                                |  Transmit
face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
lo: 5000    50    0    0    0     0          0         0  5000    50    0    0    0     0       0          0
eth0: 1000    10    0    0    0     0          0         0  2000    20    0    0    0     0       0          0
eth1: 0    0    0    0    0     0          0         0  0    0    0    0    0     0       0          0
--net2--
Inter-|   Receive                                                |  Transmit
face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
lo: 6500    65    0    0    0     0          0         0  6500    65    0    0    0     0       0          0
eth0: 3000    30    0    0    0     0          0         0  2500    25    0    0    0     0       0          0
eth1: 0    0    0    0    0     0          0         0  0    0    0    0    0     0       0          0
--ps--
1234 root        12.5  4.2 /usr/sbin/nginx -c /etc/nginx/nginx.conf
567 alice        8.0  1.1 /usr/bin/python3 /opt/app/server.py --port 8080
999 postgres      2.3 12.8 postgres: writer process
--psm--
999 postgres      2.3 12.8 postgres: writer process
1234 root        12.5  4.2 /usr/sbin/nginx -c /etc/nginx/nginx.conf
567 alice        8.0  1.1 /usr/bin/python3 /opt/app/server.py --port 8080
--dfi--
Filesystem Inodes IUsed IFree IUse% Mounted on
/dev/sda1 3200000 320000 2880000 10% /
/dev/sdb1 40960 18432 22528 55% /srv
--mem--
MemTotal:       16308856 kB
MemAvailable:   12000000 kB
SwapTotal:      2047996 kB
SwapFree:       2047996 kB
--cpu--
cpu  100 0 200 8000 40 0 0 0 0 0
cpu  200 0 300 8100 40 0 0 0 0 0
--df--
/dev/sda1 51469868 23456780 25879924 48% /
/dev/sdb1 1024000 512000 512000 50% /srv
/dev/sdc1 2048000 1024000 1024000 50% /var
";

/// Collector output in post-fix macOS shape: sysctl/vm_stat/iostat
/// fallbacks re-emit the Linux line shapes (`loadavg`/`uptime` keys,
/// meminfo-style `--mem--` rows, synthetic `cpu` tick rows), `netstat
/// -ibn` network counters and bare `df -i` inode columns.
const MACOS_FIXTURE: &str = "\
hostname=mac-mini
kernel=25.0.0
loadavg= 164.17 195.50 176.54
uptime=1789900000
nproc=10
--net--
Name       Mtu   Network       Address            Ipkts  Ierrs     Ibytes    Opkts  Oerrs     Obytes  Coll
lo0        16384 <Link#1>                       12345      0     1000    12345      0     1000     0
en0        1500  <Link#4>    aa:bb:cc:dd:ee:ff    98765      0  1000000    54321      0   200000     0
en0        1500  fe80::%en0/64 fe80::1%en0        98765      0   900000    54321      0   150000     0
en0        1500  192.168.1    192.168.1.10       98765      0  1000000    54321      0   200000     0
--net2--
Name       Mtu   Network       Address            Ipkts  Ierrs     Ibytes    Opkts  Oerrs     Obytes  Coll
lo0        16384 <Link#1>                       12346      0     2000    12346      0     2000     0
en0        1500  <Link#4>    aa:bb:cc:dd:ee:ff    98805      0  1500000    54361      0   240000     0
en0        1500  fe80::%en0/64 fe80::1%en0        98805      0  1100000    54361      0   200000     0
en0        1500  192.168.1    192.168.1.10       98805      0  1500000    54361      0   240000     0
--ps--
321 jin          45.1  8.3 /Applications/DBX.app/Contents/MacOS/DBX
402 root         3.2  0.4 /usr/libexec/taskgated
--psm--
321 jin          45.1  8.3 /Applications/DBX.app/Contents/MacOS/DBX
402 root         3.2  0.4 /usr/libexec/taskgated
--dfi--
Filesystem         512-blocks       Used Available Capacity  iused      ifree %iused  Mounted on
/dev/disk3s1s1     1942700360   24682824  96661440    21%   458732  480207920    0%   /
devfs                     541        541         0  100%      936          0  100%   /dev
--mem--
MemTotal: 33554432 kB
MemAvailable: 6104352 kB
SwapTotal: 18874368 kB
SwapFree: 809216 kB
--cpu--
cpu  0 0 0 0 0
cpu  97 0 0 3 0
--df--
/dev/disk3s1s1 971350180 12341412 48020792 21% /
devfs 270 270 0 100% /dev
";

#[test]
fn parses_linux_network_rates_and_totals() {
    let network = parse_network_samples(LINUX_FIXTURE);
    // Busy interfaces first; the idle eth1 is hidden.
    assert_eq!(network.len(), 2);
    assert_eq!(network[0].name, "lo");
    assert_eq!(network[0].rx_rate, 1500.0);
    assert_eq!(network[0].tx_rate, 1500.0);
    assert_eq!(network[0].rx_total, 6500);
    // eth0: rx delta 2000, tx delta 500.
    let eth0 = network.iter().find(|net| net.name == "eth0").unwrap();
    assert_eq!(eth0.rx_rate, 2000.0);
    assert_eq!(eth0.tx_rate, 500.0);
    assert_eq!(eth0.rx_total, 3000);
    assert_eq!(eth0.tx_total, 2500);
}

#[test]
fn parses_macos_netstat_headers_and_duplicate_rows() {
    let network = parse_network_samples(MACOS_FIXTURE);
    assert_eq!(network.len(), 2);
    // en0 aggregates the max across per-address rows: rx 1000000 -> 1500000.
    let en0 = network.iter().find(|net| net.name == "en0").unwrap();
    assert_eq!(en0.rx_rate, 500000.0);
    assert_eq!(en0.tx_rate, 40000.0);
    assert_eq!(en0.rx_total, 1500000);
    assert_eq!(en0.tx_total, 240000);
    let lo0 = network.iter().find(|net| net.name == "lo0").unwrap();
    assert_eq!(lo0.rx_rate, 1000.0);
    assert_eq!(lo0.tx_total, 2000);
    // Busiest interface sorts first.
    assert_eq!(network[0].name, "en0");
}

#[test]
fn netstat_rows_without_address_collapse_to_ten_columns() {
    // Real `netstat -ibn` prints rows like lo0's <Link#1> line without an
    // Address column; Obytes stays second-to-last and Ibytes three before.
    let counters = parse_net_counters(
        "Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll\n\
         lo0 16384 <Link#1> 12345 0 1000 12345 0 2000 0\n\
         en0 1500 <Link#4> aa:bb:cc 10 0 100 20 0 300 0\n",
    );
    assert_eq!(
        counters.get("lo0"),
        Some(&NetCounters { rx: 1000, tx: 2000 })
    );
    assert_eq!(counters.get("en0"), Some(&NetCounters { rx: 100, tx: 300 }));
}

#[test]
fn empty_or_missing_network_sections_yield_nothing() {
    assert!(parse_network_samples("--net--\n--net2--\n").is_empty());
    assert!(parse_network_samples("").is_empty());
    assert!(parse_network_samples("hostname=x\n--mem--\n").is_empty());
}

#[test]
fn handles_counter_reset_between_samples() {
    let output = "\
--net--
eth0: 9000    9 0 0 0 0 0 0 8000 8 0 0 0 0 0 0
--net2--
eth0: 100    1 0 0 0 0 0 0 50 1 0 0 0 0 0 0
";
    let network = parse_network_samples(output);
    assert_eq!(network.len(), 1);
    // Counter reset clamps to zero instead of producing a huge spike.
    assert_eq!(network[0].rx_rate, 0.0);
    assert_eq!(network[0].tx_rate, 0.0);
    assert_eq!(network[0].rx_total, 100);
}

#[test]
fn parses_top_processes_with_truncated_commands() {
    let processes = parse_processes(LINUX_FIXTURE);
    assert_eq!(processes.len(), 3);
    assert_eq!(processes[0].pid, 1234);
    assert_eq!(processes[0].user, "root");
    assert_eq!(processes[0].cpu_percent, 12.5);
    assert_eq!(processes[0].mem_percent, 4.2);
    assert_eq!(
        processes[0].command,
        "/usr/sbin/nginx -c /etc/nginx/nginx.conf"
    );
    assert_eq!(processes[2].pid, 999);
    assert_eq!(processes[2].user, "postgres");

    let long_command = "x".repeat(300);
    let text = format!("1 u 1.0 1.0 {long_command}\n");
    let parsed = parse_process_lines(&text);
    assert_eq!(parsed[0].command.chars().count(), PROCESS_COMMAND_MAX_CHARS);

    // Header-like or malformed lines are skipped.
    assert!(parse_process_lines("PID USER %CPU %MEM COMMAND\nnot-a-pid line here\n").is_empty());
}

#[test]
fn extended_json_keeps_base_fields_and_adds_defaults() {
    let metrics = parse_metrics_output(LINUX_FIXTURE);
    // Base fields from exec::parse_metrics_output stay intact.
    assert_eq!(metrics["hostname"], "web-01");
    assert_eq!(metrics["cpu"]["percent"], 66.7);
    assert_eq!(metrics["memory"]["totalBytes"], 16_308_856_u64 * 1024);
    // Network/process/dfi lines run before --mem--/--df-- so the base
    // parser must not misread them as disks: exactly the 3 real df rows.
    assert_eq!(metrics["disks"].as_array().unwrap().len(), 3);
    // New fields always exist (serde-default style for old callers).
    assert_eq!(metrics["network"][0]["name"], "lo");
    assert_eq!(metrics["network"][0]["rxRate"], 1500.0);
    assert_eq!(metrics["network"][0]["rxTotal"], 6500);
    assert_eq!(metrics["processes"][0]["pid"], 1234);
    assert_eq!(metrics["processes"][0]["cpuPercent"], 12.5);
    assert_eq!(metrics["processes"][0]["user"], "root");
    assert_eq!(metrics["topMemory"].as_array().unwrap().len(), 3);
}

#[test]
fn parses_per_core_cpu_and_memory_cache() {
    let output = "\
hostname=web-02
nproc=2
--mem--
MemTotal:       1000 kB
MemAvailable:    400 kB
MemFree:         100 kB
Buffers:          50 kB
Cached:          250 kB
--cpu--
cpu  100 0 200 8000 40
cpu1 50 0 100 4000 20
cpu0 50 0 100 4000 20
cpu  200 0 300 8100 40
cpu1 100 0 150 4050 20
cpu0 100 0 150 4050 20
";
    let metrics = parse_metrics_output(output);
    // Aggregate: delta 100+100 busy over 300 total -> 66.7%.
    assert_eq!(metrics["cpu"]["percent"], 66.7);
    let cores = metrics["cpu"]["perCore"].as_array().unwrap();
    // Sorted by index even though the fixture lists cpu1 before cpu0.
    assert_eq!(cores.len(), 2);
    assert_eq!(cores[0]["index"], 0);
    assert_eq!(cores[0]["percent"], 66.7);
    assert_eq!(cores[1]["index"], 1);
    // Memory: used 600 kB, cache 300 kB (Buffers+Cached), free 400 kB.
    assert_eq!(metrics["memory"]["usedBytes"], 600 * 1024);
    assert_eq!(metrics["memory"]["cacheBytes"], 300 * 1024);
    assert_eq!(metrics["memory"]["freeBytes"], 400 * 1024);
}

#[test]
fn parses_inode_usage_across_df_dialects() {
    // GNU layout: IUse% is the fourth column; busybox prints it third.
    // Header rows carry no percent token and are skipped.
    let rows = parse_inode_usage(
        "Filesystem Inodes IUsed IFree IUse% Mounted on\n\
         /dev/sda1 3200000 320000 2880000 10% /\n\
         tmpfs 100000 50000 50000 50% /run\n",
    );
    assert_eq!(
        rows,
        vec![("/".to_string(), 10.0), ("/run".to_string(), 50.0)]
    );

    let busybox = parse_inode_usage(
        "Filesystem Inodes Used Free Use% Mounted on\n\
         /dev/root 24576 4096 20480 17% /\n",
    );
    assert_eq!(busybox, vec![("/".to_string(), 17.0)]);

    assert!(parse_inode_usage("").is_empty());
    assert!(parse_inode_usage("Filesystem Inodes IUsed IFree IUse% Mounted on").is_empty());
}

#[test]
fn inode_usage_merges_into_matching_mounts_only() {
    let metrics = parse_metrics_output(LINUX_FIXTURE);
    let disks = metrics["disks"].as_array().unwrap();
    assert_eq!(disks[0]["mount"], json!("/"));
    assert_eq!(disks[0]["inodeUsePercent"], 10.0);
    assert_eq!(disks[1]["mount"], json!("/srv"));
    assert_eq!(disks[1]["inodeUsePercent"], 55.0);
    // /var has no df -iP row: the field stays absent instead of nulling.
    assert_eq!(disks[2]["mount"], json!("/var"));
    assert!(disks[2].get("inodeUsePercent").is_none());
}

#[test]
fn top_memory_processes_are_parsed_independently() {
    let metrics = parse_metrics_output(LINUX_FIXTURE);
    let top_memory = metrics["topMemory"].as_array().unwrap();
    assert_eq!(top_memory[0]["pid"], 999);
    assert_eq!(top_memory[0]["memPercent"], 12.8);
    assert_eq!(top_memory[0]["command"], "postgres: writer process");
    // The CPU-sorted `processes` list is untouched by the mem ordering.
    assert_eq!(metrics["processes"][0]["pid"], 1234);

    let empty = parse_metrics_output("--psm--\n--mem--\n");
    assert_eq!(empty["topMemory"].as_array().unwrap().len(), 0);
}

#[test]
fn parses_macos_json_end_to_end() {
    let metrics = parse_metrics_output(MACOS_FIXTURE);
    assert_eq!(metrics["hostname"], "mac-mini");
    // sysctl 回退产出的负载与运行时长被解析成三个 load 与整数秒。
    assert_eq!(metrics["cpu"]["load1"], 164.17);
    assert_eq!(metrics["cpu"]["load15"], 176.54);
    assert_eq!(metrics["uptimeSeconds"], 1_789_900_000_u64);
    // iostat 百分比合成的 tick 行算出 CPU 繁忙率：(97+0)*100/(97+3)。
    assert_eq!(metrics["cpu"]["percent"], 97.0);
    // meminfo 形状的 sysctl 回退行给出完整内存与 swap 视图。
    assert_eq!(metrics["memory"]["totalBytes"], 34_359_738_368_u64);
    assert_eq!(metrics["memory"]["availableBytes"], 6_250_856_448_u64);
    assert_eq!(metrics["memory"]["usedBytes"], 28_108_881_920_u64);
    assert_eq!(metrics["memory"]["swapTotalBytes"], 19_327_352_832_u64);
    assert_eq!(metrics["memory"]["swapUsedBytes"], 18_498_715_648_u64);
    // df -i 的 %iused 列（最右百分号）胜过同行的 Capacity 21%。
    let disks = metrics["disks"].as_array().unwrap();
    assert_eq!(disks[0]["mount"], json!("/"));
    assert_eq!(disks[0]["inodeUsePercent"], 0.0);
    assert_eq!(disks[1]["mount"], json!("/dev"));
    assert_eq!(disks[1]["inodeUsePercent"], 100.0);
    // Network/process extensions stay intact on the fallback path.
    assert_eq!(metrics["network"].as_array().unwrap().len(), 2);
    assert_eq!(
        metrics["processes"][0]["command"],
        "/Applications/DBX.app/Contents/MacOS/DBX"
    );
    assert_eq!(metrics["processes"][0]["pid"], 321);
}

#[test]
fn section_extraction_scopes_to_markers() {
    assert_eq!(
        section_of("--net--\nabc\n--net2--\ndef\n", "--net--"),
        "abc\n"
    );
    assert_eq!(
        section_of("--net--\nabc\n--net2--\ndef\n", "--net2--"),
        "def\n"
    );
    assert_eq!(section_of("no markers here", "--net--"), "");
    assert_eq!(section_of("--net--\nabc", "--net--"), "abc");
}

#[test]
fn serializes_camel_case_payload() {
    let value = serde_json::to_value(NetworkInterfaceMetrics {
        name: "en0".into(),
        rx_rate: 1.5,
        tx_rate: 2.5,
        rx_total: 10,
        tx_total: 20,
    })
    .unwrap();
    assert_eq!(
        value,
        json!({"name": "en0", "rxRate": 1.5, "txRate": 2.5, "rxTotal": 10, "txTotal": 20})
    );
}

// —— A5: os-release identification ————————————————

#[test]
fn parses_os_release_id_and_pretty_name() {
    let text = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n\
                PRETTY_NAME=\"Ubuntu 22.04.3 LTS\"\nVERSION_ID=\"22.04\"\n";
    let (id, pretty) = parse_os_release(text);
    assert_eq!(id.as_deref(), Some("ubuntu"));
    assert_eq!(pretty.as_deref(), Some("Ubuntu 22.04.3 LTS"));
    // ID_LIKE must not be mistaken for ID.

    // CRLF output is tolerated.
    let (id, pretty) = parse_os_release("ID=\"centos\"\r\nPRETTY_NAME=\"CentOS Stream 9\"\r\n");
    assert_eq!(id.as_deref(), Some("centos"));
    assert_eq!(pretty.as_deref(), Some("CentOS Stream 9"));

    // Unquoted values work.
    let (id, _) = parse_os_release("ID=alpine\n");
    assert_eq!(id.as_deref(), Some("alpine"));

    // Missing keys, empty text, and blank values yield nothing.
    assert_eq!(parse_os_release(""), (None, None));
    assert_eq!(parse_os_release("NAME=\"BusyBox\"\n"), (None, None));
    let (id, pretty) = parse_os_release("ID=\"\"\nPRETTY_NAME=\"\"\n");
    assert_eq!(id, None);
    assert_eq!(pretty, None);
}

#[test]
fn os_release_fields_land_in_the_payload_only_when_present() {
    // The collector section parse: ID/PRETTY_NAME become optional top
    // level fields; a missing --os-- section (BSD, busybox minimal)
    // omits both entirely instead of writing nulls.
    let with_os = parse_metrics_output("--os--\nID=ubuntu\nPRETTY_NAME=\"Ubuntu 22.04\"\n");
    assert_eq!(with_os["osId"], "ubuntu");
    assert_eq!(with_os["osPretty"], "Ubuntu 22.04");

    let without_os = parse_metrics_output("--ps--\n");
    assert!(without_os.get("osId").is_none(), "{without_os}");
    assert!(without_os.get("osPretty").is_none(), "{without_os}");

    // A section whose cat found nothing (both paths missing) stays out.
    let empty_section = parse_metrics_output("--os--\n");
    assert!(empty_section.get("osId").is_none());
    assert!(empty_section.get("osPretty").is_none());
}

// —— Process management ——————————————————————————

const PROCESS_LIST_FIXTURE: &str = "\
1234 1200 root        12.5  4.2 10-03:12:05 S /usr/sbin/nginx -c /etc/nginx/nginx.conf
567    1 alice        8.0  1.1 5-01:00:00 R /usr/bin/python3 /opt/app/server.py
 9    1 postgres     2.3 12.8 30-12:00:00 S postgres: writer process
bad line here
";

#[test]
fn parses_full_process_list_rows() {
    let rows = parse_process_list(PROCESS_LIST_FIXTURE);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].pid, 1234);
    assert_eq!(rows[0].ppid, 1200);
    assert_eq!(rows[0].user, "root");
    assert_eq!(rows[0].cpu_percent, 12.5);
    assert_eq!(rows[0].mem_percent, 4.2);
    assert_eq!(rows[0].etime, "10-03:12:05");
    assert_eq!(rows[0].state, "S");
    assert_eq!(rows[0].command, "/usr/sbin/nginx -c /etc/nginx/nginx.conf");
    assert_eq!(rows[2].command, "postgres: writer process");
    // Malformed lines are skipped (bad pid, too few columns).
    assert!(parse_process_list("not-a-pid 1 u 1.0 1.0 1-00:00:00 R cmd\n").is_empty());
    assert!(parse_process_list("").is_empty());
}

#[test]
fn process_list_rows_serialize_camel_case() {
    let value = serde_json::to_value(&parse_process_list(PROCESS_LIST_FIXTURE)[0]).unwrap();
    assert_eq!(value["pid"], 1234);
    assert_eq!(value["ppid"], 1200);
    assert_eq!(value["cpuPercent"], 12.5);
    assert_eq!(value["memPercent"], 4.2);
    assert_eq!(value["command"], "/usr/sbin/nginx -c /etc/nginx/nginx.conf");
}

/// Collector output with the fd / socket / tcp sections appended in the
/// exact shapes the PROCESS_LIST_SCRIPT emits (Linux host, user can read
/// fd dirs of pid 1234 but not 567 — the classic non-root case).
const PROCESS_EXTRAS_FIXTURE: &str = concat!(
    "  1234 1200 root        12.5  4.2 10-03:12:05 S /usr/sbin/nginx\n",
    "   567    1 alice        8.0  1.1 5-01:00:00 R /usr/bin/python3 app\n",
    "--fds--\n",
    "1234 12\n",
    "567 3\n",
    "--sock--\n",
    "/proc/1234/fd/3\tsocket:[4026531001]\n",
    "/proc/1234/fd/4\tsocket:[4026531001]\n",
    "/proc/1234/fd/5\tsocket:[4026531002]\n",
    "/proc/567/fd/7\tsocket:[4026532001]\n",
    "--netp--\n",
    "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n",
    "   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4026531001 1\n",
    "   1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4026531002 1\n",
    "   2: 0100007F:E1B4 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 4026539001 1\n",
    "   3: 00000000000000000000000000000000:07C7 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4026532001 1\n"
);

#[test]
fn fd_counts_are_parsed_and_attached() {
    let rows = parse_full_process_list(PROCESS_EXTRAS_FIXTURE);
    assert_eq!(rows[0].fd_count, Some(12));
    assert_eq!(rows[1].fd_count, Some(3));
    // Rows without a matching --fds-- entry stay None (macOS / unreadable).
    let bare = parse_full_process_list(PROCESS_LIST_FIXTURE);
    assert_eq!(bare[0].fd_count, None);
    assert!(bare[0].listen_ports.is_empty());
}

#[test]
fn listening_ports_are_mapped_to_pids() {
    let rows = parse_full_process_list(PROCESS_EXTRAS_FIXTURE);
    // 0x1F90=8080 and 0x0050=80, deduped (two fds shared one socket) and
    // sorted; 0x07C7=1991 comes from the IPv6 table.
    assert_eq!(rows[0].listen_ports, vec![80, 8080]);
    assert_eq!(rows[1].listen_ports, vec![1991]);
}

#[test]
fn non_listening_and_malformed_tcp_rows_are_skipped() {
    let inodes = parse_listening_inodes(
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
         \x20  0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4026531001 1\n\
         \x20  1: 0100007F:E1B4 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 4026539001 1\n",
    );
    assert_eq!(inodes.len(), 1);
    assert_eq!(inodes[&4026531001], 8080);
    assert!(parse_listening_inodes("").is_empty());
}

#[test]
fn listening_ports_are_capped_and_fd_socket_lines_tolerate_junk() {
    // 20 distinct listening sockets on one pid -> capped at 16.
    let mut netp = String::new();
    for index in 0..20u16 {
        netp.push_str(&format!(
            "   {index}: 00000000:{:04X} 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 {} 1\n",
            10_000 + index,
            5000 + index
        ));
    }
    let inodes_by_pid = parse_fd_socket_inodes(
        (0..20u64)
            .map(|i| format!("/proc/9/fd/{i}\tsocket:[{}]\n", 5000 + i))
            .collect::<String>()
            .as_str(),
    );
    let rows = attach_process_extras(
        parse_process_list("  9 1 root 1.0 1.0 0-01 S init\n"),
        &parse_fd_counts("9 40\n"),
        &inodes_by_pid,
        &parse_listening_inodes(&netp),
    );
    assert_eq!(rows[0].listen_ports.len(), PROCESS_PORTS_MAX);

    // Junk lines in the find dump are ignored.
    assert!(parse_fd_socket_inodes("garbage\n/proc/x/fd/1\tsocket:[abc]\n").is_empty());
    assert!(parse_fd_counts("not-a-pid 3\n").is_empty());
}

#[test]
fn kill_requests_are_validated_and_rendered() {
    assert_eq!(kill_command(1234, 15).unwrap(), "kill -TERM 1234");
    assert_eq!(kill_command(1234, 9).unwrap(), "kill -KILL 1234");
    assert_eq!(kill_command(42, 1).unwrap(), "kill -HUP 42");
    assert_eq!(kill_command(42, 2).unwrap(), "kill -INT 42");
    // init / pid 0 and unsupported signals are refused.
    assert!(kill_command(1, 15).is_err());
    assert!(kill_command(0, 9).is_err());
    assert!(kill_command(1234, 19).is_err());
}

// —— GPU / NPU sections（Task P1-4）——————————————————
