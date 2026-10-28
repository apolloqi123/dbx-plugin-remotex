use super::*;
use crate::core::model::TerminalFrame;

#[test]
fn start_request_env_defaults_to_empty() {
    // Older workbenches never send `env`: the field must stay optional.
    let request: LocalTerminalStartRequest =
        serde_json::from_str(r#"{"workbenchId":"w","cols":80,"rows":24}"#).unwrap();
    assert!(request.env.is_empty());
}

#[test]
fn user_env_sanitizes_drops_and_reserves() {
    let env = BTreeMap::from([
        ("HOME".to_string(), "/tmp/x".to_string()),
        ("TERM".to_string(), "vt100".to_string()),
        ("colorterm".to_string(), "none".to_string()),
        ("  ".to_string(), "v".to_string()),
        ("A=B".to_string(), "v".to_string()),
        ("BAD\u{7}".to_string(), "v".to_string()),
        ("OVER".to_string(), "x".repeat(USER_ENV_MAX_VALUE + 1)),
        (" OK ".to_string(), "kept".to_string()),
    ]);
    // BTreeMap iterates by the RAW keys (space sorts below letters), so
    // the trimmed " OK " pair comes out first — order stays deterministic.
    assert_eq!(
        sanitize_user_env(&env),
        vec![
            ("OK".to_string(), "kept".to_string()),
            ("HOME".to_string(), "/tmp/x".to_string()),
        ]
    );
}

#[test]
fn user_env_pair_cap_trims_deterministically() {
    let env: BTreeMap<String, String> = (0..USER_ENV_MAX_PAIRS + 5)
        .map(|i| (format!("K{i:03}"), "v".to_string()))
        .collect();
    let pairs = sanitize_user_env(&env);
    assert_eq!(pairs.len(), USER_ENV_MAX_PAIRS);
    assert_eq!(pairs[0].0, "K000");
    assert_eq!(pairs[USER_ENV_MAX_PAIRS - 1].0, "K031");
}

/// The sanitized-pairs shape `start` feeds [`fallback_lang`].
fn sanitized(map: &[(&str, &str)]) -> Vec<(String, String)> {
    map.iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn nothing_set(_key: &str) -> Option<String> {
    None
}

#[test]
fn fallback_lang_fills_the_gap_when_no_locale_is_spoken() {
    let no_env: Vec<(String, String)> = Vec::new();
    assert_eq!(
        fallback_lang(&no_env, Platform::MacOS, &nothing_set),
        Some("en_US.UTF-8")
    );
    assert_eq!(
        fallback_lang(&no_env, Platform::Linux, &nothing_set),
        Some("C.UTF-8")
    );
}

#[test]
fn fallback_lang_stays_out_when_the_user_configured_a_locale() {
    for key in ["LANG", "LC_ALL", "LC_CTYPE"] {
        let pairs = sanitized(&[(key, "zh_CN.UTF-8")]);
        assert_eq!(fallback_lang(&pairs, Platform::MacOS, &nothing_set), None);
    }
    // An unrelated LC_* key does not decide the character locale: the
    // fallback still applies beneath it.
    let pairs = sanitized(&[("LC_MESSAGES", "zh_CN.UTF-8")]);
    assert_eq!(
        fallback_lang(&pairs, Platform::MacOS, &nothing_set),
        Some("en_US.UTF-8")
    );
}

#[test]
fn fallback_lang_stays_out_when_the_sidecar_environment_has_one() {
    let inherited = |key: &str| (key == "LC_CTYPE").then(|| "en_US.UTF-8".to_string());
    let no_env: Vec<(String, String)> = Vec::new();
    assert_eq!(fallback_lang(&no_env, Platform::MacOS, &inherited), None);
    // An inherited EMPTY value speaks no locale (an exported-but-empty
    // LANG must not veto the fallback): the gap still gets filled.
    let empty = |key: &str| (key == "LANG").then(String::new);
    assert_eq!(
        fallback_lang(&no_env, Platform::MacOS, &empty),
        Some("en_US.UTF-8")
    );
}

#[test]
fn fallback_lang_skips_windows_where_lang_cannot_reach_the_console() {
    let no_env: Vec<(String, String)> = Vec::new();
    assert_eq!(
        fallback_lang(&no_env, Platform::Windows, &nothing_set),
        None
    );
}

#[test]
fn windows_utf8_console_folds_into_the_integration_command() {
    // Injection on: the switch joins the existing -Command payload —
    // argv stays one command, no extra window of shell startup args.
    let args = apply_windows_utf8_console(
        Platform::Windows,
        ShellKind::PowerShell,
        vec![
            "-NoExit".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-Command".to_string(),
            "& 'C:\\temp\\si\\integration.ps1'".to_string(),
        ],
    );
    assert_eq!(
        args,
        vec![
            "-NoExit",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "chcp 65001 >$null; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; & 'C:\\temp\\si\\integration.ps1'"
        ]
    );
}

#[test]
fn windows_utf8_console_switches_a_bare_powershell_and_keeps_it_open() {
    // Injection off (toggle or script-write failure): the shell still
    // gets the switch, and -NoExit keeps it interactive as before.
    assert_eq!(
        apply_windows_utf8_console(Platform::Windows, ShellKind::PowerShell, Vec::new()),
        vec![
            "-NoExit",
            "-Command",
            "chcp 65001 >$null; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8"
        ]
    );
}

#[test]
fn windows_utf8_console_wraps_cmd_and_restores_its_title() {
    // cmd has no injection seam: /k runs the switch, the title reset
    // keeps the console title on the cmd path (what a plain cmd shows)
    // instead of leaking the chcp command line into the tab.
    assert_eq!(
        apply_windows_utf8_console(Platform::Windows, ShellKind::Cmd, Vec::new()),
        vec!["/k", "chcp 65001>nul&title %COMSPEC%"]
    );
}

#[test]
fn windows_utf8_console_leaves_other_shells_and_unix_untouched() {
    // WSL (arrives as ShellKind::Other) manages its own UTF-8 world and
    // arbitrary shells have no safe argv seam.
    assert_eq!(
        apply_windows_utf8_console(
            Platform::Windows,
            ShellKind::Other,
            vec!["--flag".to_string()]
        ),
        vec!["--flag"]
    );
    // Unix keeps its locale injection path; the argv rewrite never runs.
    assert_eq!(
        apply_windows_utf8_console(Platform::MacOS, ShellKind::Cmd, Vec::new()),
        Vec::<String>::new()
    );
    assert_eq!(
        apply_windows_utf8_console(
            Platform::Linux,
            ShellKind::PowerShell,
            vec!["-Command".to_string(), "exit".to_string()]
        ),
        vec!["-Command", "exit"]
    );
}

#[test]
fn handshake_filter_swallows_the_query_and_reports_it() {
    let mut filter = ConptyHandshakeFilter::new(true);
    let (forward, handshake) = filter.filter(b"noise\x1b[6n".to_vec());
    assert!(handshake);
    assert_eq!(forward, b"noise");
    // Once answered, everything passes through — terminal-visible bytes
    // never lose their tail and the terminal never sees the query.
    let (forward, handshake) = filter.filter(b"banner".to_vec());
    assert!(!handshake);
    assert_eq!(forward, b"banner");
}

#[test]
fn handshake_filter_catches_a_query_split_across_chunks() {
    let mut filter = ConptyHandshakeFilter::new(true);
    let (forward, handshake) = filter.filter(b"banner\x1b[6".to_vec());
    // The partial query is held, not forwarded, and not yet answered.
    assert!(!handshake);
    assert_eq!(forward, b"banner");
    let (forward, handshake) = filter.filter(b"n rest".to_vec());
    assert!(handshake);
    assert_eq!(forward, b" rest");
}

#[test]
fn handshake_filter_flushes_held_bytes_that_never_complete() {
    let mut filter = ConptyHandshakeFilter::new(true);
    let (forward, handshake) = filter.filter(b"prompt\x1b[".to_vec());
    assert!(!handshake);
    assert_eq!(forward, b"prompt");
    // Not a query after all: the held prefix reaches the terminal.
    let (forward, handshake) = filter.filter(b"?6n".to_vec());
    assert!(!handshake);
    assert_eq!(forward, b"\x1b[?6n");
}

#[test]
fn handshake_filter_ignores_private_dsr_and_leaves_stream_intact() {
    let mut filter = ConptyHandshakeFilter::new(true);
    let (forward, handshake) = filter.filter(b"\x1b[?6n\x1b[?25h".to_vec());
    assert!(!handshake);
    assert_eq!(forward, b"\x1b[?6n\x1b[?25h");
    // DECDSR is not the handshake, so the real query is still answered —
    // and swallows only itself.
    let (forward, handshake) = filter.filter(b"\x1b[?6n\x1b[6n\x1b[?25h".to_vec());
    assert!(handshake);
    assert_eq!(forward, b"\x1b[?6n\x1b[?25h");
}

#[test]
fn handshake_filter_passes_everything_when_not_needed() {
    // Unix openpty has no handshake: not a byte is held back or dropped.
    let mut filter = ConptyHandshakeFilter::new(false);
    let (forward, handshake) = filter.filter(b"\x1b[6n".to_vec());
    assert!(!handshake);
    assert_eq!(forward, b"\x1b[6n");
}

#[test]
fn handshake_filter_flush_releases_a_trailing_prefix() {
    let mut filter = ConptyHandshakeFilter::new(true);
    let (forward, _) = filter.filter(b"bye\x1b[6".to_vec());
    assert_eq!(forward, b"bye");
    assert_eq!(filter.flush(), b"\x1b[6");
    assert!(filter.flush().is_empty());
}

#[test]
fn conpty_cpr_reply_carries_cursor_position() {
    assert_eq!(conpty_cpr_reply(3, 7), b"\x1b[3;7R".to_vec());
    assert_eq!(conpty_cpr_reply(1, 1), b"\x1b[1;1R".to_vec());
}

#[test]
fn inherited_cursor_defaults_to_top_left_without_a_report() {
    // An older workbench sends nothing: the top-left is the only truthful
    // guess for a terminal that was just laid out for a new session.
    assert_eq!(inherit_cursor(None, None, 30, 100), (1, 1));
    assert_eq!(inherit_cursor(Some(5), None, 30, 100), (5, 1));
}

#[test]
fn inherited_cursor_is_clamped_into_the_viewport() {
    // conhost reads the reply as a real position; a row past the last one
    // would be an inherit it cannot honour.
    assert_eq!(inherit_cursor(Some(0), Some(0), 30, 100), (1, 1));
    assert_eq!(inherit_cursor(Some(999), Some(999), 30, 100), (30, 100));
    assert_eq!(inherit_cursor(Some(12), Some(7), 30, 100), (12, 7));
}

/// Windows-only probe of conhost's startup handshake, measured on the
/// windows-2022 runner (inbox conhost 20348). Facts established here, each
/// one refuting a theory that had been shipping:
///
///   * conhost issues `CSI 6n` **exactly once**, and only after the child
///     exists — nothing arrives while the PTY has no client (0 bytes).
///   * a **duplicate** CPR reply is ignored outright (0 bytes followed it),
///     so neither the terminal's leaked answer nor the sidecar's own reply
///     can resize anything. The "duplicate reply" family of theories is dead.
///   * the orphan first glyph is **conhost's own doing**, and where it lands
///     follows the reply's numbers: replying `30;100` produced
///     `... \e[?25h M\r\n \e[29;100H Microsoft Windows [Version ...` — the
///     banner's first character printed at the top, an absolute jump to the
///     viewport's bottom row, then the banner re-drawn there. That is
///     precisely the "M" users report.
///
/// So the remaining variable is the reply's *value*. This runs one cmd.exe
/// session per variant and prints the raw bytes (`--nocapture`): the sidecar's
/// viewport-size answer, a truthful cursor position (what xterm would say), a
/// mismatched size to tell "read as a position" from "read as a size", and no
/// reply at all as the output-withholding control.
/// Escapes raw terminal bytes for the CI log (`--nocapture`).
fn probe_escape(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match byte {
            b'\x1b' => "\\e".to_string(),
            b'\r' => "\\r".to_string(),
            b'\n' => "\\n".to_string(),
            0x20..=0x7e => (*byte as char).to_string(),
            other => format!("\\x{other:02x}"),
        })
        .collect::<String>()
}

/// Drives one real ConPTY session and returns everything the PTY emitted.
///
/// `reply` answers the inherit-cursor handshake the way the sidecar does.
/// `None` means the program could not be spawned (not installed on this
/// machine) — callers skip it instead of failing the suite.
fn probe_conpty_session(
    program: &str,
    reply: Option<&[u8]>,
    rows: u16,
    cols: u16,
) -> Option<Vec<u8>> {
    use std::sync::mpsc;
    use std::time::Instant;

    let saw = |haystack: &[u8], needle: &[u8]| {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    };
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .ok()?;
    let mut child = pair
        .slave
        .spawn_command(CommandBuilder::new(program))
        .ok()?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().ok()?;
    let mut writer = pair.master.take_writer().ok()?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || tx.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut stream = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut answered = false;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(chunk) => {
                stream.extend_from_slice(&chunk);
                if !answered && saw(&stream, CONPTY_CPR_QUERY) {
                    if let Some(reply) = reply {
                        writer.write_all(reply).expect("write reply");
                        writer.flush().expect("flush reply");
                    }
                    answered = true;
                    // Let the first frame settle before finishing.
                    std::thread::sleep(Duration::from_millis(700));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if answered {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Some(stream)
}

/// Runs one probed session with a watchdog: a wedged ConPTY must not hang
/// the runner. Returns `None` for a missing program or a timeout.
fn probe_once(program: &str, reply: Option<Vec<u8>>, rows: u16, cols: u16) -> Option<Vec<u8>> {
    use std::sync::mpsc;
    let program = program.to_string();
    let label = program.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe_conpty_session(&program, reply.as_deref(), rows, cols));
    });
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("[{label}] HUNG: no result within 60s");
            None
        }
    }
}

