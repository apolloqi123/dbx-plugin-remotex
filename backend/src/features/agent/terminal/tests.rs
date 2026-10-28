use super::*;

#[test]
fn off_mode_keeps_even_explicit_requests_on_the_hidden_channel() {
    // The connection setting is authoritative: an agent asking for
    // visibility cannot reopen a terminal the user switched off.
    assert!(!routes_to_terminal(AgentTerminalMode::Off, Some(true)));
    assert!(!routes_to_terminal(AgentTerminalMode::Off, None));
    assert!(!routes_to_terminal(AgentTerminalMode::Off, Some(false)));
    // auto/strict follow the flag, defaulting to visible when it is absent.
    assert!(routes_to_terminal(AgentTerminalMode::Auto, None));
    assert!(routes_to_terminal(AgentTerminalMode::Auto, Some(true)));
    assert!(!routes_to_terminal(AgentTerminalMode::Auto, Some(false)));
    assert!(routes_to_terminal(AgentTerminalMode::Strict, None));
    assert!(routes_to_terminal(AgentTerminalMode::Strict, Some(true)));
    assert!(!routes_to_terminal(AgentTerminalMode::Strict, Some(false)));
}

#[test]
fn decide_covers_the_full_policy_matrix() {
    // off matrix rows are retained for completeness, but exec routing
    // (`routes_to_terminal`) never sends an off connection here.
    assert_eq!(
        decide(AgentTerminalMode::Off, CommandRisk::Low, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide(AgentTerminalMode::Off, CommandRisk::Low, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide(AgentTerminalMode::Off, CommandRisk::Elevated, true),
        RoutingDecision::Prompt
    );
    assert!(matches!(
        decide(AgentTerminalMode::Off, CommandRisk::Elevated, false),
        RoutingDecision::Deny(_)
    ));
    // auto: low runs, elevated needs approval.
    assert_eq!(
        decide(AgentTerminalMode::Auto, CommandRisk::Low, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide(AgentTerminalMode::Auto, CommandRisk::Elevated, false),
        RoutingDecision::Prompt
    );
    // strict: everything needs approval.
    assert_eq!(
        decide(AgentTerminalMode::Strict, CommandRisk::Low, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide(AgentTerminalMode::Strict, CommandRisk::Elevated, false),
        RoutingDecision::Prompt
    );
}

#[test]
fn decide_with_memory_downgrades_only_prompt_to_run() {
    // Full matrix: 3 modes x 2 risks x explicit x remembered. A
    // remembered approval only ever turns a `Prompt` into a `Run`
    // (marked below); `Run` stays `Run` and `Deny` stays `Deny` (D1).
    // off / low: already running, remembering changes nothing.
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Low, false, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Low, false, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Low, true, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Low, true, true),
        RoutingDecision::Run
    );
    // off / elevated without explicit runInTerminal: mode-level denial is
    // never overridden by a remembered approval.
    assert!(matches!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Elevated, false, false),
        RoutingDecision::Deny(_)
    ));
    assert!(matches!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Elevated, false, true),
        RoutingDecision::Deny(_)
    ));
    // off / elevated with explicit runInTerminal: matrix-only rows (routing
    // keeps off connections hidden); remembered approval would replace the
    // per-command human prompt if routing ever reached them.
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Elevated, true, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Off, CommandRisk::Elevated, true, true),
        RoutingDecision::Run
    );
    // auto / low: already running.
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Low, false, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Low, false, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Low, true, false),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Low, true, true),
        RoutingDecision::Run
    );
    // auto / elevated: remembered approval skips the prompt.
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Elevated, false, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Elevated, false, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Elevated, true, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Auto, CommandRisk::Elevated, true, true),
        RoutingDecision::Run
    );
    // strict: every remembered command skips its approval prompt.
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Strict, CommandRisk::Low, false, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Strict, CommandRisk::Low, false, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Strict, CommandRisk::Low, true, false),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Strict, CommandRisk::Low, true, true),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(
            AgentTerminalMode::Strict,
            CommandRisk::Elevated,
            false,
            false
        ),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(
            AgentTerminalMode::Strict,
            CommandRisk::Elevated,
            false,
            true
        ),
        RoutingDecision::Run
    );
    assert_eq!(
        decide_with_memory(
            AgentTerminalMode::Strict,
            CommandRisk::Elevated,
            true,
            false
        ),
        RoutingDecision::Prompt
    );
    assert_eq!(
        decide_with_memory(AgentTerminalMode::Strict, CommandRisk::Elevated, true, true),
        RoutingDecision::Run
    );
}

