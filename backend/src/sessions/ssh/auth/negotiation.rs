//! SSH auth support.

use super::*;

pub(crate) fn flow_mode_name(mode: AuthFlowMode) -> &'static str {
    mode.name()
}

pub(crate) fn method_offered(result: &AuthResult, kind: MethodKind) -> bool {
    match result {
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods.contains(&kind),
        _ => false,
    }
}

/// Extracts the server's method advertisement from a rejection. An empty set
/// carries no information — russh also produces one when the transport dies
/// mid-probe — and maps to `None` so callers fail open instead of gating on a
/// phantom restriction.
pub(crate) fn advertised_methods(result: &AuthResult) -> Option<&MethodSet> {
    match result {
        AuthResult::Failure {
            remaining_methods, ..
        } if !remaining_methods.is_empty() => Some(remaining_methods),
        _ => None,
    }
}

/// Merges the leading none probe's advertisement with the advertisement of
/// the immediately preceding auth step and decides whether `wanted` may still
/// be attempted (Tabby authMethodSelection parity, second phase of the
/// none-probe rollout): a method only stays on the menu while both
/// advertisements list it — the stricter of the two wins, so a server that
/// narrows its advertisement mid-negotiation can never be blind-tried into
/// MaxAuthTries. A missing none advertisement (abnormal shape, see
/// [`advertised_methods`]) fails open to the per-step gate.
pub(crate) fn reconcile_advertised(
    advertised_none: Option<&MethodSet>,
    step_offered: &AuthResult,
    wanted: MethodKind,
) -> bool {
    let none_still_offers = advertised_none.is_none_or(|set| set.contains(&wanted));
    none_still_offers && method_offered(step_offered, wanted)
}

/// Failure text for the "neither password nor keyboard-interactive" gate.
/// When the none probe's advertisement is known it is appended so the error
/// names what the server actually allows (publickey-only hosts, for one).
pub(crate) fn advertisement_rejection_message(advertised_none: Option<&MethodSet>) -> String {
    let message = "SSH server did not advertise password or keyboard-interactive authentication; refusing to send the password".to_string();
    match advertised_none {
        Some(set) if !set.is_empty() => {
            let names: Vec<&str> = set.iter().map(<&'static str>::from).collect();
            format!("{message} (server advertises: {})", names.join(", "))
        }
        _ => message,
    }
}

/// True when the server accepted the preceding step but demands another
/// authentication method before letting the session in — the shape koko (and
/// PAM 2FA stacks in general) uses for "password/publickey accepted, now send
/// the MFA code". It seeds the keyboard-interactive exchange whenever no
/// password was submitted first (publickey / agent / KI-only paths); the
/// password path seeds itself, since a submitted password already documents
/// that the server moved past the first factor.
pub(crate) fn auth_partial_success(result: &AuthResult) -> bool {
    match result {
        AuthResult::Failure {
            partial_success, ..
        } => *partial_success,
        _ => false,
    }
}

/// Fixed Auto try order — the public contract of the fallback chain. Method
/// names use the `external_config.authentication` spellings so log lines stay
/// greppable against connection configs.
pub(crate) const AUTO_AUTH_ORDER: [&str; 4] =
    ["password", "private-key", "keyboard-interactive", "agent"];

/// Pure stage gate for the Auto chain: `None` means "attempt the stage",
/// `Some(reason)` means "record a skipped attempt". Credential presence is
/// configuration (known before dialing); whether the server advertises the
/// password method is the runtime fact captured by the auth-none probe.
/// keyboard-interactive and agent have no local prerequisites — the user can
/// still answer prompts interactively and the agent socket comes from the
/// environment — so they are never skipped.
pub(crate) fn auto_stage_skip(
    method: &str,
    has_password: bool,
    password_offered: bool,
    has_key: bool,
) -> Option<String> {
    match method {
        "password" if !has_password => {
            Some("no password configured for this connection".to_string())
        }
        "password" if !password_offered => {
            Some("server does not advertise password authentication".to_string())
        }
        "private-key" if !has_key => {
            Some("no private key configured for this connection".to_string())
        }
        _ => None,
    }
}

/// One recorded Auto-fallback attempt: the method plus why it was skipped or
/// why it failed. The name must stay aligned with `AUTO_AUTH_ORDER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoAuthAttempt {
    pub(crate) method: &'static str,
    pub(crate) skipped: bool,
    pub(crate) detail: String,
}