/// Windows-only probe of conhost's startup handshake, measured on the
/// windows-2022 runner (inbox conhost 20348). Facts established here, each
/// one refuting a theory that had been shipping:
///
///   * conhost issues `CSI 6n` **exactly once**, and only after the child
///     exists — nothing arrives while the PTY has no client (0 bytes).
///   * a **duplicate** CPR reply is ignored outright (0 bytes followed it),
///     so neither the terminal's leaked answer nor the sidecar's own reply
///     can resize anything. The "duplicate reply" family of theories is dead.
///   * the orphan first glyph is **conhost's own doing**, and where it lands
///     follows the reply's numbers: answering the viewport size (`30;100`)
///     produced `... \e[?25h M\r\n \e[29;100H Microsoft Windows ...` — the
///     banner's first character printed at the top, an absolute jump to the
///     viewport's bottom row, then the banner re-drawn there. That is
///     precisely the "M" users reported, and it is why answering with the
///     cursor position (`1;1`, what the workbench now reports) is the fix:
///     the same session then emits one clean frame with no jump.
///
/// Both variants are asserted, so a conhost change that invalidates the
/// reasoning fails here rather than in a user's terminal.
///
/// Deliberately NOT `#[cfg(windows)]`: the body drives a real ConPTY, but
/// gating the whole item would hide it from `cargo test` on macOS/Linux —
/// where a type error in it (a moved closure, say) only surfaces after a
/// Windows CI round trip. It compiles everywhere and returns early where
/// there is no ConPTY to drive.
#[test]
fn conpty_handshake_reply_variants() {
    if !cfg!(windows) {
        return;
    }
    const ROWS: u16 = 30;
    const COLS: u16 = 100;
    // The artifact signature: after the handshake, conhost jumps to the
    // inherited bottom row and re-serializes the screen from there.
    let bottom_jump = format!("\x1b[{};{}H", ROWS - 1, COLS).into_bytes();

    let shipping = probe_once(
        "cmd.exe",
        Some(format!("\x1b[{ROWS};{COLS}R").into_bytes()),
        ROWS,
        COLS,
    )
    .expect("cmd.exe did not start");
    eprintln!(
        "[size {ROWS};{COLS}] {} bytes: {}",
        shipping.len(),
        probe_escape(&shipping)
    );
    assert!(
        shipping
            .windows(bottom_jump.len())
            .any(|window| window == bottom_jump),
        "the viewport-size reply no longer triggers the bottom-row \
         re-serialization this fix was built on: {}",
        probe_escape(&shipping)
    );

    let truthful = probe_once("cmd.exe", Some(b"\x1b[1;1R".to_vec()), ROWS, COLS)
        .expect("cmd.exe did not start");
    eprintln!(
        "[cursor 1;1] {} bytes: {}",
        truthful.len(),
        probe_escape(&truthful)
    );
    assert!(
        !truthful
            .windows(bottom_jump.len())
            .any(|window| window == bottom_jump),
        "the cursor-position reply still produced a bottom-row \
         re-serialization: {}",
        probe_escape(&truthful)
    );
    // The banner is written once, not echoed back from the inherited
    // position — the doubled-prompt shape starts with a lone first glyph.
    let banner = b"Microsoft Windows";
    let banner_hits = truthful
        .windows(banner.len())
        .filter(|window| *window == banner)
        .count();
    assert_eq!(
        banner_hits,
        1,
        "banner written {banner_hits} times with the cursor reply: {}",
        probe_escape(&truthful)
    );
}