#[test]
fn modes_round_trip_through_the_data_dir() {
    let dir = std::env::temp_dir().join(format!("dbx-agent-modes-{}", uuid::Uuid::new_v4()));
    let mut modes = HashMap::new();
    modes.insert("conn-a".to_string(), AgentTerminalMode::Auto);
    modes.insert("conn-b".to_string(), AgentTerminalMode::Strict);
    save_modes(&dir, &modes).expect("save");

    let loaded = load_modes(&dir);
    assert_eq!(loaded.get("conn-a"), Some(&AgentTerminalMode::Auto));
    assert_eq!(loaded.get("conn-b"), Some(&AgentTerminalMode::Strict));

    // A corrupted file degrades to an empty map instead of failing.
    std::fs::write(modes_path(&dir), "{ not json").unwrap();
    assert!(load_modes(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mode_parse_degrades_and_parse_exact_refuses_unknown() {
    assert_eq!(AgentTerminalMode::parse("auto"), AgentTerminalMode::Auto);
    assert_eq!(
        AgentTerminalMode::parse(" STRICT "),
        AgentTerminalMode::Strict
    );
    assert_eq!(AgentTerminalMode::parse("bogus"), AgentTerminalMode::Off);
    assert_eq!(AgentTerminalMode::parse(""), AgentTerminalMode::Off);

    assert_eq!(
        AgentTerminalMode::parse_exact("off"),
        Ok(AgentTerminalMode::Off)
    );
    assert!(AgentTerminalMode::parse_exact("Auto").is_ok());
    assert!(AgentTerminalMode::parse_exact("bogus").is_err());
    assert!(AgentTerminalMode::parse_exact("").is_err());
    let error = AgentTerminalMode::parse_exact("on").unwrap_err();
    assert!(error.contains("off"), "unexpected error: {error}");
}

#[test]
fn sanitize_strips_control_characters_and_keeps_newlines() {
    // \x03 (interrupt) and \x1b (escape) must never reach the terminal.
    assert_eq!(sanitize_command("echo\x03 hi").unwrap(), "echo hi");
    assert_eq!(sanitize_command("echo\x1b[31m hi").unwrap(), "echo[31m hi");
    // Whitespace controls that carry meaning survive.
    assert_eq!(sanitize_command("echo \n\thi").unwrap(), "echo \n\thi");
    // Surrounding whitespace is trimmed.
    assert_eq!(sanitize_command("  echo hi \n").unwrap(), "echo hi");
    // Empty (or control-only) commands are refused.
    assert!(sanitize_command("").is_err());
    assert!(sanitize_command("\x03\x1b \n\t ").is_err());
    assert!(sanitize_command("   ").is_err());
}

#[test]
fn strip_ansi_removes_csi_osc_and_plain_escapes() {
    assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m text"), "bold text");
    assert_eq!(strip_ansi("\x1b[2J\x1b[Hcleared"), "cleared");
    assert_eq!(
        strip_ansi("\x1b]0;window title\x07body"),
        "body",
        "OSC terminated by BEL"
    );
    assert_eq!(
        strip_ansi("\x1b]8;;http://x\x1b\\link"),
        "link",
        "OSC terminated by ST"
    );
    assert_eq!(strip_ansi("\x1b7saved"), "saved", "two-byte escape");
    assert_eq!(strip_ansi("plain text"), "plain text");
    assert_eq!(strip_ansi("trailing \x1b"), "trailing ", "dangling ESC");
    assert_eq!(strip_ansi("trailing \x1b[3"), "trailing ", "dangling CSI");
}

#[test]
fn recorder_settles_after_prompt_and_silence() {
    let mut recorder = TerminalRecorder::default();
    assert_eq!(recorder.observe("ignored"), RecorderState::Idle);

    recorder.arm("echo dbx-agent-marker");
    assert_eq!(
        recorder.observe("echo dbx-agent-marker\r\n"),
        RecorderState::Capturing
    );
    assert!(!recorder.is_settled(), "no prompt seen yet");
    assert_eq!(
        recorder.observe("out-a\r\nout-b\r\n"),
        RecorderState::Capturing
    );
    // The prompt may arrive in its own chunk (and split across chunks).
    recorder.observe("user@host");
    recorder.observe(":~$ ");
    assert!(!recorder.is_settled(), "silence debounce has not elapsed");
    std::thread::sleep(SILENCE_DEBOUNCE + Duration::from_millis(50));
    assert!(recorder.is_settled());

    let output = recorder.take_output("echo dbx-agent-marker");
    assert_eq!(
        output, "out-a\nout-b",
        "echo and trailing prompt are stripped"
    );
}

#[test]
fn recorder_finish_forces_the_capture_closed() {
    let mut recorder = TerminalRecorder::default();
    recorder.arm("never echoed");
    recorder.observe("partial output without any prompt");
    assert!(!recorder.is_settled());
    recorder.finish();
    assert_eq!(recorder.observe("straggler"), RecorderState::Settled);
    assert!(recorder.is_settled());
    assert_eq!(
        recorder.take_output("never echoed"),
        "partial output without any prompt"
    );
}

#[test]
fn recorder_buffer_stays_bounded_and_keeps_the_newest_output() {
    let mut recorder = TerminalRecorder::default();
    recorder.arm("echo not-present");
    // 1.5 MiB total: the head must be evicted, the tail must survive.
    let chunk = "x".repeat(4096);
    for _ in 0..384 {
        recorder.observe(&chunk);
    }
    let marker = "dbx-agent-tail-marker\n$ ";
    recorder.observe(marker);
    recorder.finish();
    let output = recorder.take_output("echo not-present");
    assert!(
        output.contains("dbx-agent-tail-marker"),
        "tail lost: {} bytes",
        output.len()
    );
    assert!(
        output.len() < 1024 * 1024 + 256,
        "buffer cap breached: {} bytes",
        output.len()
    );
}

#[test]
fn take_output_handles_multiline_echo_and_blank_padding() {
    let mut recorder = TerminalRecorder::default();
    recorder.arm("echo first");
    recorder.observe("root@host:/# echo first\r\nline1\r\n\r\nroot@host:/# ");
    assert_eq!(recorder.take_output("echo first"), "line1");
}

#[test]
fn sudo_command_text_never_duplicates_an_existing_prefix() {
    // Already prefixed: returned untouched (no `sudo sudo`).
    assert_eq!(sudo_command_text("sudo whoami"), "sudo whoami");
    assert_eq!(sudo_command_text("sudo -E ls -la"), "sudo -E ls -la");
    // Leading whitespace on the first line still counts as prefixed; the
    // original text passes through and `sanitize_command` trims it later.
    assert_eq!(sudo_command_text("  sudo id"), "  sudo id");
    // Bare `sudo` is a prefix too, not a command to wrap again.
    assert_eq!(sudo_command_text("sudo"), "sudo");
}

#[test]
fn sudo_command_text_prefixes_plain_commands_first_line_only() {
    assert_eq!(sudo_command_text("whoami"), "sudo whoami");
    // The prefix lands on the first line only; the remainder is left as
    // typed because multi-line input executes line by line (known
    // limitation of the injection path).
    assert_eq!(sudo_command_text("whoami\nid"), "sudo whoami\nid");
    // A `sudo` on a later line does not count as prefixed: the first
    // line decides, otherwise the intended command would be lost.
    assert_eq!(
        sudo_command_text("echo hi\nsudo id"),
        "sudo echo hi\nsudo id"
    );
    // Empty input is rejected upstream by `sanitize_command`; here it
    // degrades to a bare prefix instead of panicking.
    assert_eq!(sudo_command_text(""), "sudo ");
}