pub(crate) fn auto_auth_attempt(method: &'static str, detail: String) -> AutoAuthAttempt {
    AutoAuthAttempt {
        method,
        skipped: false,
        detail,
    }
}

pub(crate) fn auto_auth_skip(method: &'static str, detail: String) -> AutoAuthAttempt {
    AutoAuthAttempt {
        method,
        skipped: true,
        detail,
    }
}

/// Merges one recorded Auto-stage outcome into the attempt list. The same
/// method can legitimately be recorded more than once: the private-key /
/// agent partial-success KI continuations plus the dedicated KI stage each
/// record `keyboard-interactive`. Repeats merge their detail instead of
/// pushing a duplicate entry, keeping the "one entry per method, in
/// first-try order" invariant that the aggregated failure message and the
/// order assertion below rely on.
pub(crate) fn merge_auto_auth_attempt(
    attempts: &mut Vec<AutoAuthAttempt>,
    attempt: AutoAuthAttempt,
) {
    if let Some(existing) = attempts
        .iter_mut()
        .find(|recorded| recorded.method == attempt.method)
    {
        if !attempt.skipped {
            existing.skipped = false;
        }
        existing.detail.push_str("; ");
        existing.detail.push_str(&attempt.detail);
    } else {
        attempts.push(attempt);
    }
}