/// The three shells users reported the artifact on, each driven under the
/// reply the sidecar now sends. Their banners differ, but the failure mode
/// is conhost-side and identical: a jump to the inherited bottom row with
/// the first frame re-serialized there. Shells that are not installed on
/// the runner are skipped (recorded in the log, not failed).
#[test]
fn conpty_first_frame_stays_clean_across_windows_shells() {
    if !cfg!(windows) {
        return;
    }
    const ROWS: u16 = 30;
    const COLS: u16 = 100;
    let bottom_jump = format!("\x1b[{};{}H", ROWS - 1, COLS).into_bytes();
    for program in ["cmd.exe", "powershell.exe", "wsl.exe"] {
        match probe_once(program, Some(b"\x1b[1;1R".to_vec()), ROWS, COLS) {
            Some(stream) => {
                eprintln!(
                    "[{program}] {} bytes: {}",
                    stream.len(),
                    probe_escape(&stream)
                );
                assert!(
                    !stream
                        .windows(bottom_jump.len())
                        .any(|window| window == bottom_jump),
                    "{program} re-serialized its first frame from the inherited \
                     bottom row: {}",
                    probe_escape(&stream)
                );
            }
            None => eprintln!("[{program}] skipped (not installed or no output)"),
        }
    }
}

#[test]
fn pty_resize_drops_same_size_requests() {
    assert_eq!(requested_pty_size(32, 120, 32, 120), None);
    assert_eq!(requested_pty_size(24, 80, 24, 80), None);
}

