//! SSH support.

use super::*;

pub struct SshClient {
    pub(crate) verifier: Arc<HostKeyVerifier>,
    pub(crate) prompts: PromptBroker,
    /// Event emitter; `None` in MCP stdio mode where stdout belongs to the
    /// JSON-RPC protocol and unknown host keys are trusted on first use.
    pub(crate) emitter: Option<PluginEmitter>,
    pub(crate) auto_trust: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) connection_id: String,
    pub(crate) operation_id: String,
    pub(crate) dial_deadline: Arc<DialDeadline>,
    pub(crate) connect_timeout: Duration,
    /// Remote (-R) port mappings of this connection: `(listen_host,
    /// bound_port) -> local dial target`. Populated by `ssh/forward/start`
    /// and consulted by the forwarded-tcpip handler below; per-connection so
    /// two servers can forward the same port independently.
    pub(crate) remote_forwards: Arc<RemoteForwardTable>,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self
            .verifier
            .check(&self.host, self.port, server_public_key)
        {
            Ok(HostKeyState::Trusted) => {
                eprintln!(
                    "[ssh-trace] host-key trusted {}:{}, fingerprint ok",
                    self.host, self.port
                );
                Ok(true)
            }
            Ok(HostKeyState::Unknown) => {
                eprintln!(
                    "[ssh-trace] host-key unknown {}:{} (auto_trust={})",
                    self.host, self.port, self.auto_trust
                );
                if self.auto_trust {
                    // Trust-on-first-use for MCP mode: keys land in the same
                    // known_hosts store and changes are still rejected.
                    if let Err(error) =
                        self.verifier
                            .learn(&self.host, self.port, server_public_key)
                    {
                        eprintln!("[dbx-plugin-sshx] failed to record host key: {error}");
                    }
                    return Ok(true);
                }
                let Some(emitter) = self.emitter.as_ref() else {
                    eprintln!("[ssh-trace] host-key unknown and no emitter -> reject");
                    return Ok(false);
                };
                // The user may take arbitrarily long to confirm the
                // fingerprint; that wait must not consume the dial timeout.
                self.dial_deadline
                    .enter_challenge(HOST_KEY_CHALLENGE_WAIT + self.connect_timeout);
                eprintln!("[ssh-trace] host-key challenge raised, waiting for user decision");
                let decision = self
                    .prompts
                    .request(
                        &self.host,
                        self.port,
                        server_public_key.algorithm().to_string(),
                        server_public_key.fingerprint(HashAlg::Sha256).to_string(),
                        &self.connection_id,
                        &self.operation_id,
                        emitter,
                    )
                    .await;
                self.dial_deadline.exit_challenge(self.connect_timeout);
                eprintln!(
                    "[ssh-trace] host-key challenge resolved: present={} accept={:?} remember={:?}",
                    decision.is_some(),
                    decision.as_ref().map(|d| d.accept),
                    decision.as_ref().map(|d| d.remember)
                );
                let Some(decision) = decision else {
                    return Ok(false);
                };
                if !decision.accept {
                    return Ok(false);
                }
                if decision.remember {
                    if let Err(error) =
                        self.verifier
                            .learn(&self.host, self.port, server_public_key)
                    {
                        let _ = emitter.event(
                            "ssh/host-key/notice",
                            json!({ "kind": "learn-failed", "message": error.to_string() }),
                        );
                    }
                }
                Ok(true)
            }
            Err(error) => {
                eprintln!(
                    "[ssh-trace] host-key CHANGED for {}:{}: {error}",
                    self.host, self.port
                );
                if let Some(emitter) = self.emitter.as_ref() {
                    let _ = emitter.event(
                        "ssh/host-key/notice",
                        json!({ "kind": "changed", "message": error.to_string() }),
                    );
                }
                Err(russh::Error::from(error))
            }
        }
    }

    /// Incoming `forwarded-tcpip` channel: the server accepted a connection
    /// on a port this side registered with `tcpip-forward` (remote -R
    /// mapping). The local dial target is looked up in this connection's
    /// remote-forward table; the relay runs detached so the handler returns
    /// immediately and the SSH reader is never blocked by user traffic.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<russh::client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let _ = originator_address;
        let _ = originator_port;
        let _ = session;
        let target =
            forward::lookup_remote_target(&self.remote_forwards, connected_address, connected_port);
        let Some(target) = target else {
            eprintln!(
                "[ssh-forward] forwarded-tcpip {connected_address}:{connected_port} has no registered mapping; rejecting"
            );
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        reply.accept().await;
        // Dial from the client machine; on failure the channel is dropped,
        // which the server surfaces as a closed connection to its client.
        match TcpStream::connect((target.host.as_str(), target.port)).await {
            Ok(tcp) => {
                // 计数与句柄登记走 forward 侧统一入口：relay 进 entry.relays，
                // stop/会话关闭才能中止它（此前 detach 的 relay 会躲过 abort，
                // 挂死连接一直存活到通道自然终结）。
                forward::spawn_remote_relay(target.entry, channel, tcp);
            }
            Err(error) => {
                eprintln!(
                    "[ssh-forward] local dial {}:{} failed: {error}",
                    target.host, target.port
                );
            }
        }
        Ok(())
    }

    /// Incoming `x11` channel: the server accepted the `x11-req` sent when
    /// the shell opened and a remote X client connected. russh's default
    /// handler accepts unconditionally, so this override is the fail-closed
    /// boundary (docs/plans/SPIKE_X11_FORWARDING.zh-CN.md §3.4): the armed-session
    /// gate must admit the channel and the X setup block must carry the
    /// issued fake cookie before a single byte is relayed to the local
    /// display. SSH delivers channel data only after the open is confirmed,
    /// so the channel is accepted first and then held (nothing relayed)
    /// until the setup block validates; a mismatch closes it immediately.
    async fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<russh::client::Msg>,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let _ = (originator_address, originator_port, session);
        // Admission first: no armed gate (X11 off or the last session
        // closed) or the armed gate's bridge cap exhausted -> reject, never
        // accept.
        let Some((cookie, permit)) = crate::features::forwarding::x11::try_admit_active() else {
            eprintln!("[x11] channel refused: no armed gate or the bridge cap is exhausted");
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        reply.accept().await;
        // The relay runs detached so the SSH reader is never blocked by X
        // traffic; the admission permit travels with the task and frees its
        // cap slot whenever the task ends.
        tokio::spawn(async move {
            // Keeping the admission permit alive for the bridge's lifetime
            // holds the cap slot; its Drop frees the slot whenever this
            // task ends (bridge finished, setup rejected, or channel gone).
            let _bridge_permit = permit;
            let mut channel = channel;
            let mut buffer = crate::features::forwarding::x11::SetupBuffer::default();
            let verdict = loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => match buffer.push(&data, &cookie) {
                        crate::features::forwarding::x11::SetupInspection::Incomplete => continue,
                        verdict => break verdict,
                    },
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                        break crate::features::forwarding::x11::SetupInspection::Reject(
                            "x11 channel closed before the X setup block arrived",
                        );
                    }
                    Some(_) => continue,
                }
            };
            match verdict {
                crate::features::forwarding::x11::SetupInspection::Accept { cookie_offset } => {
                    let mut setup = buffer.into_bytes();
                    let target = crate::features::forwarding::x11::current_display_target();
                    crate::features::forwarding::x11::substitute_cookie(
                        &mut setup,
                        cookie_offset,
                        crate::features::forwarding::x11::load_real_cookie(&target).as_deref(),
                    );
                    if let Err(error) =
                        crate::features::forwarding::x11::bridge_channel(channel, &target, setup)
                            .await
                    {
                        eprintln!("[x11] {error}");
                    }
                }
                crate::features::forwarding::x11::SetupInspection::Reject(reason) => {
                    eprintln!("[x11] x11 channel rejected: {reason}");
                    let _ = channel.close().await;
                }
                crate::features::forwarding::x11::SetupInspection::Incomplete => {
                    unreachable!("the setup loop only breaks on a final verdict")
                }
            }
        });
        Ok(())
    }
}

