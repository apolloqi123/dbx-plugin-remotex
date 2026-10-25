use super::*;

// The OTP usage/committed ledgers are process-global (MCP exec calls
// resolve a fresh `SudoAuth` per call, so that is the point under
// test). Each test holds the ledger guard so parallel tests neither
// share marks nor wipe each other mid-run.

#[test]
fn parses_multiple_totp_secrets_across_separators() {
    let secrets = parse_totp_secrets("JBSWY3DPEHPK3PXP\nGEZDGNBVGY3TQOJQ; 123456\r\n");
    assert_eq!(secrets.len(), 3);
    assert!(parse_totp_secrets("  \n; ").is_empty());
}

#[test]
fn rotating_totp_avoids_reusing_the_same_code() {
    let _otp_ledger = otp_ledger_test_guard();
    let auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("JBSWY3DPEHPK3PXP\nGEZDGNBVGY3TQOJQ"),
        ..Default::default()
    };
    let first = auth.current_totp_selection().expect("code").code;
    let second = auth.current_totp_selection().expect("code").code;
    assert_ne!(
        first, second,
        "same-window second answer must use the other secret"
    );
    // Once every candidate is used we fall back instead of failing.
    let third = auth.current_totp_selection().expect("code").code;
    assert!(third == first || third == second);
}

/// Two `ssh_exec_sudo` calls never share a `SudoAuth` instance (each
/// call resolves a fresh one), so rotation state must live in the
/// process-global ledger: the second call inside the same window has to
/// switch to another secret's unused code instead of resubmitting the
/// first secret's already-used one, and once every code is used (and
/// its replay window is open) the guard skips instead of replaying.
#[test]
fn otp_rotation_spans_separate_instances() {
    let _otp_ledger = otp_ledger_test_guard();
    let make_call = || SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("MFRGGZDFMZTWQ2LK\nNBSWY3DPEBLWCZ4A"),
        ..Default::default()
    };
    let first = make_call().take_totp_answer().expect("first call");
    let second = make_call()
        .take_totp_answer()
        .expect("second call must rotate to an unused secret");
    assert_ne!(first, second, "a used code must not be resubmitted");
    let error = make_call()
        .take_totp_answer()
        .expect_err("no unused code is left inside the replay window");
    assert!(error.contains("already submitted"), "{error}");
}

/// The committed-replay guard must also span instances: a second exec
/// call in the same window sees the first call's submission instead of
/// silently resubmitting the same code.
#[test]
fn committed_replay_guard_spans_separate_instances() {
    let _otp_ledger = otp_ledger_test_guard();
    let make_call = || SudoAuth {
        totp_secrets: parse_totp_secrets("778899"),
        ..Default::default()
    };
    assert!(make_call().take_totp_answer().is_ok());
    let error = make_call()
        .take_totp_answer()
        .expect_err("the second call must see the earlier submission");
    assert!(error.contains("already submitted"), "{error}");
}

/// Static codes have no aligned time window, so their usage marks must
/// not be keyed on a per-call `now + window` timestamp: two calls two
/// seconds apart still have to rotate between the two static secrets.
#[test]
fn static_code_usage_keys_do_not_drift_across_calls() {
    let _otp_ledger = otp_ledger_test_guard();
    let make_call = || SudoAuth {
        totp_secrets: parse_totp_secrets("111111\n222222"),
        ..Default::default()
    };
    let first = make_call().take_totp_answer().expect("first call");
    let second = make_call()
        .take_totp_answer()
        .expect("second call must rotate to the other static code");
    assert_ne!(first, second);
}

/// Two connections reusing one secret validate codes independently: the
/// usage/replay marks are scoped per target, so host B can still submit
/// in the same window the code host A already burned.
#[test]
fn otp_ledger_marks_do_not_leak_across_targets() {
    let _otp_ledger = otp_ledger_test_guard();
    let make_call = |scope: &str| SudoAuth {
        totp_secrets: parse_totp_secrets("667788"),
        otp_ledger_scope: scope.into(),
        ..Default::default()
    };
    assert!(make_call("a@h1:22").take_totp_answer().is_ok());
    assert!(
        make_call("b@h2:22").take_totp_answer().is_ok(),
        "host B must not inherit host A's replay guard"
    );
    let error = make_call("a@h1:22")
        .take_totp_answer()
        .expect_err("the same target stays guarded");
    assert!(error.contains("already submitted"), "{error}");
}

#[test]
fn terminal_auto_sudo_reads_settings_updates_live() {
    let shared = Arc::new(RwLock::new(terminal_auth_for("pw1")));
    let mut auto = TerminalAutoSudo::new(shared.clone());
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw1".to_string()))
    );
    assert_eq!(auto.observe("user@host:~$ "), None);

    // Runtime settings update takes effect without recreating anything.
    shared
        .write()
        .unwrap_or_else(|poison| poison.into_inner())
        .password = "pw2".to_string();
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw2".to_string()))
    );
}