#[test]
fn pty_resize_passes_real_geometry_changes() {
    let size = requested_pty_size(24, 80, 30, 100).unwrap();
    assert_eq!((size.rows, size.cols), (30, 100));
    assert_eq!(requested_pty_size(24, 80, 24, 81).unwrap().cols, 81);
    assert_eq!(requested_pty_size(24, 80, 25, 80).unwrap().rows, 25);
}

#[test]
fn pty_resize_clamps_out_of_range_values() {
    // Same 2-row floor as `start` — ConPTY misbehaves on 1-row geometry.
    let size = requested_pty_size(24, 80, 0, 500_000).unwrap();
    assert_eq!((size.rows, size.cols), (2, u16::MAX));
    // Clamped onto the current size: nothing reaches the PTY.
    assert_eq!(requested_pty_size(2, u16::MAX, 0, 500_000), None);
}

fn gate_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[test]
fn startup_resize_gate_passes_through_on_unix() {
    // Unix PTYs do not repaint on resize (shells reflow themselves on
    // SIGWINCH), so the gate must never engage there.
    let mut gate = StartupResizeGate::new(Platform::MacOS);
    gate.on_output(Duration::from_millis(100));
    assert_eq!(
        gate.resize(gate_size(30, 100), Duration::from_millis(150)),
        StartupResizeDecision::Apply
    );
    assert_eq!(gate.next_wake(), None);
}

#[test]
fn startup_resize_gate_applies_before_first_output() {
    // A resize racing ahead of the banner lands on an empty screen —
    // ConPTY's repaint is trivially safe, so apply immediately.
    let mut gate = StartupResizeGate::new(Platform::Windows);
    assert_eq!(
        gate.resize(gate_size(30, 100), Duration::from_millis(100)),
        StartupResizeDecision::Apply
    );
    assert_eq!(gate.pending(), None);
}