/// Verdict of a key-exchange-only host-key probe against the known_hosts
/// stores.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HostKeyVerdict {
    Trusted,
    Unknown,
    /// A recorded key differs from the presented one; carries the
    /// verifier's explanation (possible man-in-the-middle).
    Changed(String),
}

pub(crate) fn host_key_verdict(check: Result<HostKeyState, std::io::Error>) -> HostKeyVerdict {
    match check {
        Ok(HostKeyState::Trusted) => HostKeyVerdict::Trusted,
        Ok(HostKeyState::Unknown) => HostKeyVerdict::Unknown,
        Err(error) => HostKeyVerdict::Changed(error.to_string()),
    }
}

/// Response payload for `ssh/host-key/check`. Split from the probe path so
/// the state mapping is unit-testable without a server.
pub(crate) fn host_key_check_response(
    verdict: &HostKeyVerdict,
    key_type: &str,
    fingerprint: &str,
) -> Value {
    let state = match verdict {
        HostKeyVerdict::Trusted => "trusted",
        HostKeyVerdict::Unknown => "unknown",
        HostKeyVerdict::Changed(_) => "changed",
    };
    json!({
        "state": state,
        "keyType": key_type,
        "fingerprint": fingerprint,
    })
}

pub(crate) fn host_key_unreachable_response(error: String) -> Value {
    json!({ "state": "unreachable", "error": error })
}