/// Aggregated failure message for the Auto chain: every attempted (or
/// skipped) method with its reason, in the fixed try order, so a total
/// failure explains itself instead of surfacing only the last error.
pub(crate) fn auto_auth_failure_message(attempts: &[AutoAuthAttempt]) -> String {
    if attempts.is_empty() {
        return "SSH authentication failed in Auto mode: no method could be attempted".to_string();
    }
    let summary = attempts
        .iter()
        .map(|attempt| {
            if attempt.skipped {
                format!("{} (skipped: {})", attempt.method, attempt.detail)
            } else {
                format!("{} ({})", attempt.method, attempt.detail)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("SSH authentication failed in Auto mode, tried in order — {summary}")
}

/// Auto authentication, Tabby-style ordered fallback: password → private key
/// → interactive keyboard-interactive (incl. TOTP) → ssh-agent, in that fixed
/// order (`AUTO_AUTH_ORDER`), until one succeeds or every attempt failed.
///
/// Deliberately a thin orchestrator: every credential flow reuses the exact
/// helper of the explicit method it mirrors (`try_password`,
/// `authenticate_private_key_result`, `authenticate_keyboard_interactive`,
/// `authenticate_agent`), including the Quick Sudo OTP orchestration and the
/// host-key challenge path, which stay untouched under Auto. A private-key or
/// agent partial success continues into the same MFA keyboard-interactive
/// continuation as the explicit paths; that continuation counts as the KI
/// stage so a later failure does not ask the user the same questions twice.
///
/// Each non-successful stage is reported as a sidecar log event
/// (`ssh/auth/auto`, best-effort — skipped in MCP headless mode without an
/// emitter) and recorded for the aggregated failure message.
pub(crate) async fn authenticate_auto(
    session: &mut Handle<SshClient>,
    connection: &StoredConnection,
    orchestration: &SudoAuth,
    offered: &AuthResult,
    prompts: &PromptBroker,
    emitter: Option<&PluginEmitter>,
    operation_id: &str,
) -> Result<(), String> {
    let has_password = !connection.password.is_empty();
    let password_offered = method_offered(offered, MethodKind::Password);
    let has_key = !connection.private_key.is_empty() || !connection.private_key_path.is_empty();
    let mut attempts: Vec<AutoAuthAttempt> = Vec::new();
    // 私钥/agent 的 partial-success 续答就是 KI 阶段本身：再跑一轮全新 KI
    // 会把同样的提问（验证码）重复问一遍。
    let mut ki_attempted = false;
    let report = |attempt: &AutoAuthAttempt| {
        if let Some(emitter) = emitter {
            let _ = emitter.event(
                "ssh/auth/auto",
                json!({
                    "operationId": operation_id,
                    "connectionId": connection.id,
                    "method": attempt.method,
                    "status": if attempt.skipped { "skipped" } else { "failed" },
                    "detail": attempt.detail,
                }),
            );
        }
    };
    let mut record = |attempt: AutoAuthAttempt| {
        eprintln!(
            "[ssh-trace] auth auto: {} {} ({})",
            attempt.method,
            if attempt.skipped { "skipped" } else { "failed" },
            attempt.detail
        );
        report(&attempt);
        merge_auto_auth_attempt(&mut attempts, attempt);
    };

    // Stage 1: password.
    if let Some(reason) = auto_stage_skip("password", has_password, password_offered, has_key) {
        record(auto_auth_skip("password", reason));
    } else {
        match try_password(session, connection).await {
            Ok(result) if result.success() => return Ok(()),
            Ok(result) => record(auto_auth_attempt(
                "password",
                format!(
                    "rejected by the server (partial_success={})",
                    auth_partial_success(&result)
                ),
            )),
            Err(error) => record(auto_auth_attempt("password", error)),
        }
    }

    // Stage 2: private key.
    if let Some(reason) = auto_stage_skip("private-key", has_password, password_offered, has_key) {
        record(auto_auth_skip("private-key", reason));
    } else {
        match authenticate_private_key_result(session, connection).await {
            Ok(result) if result.success() => return Ok(()),
            Ok(result)
                if auth_partial_success(&result)
                    && method_offered(&result, MethodKind::KeyboardInteractive) =>
            {
                eprintln!(
                    "[ssh-trace] auth auto: publickey partial success, continuing with keyboard-interactive"
                );
                ki_attempted = true;
                match authenticate_keyboard_interactive(
                    session,
                    connection,
                    orchestration,
                    true,
                    prompts,
                )
                .await
                {
                    Ok(()) => return Ok(()),
                    Err(error) => record(auto_auth_attempt("keyboard-interactive", error)),
                }
            }
            Ok(_) => record(auto_auth_attempt(
                "private-key",
                "rejected by the server".to_string(),
            )),
            Err(error) => record(auto_auth_attempt("private-key", error)),
        }
    }

    // Stage 3: interactive keyboard-interactive (incl. TOTP).
    if ki_attempted {
        record(auto_auth_skip(
            "keyboard-interactive",
            "already answered as the MFA follow-up of an earlier stage".to_string(),
        ));
    } else {
        match authenticate_keyboard_interactive(session, connection, orchestration, false, prompts)
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) => record(auto_auth_attempt("keyboard-interactive", error)),
        }
    }

    // Stage 4: ssh-agent.
    match authenticate_agent(session, connection).await {
        Ok(AgentAuthOutcome::Accepted) => return Ok(()),
        Ok(AgentAuthOutcome::NeedsKeyboardInteractive) => {
            eprintln!(
                "[ssh-trace] auth auto: agent partial success, continuing with keyboard-interactive"
            );
            match authenticate_keyboard_interactive(
                session,
                connection,
                orchestration,
                true,
                prompts,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(error) => record(auto_auth_attempt("keyboard-interactive", error)),
            }
        }
        Ok(AgentAuthOutcome::Rejected(reason)) => record(auto_auth_attempt("agent", reason)),
        Err(error) => record(auto_auth_attempt("agent", error)),
    }

    // Order invariant: attempts (skips included) must appear in the fixed try
    // order of `AUTO_AUTH_ORDER` — filtering the contract list by the methods
    // actually attempted must reproduce the record exactly.
    debug_assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.method)
            .collect::<Vec<_>>(),
        AUTO_AUTH_ORDER
            .iter()
            .filter(|method| attempts.iter().any(|attempt| attempt.method == **method))
            .copied()
            .collect::<Vec<_>>(),
        "Auto authentication attempts must follow the fixed try order"
    );
    Err(auto_auth_failure_message(&attempts))
}