#[test]
fn startup_resize_gate_defers_while_banner_in_flight() {
    let mut gate = StartupResizeGate::new(Platform::Windows);
    gate.on_output(Duration::from_millis(200));
    assert_eq!(
        gate.resize(gate_size(30, 100), Duration::from_millis(400)),
        StartupResizeDecision::Defer
    );
    // A second geometry wins: only the latest viewport is worth applying.
    assert_eq!(
        gate.resize(gate_size(32, 120), Duration::from_millis(450)),
        StartupResizeDecision::Defer
    );
    assert_eq!(gate.pending(), Some(gate_size(32, 120)));
}

#[test]
fn startup_resize_gate_next_wake_tracks_last_output() {
    let mut gate = StartupResizeGate::new(Platform::Windows);
    assert_eq!(gate.next_wake(), None);
    gate.on_output(Duration::from_millis(200));
    gate.resize(gate_size(30, 100), Duration::from_millis(400));
    // Fresh output while deferred pushes the quiet point out with it.
    gate.on_output(Duration::from_millis(600));
    assert_eq!(
        gate.next_wake(),
        Some(Duration::from_millis(600) + STARTUP_RESIZE_QUIET)
    );
    // ...and the startup window caps how long the flush can wait.
    gate.on_output(Duration::from_millis(2_900));
    assert_eq!(gate.next_wake(), Some(STARTUP_RESIZE_WINDOW));
}

#[test]
fn startup_resize_gate_flushes_after_quiet() {
    let mut gate = StartupResizeGate::new(Platform::Windows);
    gate.on_output(Duration::from_millis(200));
    gate.resize(gate_size(30, 100), Duration::from_millis(400));
    gate.resize(gate_size(32, 120), Duration::from_millis(450));
    // One tick before the quiet point: still held.
    let quiet = Duration::from_millis(200) + STARTUP_RESIZE_QUIET;
    assert_eq!(gate.poll(quiet - Duration::from_millis(1)), None);
    // Quiet reached: the merged geometry applies exactly once.
    assert_eq!(gate.poll(quiet), Some(gate_size(32, 120)));
    assert_eq!(gate.poll(quiet + Duration::from_millis(1)), None);
    assert_eq!(gate.pending(), None);
}

#[test]
fn startup_resize_gate_force_flushes_at_window_expiry() {
    // Output still trickling at the cap: flush anyway rather than hold a
    // real resize hostage to a chatty banner.
    let mut gate = StartupResizeGate::new(Platform::Windows);
    gate.on_output(Duration::from_millis(2_900));
    gate.resize(gate_size(30, 100), Duration::from_millis(2_950));
    assert_eq!(
        gate.poll(STARTUP_RESIZE_WINDOW - Duration::from_millis(1)),
        None
    );
    assert_eq!(gate.poll(STARTUP_RESIZE_WINDOW), Some(gate_size(30, 100)));
}

#[test]
fn startup_resize_gate_inert_after_window() {
    // Past the window the gate is gone: every resize applies at once.
    let mut gate = StartupResizeGate::new(Platform::Windows);
    gate.on_output(Duration::from_millis(2_900));
    gate.resize(gate_size(30, 100), Duration::from_millis(2_950));
    assert_eq!(
        gate.resize(
            gate_size(40, 140),
            STARTUP_RESIZE_WINDOW + Duration::from_millis(50)
        ),
        StartupResizeDecision::Apply
    );
    assert_eq!(gate.pending(), None);
    assert_eq!(gate.next_wake(), None);
}