/// Host-side RPC deadline fallback for `connection/test` when the stored
/// connection omits `connect_timeout_secs`: the host materializes 0/absent
/// via dbx-core `default_connect_timeout_secs()` = 10s
/// (crates/dbx-core/src/models/connection.rs:501) — NOT this plugin's
/// manifest default of 30s. Keep in sync with the host.
pub(crate) const HOST_FALLBACK_CONNECT_TIMEOUT_SECS: u64 = 10;

/// `connection/test` dial budget: the host kills the RPC at the effective
/// connect timeout, so the sidecar must answer one second earlier (the
/// margin covers stdio write→parse→dispatch latency), floored at 1s. An
/// explicit `connect_timeout_secs` is the deadline itself; an absent one
/// means the host falls back to its own 10s default.
pub(crate) fn test_dial_budget_secs(connect_timeout_secs: u64, explicit: bool) -> u64 {
    let deadline = if explicit {
        connect_timeout_secs.max(1)
    } else {
        HOST_FALLBACK_CONNECT_TIMEOUT_SECS
    };
    deadline.saturating_sub(1).max(1)
}

/// Re-arm decision for the `connection/test` budget once it elapsed: a host
/// dialog keeps the probe parked past the mirrored dial budget, so when the
/// dialog path is serving the challenge the budget extends once by the
/// challenge wait plus the connect timeout. Every other timeout (1.0 hosts,
/// workbench fallback, already-extended window) returns `None` and keeps the
/// readable error. Callers fold the already-extended state into
/// `dialog_used` so the extension can only arm a single time.
pub(crate) fn next_test_budget(connect_timeout_secs: u64, dialog_used: bool) -> Option<u64> {
    dialog_used.then(|| {
        HOST_KEY_CHALLENGE_WAIT
            .as_secs()
            .saturating_add(connect_timeout_secs)
    })
}

/// Actionable `connection/test` timeout: the cryptic host RPC-timeout
/// message gave no remedy; this names the effective budget (flagged as the
/// host default when the field was absent) plus the user-side fix. When a
/// host-key confirmation was raised during the probe, the challenge note is
/// appended in addition to the timeout-setting remedy — and only when the
/// dialog path did not serve the challenge, so the "update DBX" advice stays
/// truthful for 1.0 hosts.
pub(crate) fn test_timeout_message(
    host: &str,
    port: u16,
    budget_secs: u64,
    host_default: bool,
    challenge_raised: bool,
) -> String {
    let source = if host_default { " (host default)" } else { "" };
    let mut message = format!(
        "SSH connection to {host}:{port} timed out after {budget_secs} seconds{source}. Increase 'SSH timeout' under Advanced options and retry."
    );
    if challenge_raised {
        message.push_str(
            " A host-key confirmation was raised but went unanswered; connect once from the SSH workbench to trust this host, or update DBX to 0.6.17+ so the confirmation can appear here.",
        );
    }
    message
}

/// Key-exchange-only probe handler (tiny-rdm's CheckHostKey equivalent):
/// the server key is fingerprinted and compared against the known_hosts
/// stores, then the handshake is aborted with `Ok(false)` so no
/// authentication, challenge, or session ever happens. Deliberately not
/// `SshClient`, whose check_server_key would raise a host-key challenge.
pub(crate) struct HostKeyProbe {
    pub(crate) verifier: Arc<HostKeyVerifier>,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) seen: Arc<Mutex<Option<(HostKeyVerdict, String, String)>>>,
}

impl client::Handler for HostKeyProbe {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let verdict = host_key_verdict(self.verifier.check(
            &self.host,
            self.port,
            server_public_key,
        ));
        if let Ok(mut slot) = self.seen.lock() {
            *slot = Some((
                verdict,
                server_public_key.algorithm().to_string(),
                server_public_key.fingerprint(HashAlg::Sha256).to_string(),
            ));
        }
        // Abort before authentication; russh surfaces this as
        // Error::UnknownKey and tears the transport down on drop.
        Ok(false)
    }
}