#[test]
fn sudo_auth_usefulness_drives_watcher_attach() {
    // No password and no TOTP: the connection cannot answer anything.
    assert!(!SudoAuth::default().useful());
    assert!(terminal_auth_for("").useful(), "TOTP alone is enough");
    assert!(terminal_auth_for("pw").useful());
}

/// A watcher never attached for a credential-less session (key-auth
/// connect, nothing configured yet) must start answering once the user
/// configures Quick Sudo at runtime — sync re-arms it from the updated
/// shared auth instead of staying detached for the session's lifetime.
#[test]
fn terminal_auto_sudo_arms_after_late_configuration() {
    let shared = Arc::new(RwLock::new(SudoAuth::default()));
    // Pre-configuration: sync sees a useless auth and keeps no watcher,
    // so nothing answers the prompt.
    assert!(!shared.read().unwrap().useful());
    let mut watcher: Option<TerminalAutoSudo> = None;
    assert_eq!(
        watcher
            .as_mut()
            .and_then(|auto| auto.observe("[sudo] password for user: ")),
        None
    );

    // The user saves a sudo password + hint while the terminal is open;
    // sync_auto_sudo re-arms the watcher from the updated shared auth.
    {
        let mut auth = shared.write().unwrap();
        auth.password = "late-pw".to_string();
        auth.password_prompt_hint = "password for".to_string();
    }
    assert!(shared.read().unwrap().useful());
    watcher = Some(TerminalAutoSudo::new(shared.clone()));
    assert_eq!(
        watcher
            .as_mut()
            .and_then(|auto| auto.observe("[sudo] password for user: ")),
        Some((AutoSudoKind::Password, "late-pw".to_string()))
    );
}

fn terminal_auth_for(password: &str) -> SudoAuth {
    SudoAuth {
        password: password.to_string(),
        totp_secrets: parse_totp_secrets("654321"),
        ..Default::default()
    }
}

/// User setEnv entries win over built-in defaults on duplicate keys and
/// every variable is requested exactly once (the merged list carries no
/// duplicates), so the sudo channel cannot see the internal
/// SUDO_ASKPASS clear clobber a user-configured value.
#[test]
fn merge_channel_env_user_entries_win_and_dedup() {
    let user = vec![
        ("LANG".to_string(), "C".to_string()),
        ("SUDO_ASKPASS".to_string(), "/tmp/askpass.sh".to_string()),
    ];
    let merged = merge_channel_env(&[("SUDO_ASKPASS", "")], &user);
    assert_eq!(
        merged,
        vec![
            ChannelEnv {
                key: "SUDO_ASKPASS".to_string(),
                value: "/tmp/askpass.sh".to_string(),
                strict: true,
            },
            ChannelEnv {
                key: "LANG".to_string(),
                value: "C".to_string(),
                strict: true,
            },
        ],
        "user value must replace the default and turn the entry strict"
    );
}

/// Without overlap the defaults keep their best-effort policy (default
/// sshd configs commonly refuse SUDO_ASKPASS) and user entries are
/// appended strict, in their configured order.
#[test]
fn merge_channel_env_keeps_defaults_lenient_and_user_strict() {
    let user = vec![("MY_APP_MODE".to_string(), "debug".to_string())];
    let merged = merge_channel_env(&[("SUDO_ASKPASS", "")], &user);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].key, "SUDO_ASKPASS");
    assert!(!merged[0].strict, "built-in defaults stay best-effort");
    assert_eq!(merged[1].key, "MY_APP_MODE");
    assert_eq!(merged[1].value, "debug");
    assert!(merged[1].strict, "user entries fail loudly when refused");
}

/// Empty inputs collapse to an empty request list: connections without
/// setEnv must not emit any env request at all.
#[test]
fn merge_channel_env_empty_inputs_stay_empty() {
    assert!(merge_channel_env(&[], &[]).is_empty());
    let user = vec![("A".to_string(), "1".to_string())];
    assert_eq!(merge_channel_env(&[], &user).len(), 1);
}

#[tokio::test]
async fn wait_cancelled_resolves_on_signal_but_stays_pending_without_one() {
    // 信号路径：send(true) 后立即解除。
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    tx.send(true).expect("send");
    tokio::time::timeout(Duration::from_millis(100), wait_cancelled(&mut rx))
        .await
        .expect("signal resolves the wait");
    // 无信号：保持挂起（超时即证明未误触发）。
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    tokio::time::timeout(Duration::from_millis(20), wait_cancelled(&mut rx))
        .await
        .expect_err("stays pending without a signal");
    drop(tx);
}

#[tokio::test]
async fn wait_cancelled_treats_dropped_sender_as_never() {
    // sender 被丢弃（任务已从注册表移除 / never_cancels 立即弃 sender）
    // = 永不取消：future 必须永久挂起。此前这里错误地断言「立即解除」
    // ——biased select 的取消臂因此抢占工作臂，never_cancels 系 exec
    // （metrics/processes/docker/completion）在真实会话上秒报 cancelled。
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    drop(tx);
    tokio::time::timeout(Duration::from_millis(100), wait_cancelled(&mut rx))
        .await
        .expect_err("dropped sender must park the future, never resolve as cancelled");
}