#[test]
fn observe_local_recording_writes_stdout_events() {
    let dir = std::env::temp_dir().join(format!("dbx-local-rec-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let recorder = session_recording::SessionRecorder::start(
        &dir,
        "unit-local-rec",
        "local",
        "local",
        "unit-session",
        100,
        24,
    )
    .expect("start recorder");
    let slot: Arc<Mutex<Option<session_recording::SessionRecorder>>> =
        Arc::new(Mutex::new(Some(recorder)));
    observe_local_recording(&slot, b"hello ");
    observe_local_recording(&slot, b"world");
    let summary = slot
        .lock()
        .unwrap()
        .take()
        .expect("active recorder")
        .finish()
        .expect("finish recorder");
    assert_eq!(summary["events"], 2);
    // The two bursts must land as one `o` event each, in order — replay
    // re-serializes the terminal from exactly these bytes.
    let path = session_recording::cast_path(&dir, "unit-local-rec").expect("cast path");
    let text = std::fs::read_to_string(&path).expect("read cast");
    let events: Vec<&str> = text.lines().skip(1).collect();
    // 两条输出事件 + 停止时的时长标记。
    assert_eq!(events.len(), 3);
    assert!(events[0].contains("\"eventdata\":\"hello \""));
    assert!(events[1].contains("\"eventdata\":\"world\""));
    assert!(events[2].contains("\"eventtype\":\"m\""));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn launch_options_default_first_then_local_terminal_shell_group() {
    let runtime = LocalTerminalRuntime::new();
    let options = runtime.launch_options("zh-CN", "/bin/zsh", None);
    let entries = options["entries"].as_array().unwrap();
    // First entry: the default-shell launcher, carried without a `group`
    // so the host pins it top-level as the quickest action.
    assert_eq!(entries[0]["label"], "本地终端");
    assert_eq!(entries[0]["description"], "默认 Shell：/bin/zsh");
    assert_eq!(entries[0]["context"]["plugin"]["mode"], "local-terminal");
    assert!(entries[0]["group"].is_null());
    // Discovered shells share the localized group label — the host renders
    // them as one collapsible "本地SHELL" section right below the default.
    for entry in entries.iter().skip(1) {
        assert_eq!(entry["group"], "本地SHELL");
        assert_eq!(entry["context"]["plugin"]["mode"], "local-terminal");
        assert!(entry["context"]["plugin"]["shell"].as_str().is_some());
        assert!(!entry["label"].as_str().expect("label string").is_empty());
    }
}

#[test]
fn launch_options_fall_back_to_english_for_unknown_locale() {
    // Older hosts pass no locale at all (empty string) — same fallback.
    let runtime = LocalTerminalRuntime::new();
    let english = runtime.launch_options("ko", "/bin/bash", None);
    assert_eq!(english["entries"][0]["label"], "Local terminal");
    assert!(english["entries"][0]["group"].is_null());
    assert_eq!(
        runtime.launch_options("", "/bin/bash", None)["entries"][0]["description"],
        "Default shell: /bin/bash"
    );
}

#[test]
fn launch_options_configured_shell_collapses_the_discovery_group() {
    // The user picked a default shell: the picker has nothing left to
    // choose among, so only the single configured entry remains.
    let runtime = LocalTerminalRuntime::new();
    let options = runtime.launch_options("zh-CN", "/usr/bin/fish", Some("/usr/bin/fish"));
    let entries = options["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["label"], "本地终端");
    assert_eq!(entries[0]["description"], "默认 Shell：/usr/bin/fish");
    assert!(entries[0]["group"].is_null());
}

#[test]
fn resolve_default_shell_prefers_configured_preference() {
    let dir = std::env::temp_dir().join(format!("dbx-launch-shell-prefs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create prefs dir");
    std::fs::write(
        dir.join("preferences.json"),
        r#"{"prefs":{"localShell":"/usr/bin/fish"}}"#,
    )
    .expect("write prefs");
    assert_eq!(resolve_default_shell(&dir), "/usr/bin/fish");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn resolve_default_shell_falls_back_to_auto_detect_chain() {
    let dir = std::env::temp_dir().join(format!("dbx-launch-shell-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create empty prefs dir");
    // No preference file: the chain resolves to something (Directory
    // Services / $SHELL / platform default — never empty).
    assert!(!resolve_default_shell(&dir).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn macos_prefers_directory_services_shell_over_env() {
    let spec = pick_shell(
        None,
        Some("/bin/bash"),
        Some("/opt/homebrew/bin/fish"),
        Platform::MacOS,
    );
    assert_eq!(spec.program, "/opt/homebrew/bin/fish");
    assert_eq!(spec.kind, ShellKind::Fish);
}

#[test]
fn shell_env_is_used_when_directory_services_has_nothing() {
    let spec = pick_shell(None, Some("/usr/bin/fish"), None, Platform::Linux);
    assert_eq!(spec.program, "/usr/bin/fish");
    assert_eq!(spec.kind, ShellKind::Fish);
}

#[test]
fn login_reject_shells_fall_through_to_next_candidate() {
    let spec = pick_shell(
        None,
        Some("/bin/bash"),
        Some("/usr/sbin/nologin"),
        Platform::Linux,
    );
    assert_eq!(spec.program, "/bin/bash");
}

#[test]
fn platform_defaults_when_nothing_detected() {
    let macos = pick_shell(None, None, None, Platform::MacOS);
    assert_eq!(macos.program, "/bin/zsh");
    assert_eq!(macos.kind, ShellKind::Zsh);
    let linux = pick_shell(None, None, None, Platform::Linux);
    assert_eq!(linux.program, "/bin/bash");
    assert_eq!(linux.kind, ShellKind::Bash);
}

#[test]
fn windows_defaults_to_powershell_with_no_login_args() {
    let spec = pick_shell(None, None, None, Platform::Windows);
    assert_eq!(spec.program, "powershell.exe");
    assert_eq!(spec.kind, ShellKind::PowerShell);
    assert!(base_args(ShellKind::PowerShell, Platform::Windows).is_empty());
}

#[test]
fn unix_shells_start_as_login_shells() {
    assert_eq!(base_args(ShellKind::Zsh, Platform::MacOS), vec!["-l"]);
    assert_eq!(base_args(ShellKind::Bash, Platform::Linux), vec!["-l"]);
}

#[test]
fn requested_shell_overrides_detection_and_keeps_login_args() {
    let spec = pick_shell(Some("/bin/bash"), Some("/bin/zsh"), None, Platform::MacOS);
    assert_eq!(spec.program, "/bin/bash");
    assert_eq!(spec.kind, ShellKind::Bash);
}

#[test]
fn shell_kind_handles_windows_paths_exe_and_case() {
    assert_eq!(
        shell_kind_from_program(r"C:\Program Files\PowerShell\7\pwsh.EXE"),
        ShellKind::PowerShell
    );
    assert_eq!(
        shell_kind_from_program("/opt/homebrew/bin/Zsh"),
        ShellKind::Zsh
    );
    assert_eq!(shell_kind_from_program("/bin/bash"), ShellKind::Bash);
    assert_eq!(shell_kind_from_program("cmd.exe"), ShellKind::Cmd);
    assert_eq!(shell_kind_from_program("/usr/bin/ksh"), ShellKind::Other);
}

#[test]
fn zsh_injection_wraps_zdotdir_and_keeps_login() {
    let dir = Path::new("/tmp/si");
    let plan = injection_plan(ShellKind::Zsh, dir, Some("/Users/u/zdot")).expect("zsh plan");
    assert_eq!(plan.0, vec!["-l"]);
    assert!(plan
        .1
        .contains(&("ZDOTDIR".to_string(), "/tmp/si".to_string())));
    assert!(plan
        .1
        .contains(&("DBX_USER_ZDOTDIR".to_string(), "/Users/u/zdot".to_string())));
    // Without a pre-existing ZDOTDIR the wrapper falls back to HOME.
    let plain = injection_plan(ShellKind::Zsh, dir, None).expect("zsh plan");
    assert_eq!(plain.1.len(), 1);
}

#[test]
fn bash_injection_swaps_login_for_rcfile_wrapper() {
    let dir = Path::new("/tmp/si");
    let plan = injection_plan(ShellKind::Bash, dir, None).expect("bash plan");
    // 期望值经 PathBuf 拼接生成，分隔符断言在 Windows（\）上同样成立。
    assert_eq!(
        plan.0,
        vec![
            "--rcfile",
            dir.join("wrapper.bash").to_string_lossy().as_ref()
        ]
    );
    assert!(plan.1.is_empty());
}

#[test]
fn fish_and_powershell_quote_script_paths() {
    let dir = Path::new("/tmp/my si");
    let fish = injection_plan(ShellKind::Fish, dir, None).expect("fish plan");
    assert_eq!(fish.0[0], "-l");
    assert_eq!(fish.0[1], "-C");
    assert_eq!(
        fish.0[2],
        format!(
            "source '{}'",
            escape_posix_single_quoted(dir.join("integration.fish").to_string_lossy().as_ref())
        )
    );

    let pwsh = injection_plan(ShellKind::PowerShell, dir, None).expect("pwsh plan");
    assert_eq!(pwsh.0[0], "-NoExit");
    // Restricted-policy machines (Windows client default) must still be
    // able to run the temp-dir integration script.
    assert_eq!(pwsh.0[1], "-ExecutionPolicy");
    assert_eq!(pwsh.0[2], "Bypass");
    assert_eq!(pwsh.0[3], "-Command");
    assert_eq!(
        pwsh.0[4],
        format!(
            "& '{}'",
            escape_powershell_single_quoted(dir.join("integration.ps1").to_string_lossy().as_ref())
        )
    );
}

#[test]
fn cmd_and_unknown_shells_get_no_injection() {
    assert!(injection_plan(ShellKind::Cmd, Path::new("/tmp/si"), None).is_none());
    assert!(injection_plan(ShellKind::Other, Path::new("/tmp/si"), None).is_none());
}

#[test]
fn prepare_integration_disabled_falls_back_to_base_args_without_io() {
    let prepared = prepare_integration(ShellKind::Bash, false);
    // Windows 无登录参数概念（见 base_args），Unix 一律 -l 登录 shell。
    if cfg!(windows) {
        assert!(prepared.args.is_empty());
    } else {
        assert_eq!(prepared.args, vec!["-l"]);
    }
    assert!(prepared.env.is_empty());
    assert!(!prepared.injected);
}

#[test]
fn integration_scripts_emit_lifecycle_marks() {
    for script in [
        INTEGRATION_ZSH,
        INTEGRATION_BASH,
        INTEGRATION_FISH,
        INTEGRATION_PWSH,
    ] {
        assert!(script.contains("133;A"), "missing prompt mark");
        assert!(script.contains("133;D;"), "missing exit-code mark");
        assert!(script.contains("633;P;Cwd="), "missing cwd report");
    }
    // 633;E command line is emitted by the POSIX scripts; PowerShell
    // reports cwd over OSC 9;9 because OSC 7 is POSIX-path shaped.
    assert!(INTEGRATION_ZSH.contains("633;E;"));
    assert!(INTEGRATION_BASH.contains("633;E;"));
    assert!(INTEGRATION_PWSH.contains("9;9;"));
}

#[test]
fn wrappers_chain_user_startup_files_fail_safe() {
    // Only the rc-entry wrappers reference the integration directory;
    // the zprofile/zlogin chains just forward the user's own files.
    for wrapper in [WRAPPER_ZSHRC, WRAPPER_BASH] {
        assert!(wrapper.contains("{{INTEGRATION_DIR}}"));
    }
    // Guarded user-rc chaining: DBX_USER_ZDOTDIR, HOME fallback, -f tests.
    for zsh_wrapper in [WRAPPER_ZSHRC, WRAPPER_ZPROFILE, WRAPPER_ZLOGIN] {
        assert!(zsh_wrapper.contains("DBX_USER_ZDOTDIR"));
        assert!(zsh_wrapper.contains("$HOME"));
    }
    assert!(WRAPPER_BASH.contains("/etc/profile"));
    assert!(WRAPPER_BASH.contains("$HOME/.bash_profile"));
}

#[test]
fn render_template_substitutes_the_integration_dir() {
    let rendered = render_template("source '{{INTEGRATION_DIR}}/integration.zsh'", "/opt/x");
    assert_eq!(rendered, "source '/opt/x/integration.zsh'");
}

#[test]
fn shell_discovery_merges_user_shell_etc_shells_and_defaults() {
    // /etc/shells 注释与空行跳过；用户 shell 置顶；basename 大小写不敏感去重；
    // 不存在的候选被 exists 谓词过滤。
    let exists = |program: &str| {
        matches!(
            program,
            "/opt/homebrew/bin/fish" | "/bin/zsh" | "/bin/bash" | "/usr/bin/fish"
        )
    };
    let merged = merge_shell_candidates(
        Some("/opt/homebrew/bin/fish"),
        &[
            "# comment".to_string(),
            "".to_string(),
            "/bin/zsh".to_string(),
            "/opt/homebrew/bin/FISH".to_string(),
            "/usr/bin/fish".to_string(),
            "/bin/bash".to_string(),
        ],
        &["/bin/zsh", "/bin/bash"],
        &exists,
    );
    // "/opt/homebrew/bin/FISH" 与 "/usr/bin/fish" 都是 basename fish，
    // 大小写不敏感去重后只保留先到的用户 shell。
    assert_eq!(
        merged,
        vec!["/opt/homebrew/bin/fish", "/bin/zsh", "/bin/bash"]
    );
}

#[test]
fn shell_discovery_survives_missing_user_shell_and_empty_etc_shells() {
    let merged = merge_shell_candidates(None, &[], &["/bin/bash"], &|p| p == "/bin/bash");
    assert_eq!(merged, vec!["/bin/bash"]);
}

#[test]
fn shell_discovery_skips_login_unusable_entries() {
    // /etc/shells 常见 /sbin/nologin 与 /bin/false：文件存在但不能登录，
    // 列进选择器就是"点了就闪退"的启动项，必须与 pick_shell 同规则过滤。
    let exists = |program: &str| {
        matches!(
            program,
            "/bin/bash" | "/sbin/nologin" | "/usr/sbin/false" | "/bin/zsh"
        )
    };
    let merged = merge_shell_candidates(
        None,
        &[
            "/bin/bash".to_string(),
            "/sbin/nologin".to_string(),
            "/usr/sbin/false".to_string(),
            "/bin/zsh".to_string(),
        ],
        &[],
        &exists,
    );
    assert_eq!(merged, vec!["/bin/bash", "/bin/zsh"]);
}

#[test]
fn discovery_marks_injectable_kinds() {
    let platform = if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    };
    let shells = discover_shells(platform);
    assert!(!shells.is_empty());
    for entry in &shells {
        let kind = shell_kind_from_program(&entry.program);
        let expected = !matches!(kind, ShellKind::Cmd | ShellKind::Other);
        assert_eq!(entry.injectable, expected, "entry {}", entry.program);
    }
}

#[test]
fn display_names_read_naturally() {
    assert_eq!(shell_display_name("zsh"), "Zsh");
    assert_eq!(shell_display_name("powershell"), "Windows PowerShell");
    assert_eq!(shell_display_name("pwsh"), "PowerShell");
    assert_eq!(shell_display_name("wsl"), "WSL");
}

#[test]
fn quoting_survives_embedded_single_quotes() {
    assert_eq!(
        escape_posix_single_quoted("/tmp/o'brien/si"),
        "/tmp/o'\\''brien/si"
    );
    assert_eq!(
        escape_powershell_single_quoted("C:\\o'brien"),
        "C:\\o''brien"
    );
}

#[test]
fn write_integration_files_materializes_every_script() {
    let dir = std::env::temp_dir().join(format!("dbx-si-test-{}", uuid::Uuid::new_v4()));
    write_integration_files(&dir).expect("write files");
    for name in [
        "integration.zsh",
        "integration.bash",
        "integration.fish",
        "integration.ps1",
        ".zshrc",
        ".zprofile",
        ".zlogin",
        "wrapper.bash",
    ] {
        assert!(dir.join(name).is_file(), "missing {name}");
    }
    let rendered = std::fs::read_to_string(dir.join(".zshrc")).expect("read wrapper");
    assert!(rendered.contains(dir.to_string_lossy().as_ref()));
    assert!(!rendered.contains("{{INTEGRATION_DIR}}"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn terminal_frame_encode_roundtrip() {
    let frame = TerminalFrame {
        sequence: 42,
        stream: TerminalStream::Stdout,
        data: b"hello".to_vec(),
    };
    let encoded = frame.encode();
    assert_eq!(encoded[0], 0);
    assert_eq!(&encoded[1..9], &42u64.to_be_bytes());
    assert_eq!(&encoded[9..], b"hello");

    let state = TerminalFrame {
        sequence: 7,
        stream: TerminalStream::State,
        data: b"local-terminal-exited".to_vec(),
    };
    assert_eq!(state.encode()[0], 2);
}
