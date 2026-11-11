//! Metrics support.

use super::*;

/// Accept loop for a local (-L) mapping: binds the port on the client
/// machine, then relays every accepted TCP connection through a
/// `direct-tcpip` channel. Binding happens before the task starts so the
/// mapping is only marked active once the port is actually usable.
pub(crate) async fn spawn_local_listener(
    entry: Arc<ForwardEntry>,
    handle: Arc<Handle<SshClient>>,
) -> Result<(), String> {
    let listener = TcpListener::bind((entry.listen_host.as_str(), entry.listen_port))
        .await
        .map_err(|error| {
            format!(
                "Cannot bind {}:{}: {error}",
                entry.listen_host, entry.listen_port
            )
        })?;
    let actual = listener.local_addr().map_err(|error| error.to_string())?;
    entry
        .bound_port
        .store(u32::from(actual.port()), Ordering::Relaxed);
    let task = tokio::spawn(accept_loop(entry.clone(), handle, listener));
    *entry
        .listener_task
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(task.abort_handle());
    Ok(())
}

pub(crate) async fn accept_loop(
    entry: Arc<ForwardEntry>,
    handle: Arc<Handle<SshClient>>,
    listener: TcpListener,
) {
    set_state(&entry, ForwardState::Active, None);
    loop {
        let accepted = listener.accept().await;
        match accepted {
            Ok((tcp, peer)) => {
                entry.connections_total.fetch_add(1, Ordering::Relaxed);
                entry.connections_active.fetch_add(1, Ordering::Relaxed);
                let relay = if entry.kind == ForwardKind::Dynamic {
                    tokio::spawn(relay_dynamic(entry.clone(), handle.clone(), tcp, peer))
                } else {
                    tokio::spawn(relay_local(entry.clone(), handle.clone(), tcp, peer))
                };
                if track_relay(&entry, relay) {
                    return;
                }
            }
            Err(error) => {
                set_state(
                    &entry,
                    ForwardState::Error,
                    Some(format!(
                        "Listener on {}:{} failed: {error}",
                        entry.listen_host,
                        bound_display(&entry)
                    )),
                );
                return;
            }
        }
    }
}

/// Relay one local mapping connection: client TCP stream <-> direct-tcpip
/// channel. The server dials `target_host:target_port` from its own network.
pub(crate) async fn relay_local(
    entry: Arc<ForwardEntry>,
    handle: Arc<Handle<SshClient>>,
    mut tcp: TcpStream,
    peer: SocketAddr,
) {
    // 拨号也要限时（与 SOCKS5 路径同一理由）：transport 半死（TCP 活着但不
    // 回包）时，无超时的 channel_open 会让客户端停在"连接中"，且
    // connections_active 虚高。
    let channel_result = tokio::time::timeout(
        CHANNEL_OPEN_TIMEOUT,
        handle.channel_open_direct_tcpip(
            entry.target_host.clone(),
            u32::from(entry.target_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        ),
    )
    .await;
    let mut channel = match channel_result {
        Ok(Ok(channel)) => channel.into_stream(),
        Ok(Err(error)) => {
            note_relay_end(&entry);
            eprintln!(
                "[ssh-forward] {} channel open failed: {error}",
                describe(
                    entry.kind,
                    &entry.listen_host,
                    bound_display(&entry),
                    &entry.target_host,
                    entry.target_port
                )
            );
            return;
        }
        Err(_) => {
            note_relay_end(&entry);
            eprintln!(
                "[ssh-forward] {} channel open timed out",
                describe(
                    entry.kind,
                    &entry.listen_host,
                    bound_display(&entry),
                    &entry.target_host,
                    entry.target_port
                )
            );
            return;
        }
    };
    let (up, down) = match copy_bidirectional_with_sizes(
        &mut tcp,
        &mut channel,
        FORWARD_COPY_BUFFER,
        FORWARD_COPY_BUFFER,
    )
    .await
    {
        Ok((up, down)) => (up, down),
        Err(error) => {
            eprintln!("[ssh-forward] relay ended with {error}");
            (0, 0)
        }
    };
    entry.bytes_up.fetch_add(up, Ordering::Relaxed);
    entry.bytes_down.fetch_add(down, Ordering::Relaxed);
    note_relay_end(&entry);
}

pub(crate) const SOCKS5_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// direct-tcpip 通道打开（拨号）限时：transport 半死（TCP 活着但不回包）时，
/// 无超时的 channel_open 会让客户端停在"连接中"，且 connections_active 虚高。
/// -L 与 SOCKS5(-D) 两条车道共用同一上限。
pub(crate) const CHANNEL_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Negotiate the SOCKS5 no-auth CONNECT subset. The host string is passed to
/// direct-tcpip unchanged so domain names are resolved by the SSH server.
pub(crate) async fn socks5_destination(tcp: &mut TcpStream) -> Result<(String, u16), String> {
    let mut greeting = [0u8; 2];
    tcp.read_exact(&mut greeting)
        .await
        .map_err(|e| e.to_string())?;
    if greeting[0] != 5 || greeting[1] == 0 {
        let _ = tcp.write_all(&[5, 0xff]).await;
        return Err("Invalid SOCKS5 greeting".to_string());
    }
    let mut methods = vec![0u8; usize::from(greeting[1])];
    tcp.read_exact(&mut methods)
        .await
        .map_err(|e| e.to_string())?;
    if !methods.contains(&0) {
        let _ = tcp.write_all(&[5, 0xff]).await;
        return Err("SOCKS5 client did not offer no-authentication".to_string());
    }
    tcp.write_all(&[5, 0]).await.map_err(|e| e.to_string())?;

    let mut request = [0u8; 4];
    tcp.read_exact(&mut request)
        .await
        .map_err(|e| e.to_string())?;
    if request[0] != 5 || request[2] != 0 {
        socks5_reply(tcp, 1).await;
        return Err("Invalid SOCKS5 request".to_string());
    }
    if request[1] != 1 {
        socks5_reply(tcp, 7).await;
        return Err("Only SOCKS5 CONNECT is supported".to_string());
    }
    let host = match request[3] {
        1 => {
            let mut ip = [0u8; 4];
            tcp.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let length = tcp.read_u8().await.map_err(|e| e.to_string())?;
            if length == 0 {
                socks5_reply(tcp, 8).await;
                return Err("Empty SOCKS5 domain".to_string());
            }
            let mut bytes = vec![0u8; usize::from(length)];
            tcp.read_exact(&mut bytes)
                .await
                .map_err(|e| e.to_string())?;
            match String::from_utf8(bytes) {
                Ok(host) if !host.contains('\0') => host,
                _ => {
                    socks5_reply(tcp, 8).await;
                    return Err("Invalid SOCKS5 domain".to_string());
                }
            }
        }
        4 => {
            let mut ip = [0u8; 16];
            tcp.read_exact(&mut ip).await.map_err(|e| e.to_string())?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => {
            socks5_reply(tcp, 8).await;
            return Err("Unsupported SOCKS5 address type".to_string());
        }
    };
    let port = tcp.read_u16().await.map_err(|e| e.to_string())?;
    if port == 0 {
        socks5_reply(tcp, 1).await;
        return Err("SOCKS5 target port must not be zero".to_string());
    }
    Ok((host, port))
}

pub(crate) async fn socks5_reply(tcp: &mut TcpStream, code: u8) {
    let _ = tcp.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await;
}

pub(crate) async fn relay_dynamic(
    entry: Arc<ForwardEntry>,
    handle: Arc<Handle<SshClient>>,
    mut tcp: TcpStream,
    peer: SocketAddr,
) {
    let destination =
        tokio::time::timeout(SOCKS5_HANDSHAKE_TIMEOUT, socks5_destination(&mut tcp)).await;
    let (host, port) = match destination {
        Ok(Ok(destination)) => destination,
        Ok(Err(error)) => {
            eprintln!("[ssh-forward] SOCKS5 handshake failed: {error}");
            note_relay_end(&entry);
            return;
        }
        Err(_) => {
            eprintln!("[ssh-forward] SOCKS5 handshake timed out");
            note_relay_end(&entry);
            return;
        }
    };
    // 拨号也要限时：transport 半死（TCP 活着但不回包）时，无超时的
    // channel_open 会让客户端停在"连接中"，且 connections_active 虚高。
    let channel = tokio::time::timeout(
        CHANNEL_OPEN_TIMEOUT,
        handle.channel_open_direct_tcpip(
            host,
            u32::from(port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        ),
    )
    .await;
    let mut channel = match channel {
        Ok(Ok(channel)) => channel.into_stream(),
        Ok(Err(error)) => {
            socks5_reply(&mut tcp, 5).await;
            eprintln!("[ssh-forward] SOCKS5 channel open failed: {error}");
            note_relay_end(&entry);
            return;
        }
        Err(_) => {
            // 6 = TTL expired：最能对上"上游打开超时"的 SOCKS5 回码。
            socks5_reply(&mut tcp, 6).await;
            eprintln!("[ssh-forward] SOCKS5 channel open timed out");
            note_relay_end(&entry);
            return;
        }
    };
    socks5_reply(&mut tcp, 0).await;
    match copy_bidirectional_with_sizes(
        &mut tcp,
        &mut channel,
        FORWARD_COPY_BUFFER,
        FORWARD_COPY_BUFFER,
    )
    .await
    {
        Ok((up, down)) => {
            entry.bytes_up.fetch_add(up, Ordering::Relaxed);
            entry.bytes_down.fetch_add(down, Ordering::Relaxed);
        }
        Err(error) => eprintln!("[ssh-forward] SOCKS5 relay ended with {error}"),
    }
    note_relay_end(&entry);
}

/// Relay one remote (-R) mapping connection handed over by the SSH client
/// handler: forwarded-tcpip channel <-> TCP stream dialed from the client
/// machine. Counters land on the same mapping row as local relays; they are
/// taken by [`spawn_remote_relay`] before the task starts, so an abort on the
/// stopping path can unwind them via `note_relay_end` exactly like the
/// accept loop's relays.
pub(crate) async fn relay_remote(
    entry: Arc<ForwardEntry>,
    channel: russh::Channel<russh::client::Msg>,
    mut tcp: TcpStream,
) {
    let mut channel = channel.into_stream();
    let (up, down) = match copy_bidirectional_with_sizes(
        &mut tcp,
        &mut channel,
        FORWARD_COPY_BUFFER,
        FORWARD_COPY_BUFFER,
    )
    .await
    {
        Ok((up, down)) => (up, down),
        Err(error) => {
            eprintln!("[ssh-forward] remote relay ended with {error}");
            (0, 0)
        }
    };
    entry.bytes_up.fetch_add(up, Ordering::Relaxed);
    entry.bytes_down.fetch_add(down, Ordering::Relaxed);
    note_relay_end(&entry);
}

/// -R 入站 relay 由 SSH 事件处理器直接 spawn（没有 accept_loop 外层）：
/// 计数先行、句柄登记复用 [`track_relay`]，保持与 accept_loop 同一关闭竞态
/// 语义（stop 已 drain 时自行 abort 并回冲计数）。
pub(crate) fn spawn_remote_relay(
    entry: Arc<ForwardEntry>,
    channel: russh::Channel<russh::client::Msg>,
    tcp: TcpStream,
) {
    entry.connections_total.fetch_add(1, Ordering::Relaxed);
    entry.connections_active.fetch_add(1, Ordering::Relaxed);
    let handle = tokio::spawn(relay_remote(entry.clone(), channel, tcp));
    track_relay(&entry, handle);
}

/// spawn 后登记 relay 句柄：锁内复查 stopping——stop 已 drain 过 relays 时，
/// 这个句柄再 push 也无人 abort，该连接会躲过停止继续搬运流量。锁内判定后
/// 自行 abort 并回冲计数（被 abort 的任务不会再走 note_relay_end）。
/// 返回 true 表示走了 stopping-abort 分支，调用方（accept_loop）据此终止
/// 接受循环，让监听口随停止语义尽早关闭。
pub(crate) fn track_relay(entry: &Arc<ForwardEntry>, relay: tokio::task::JoinHandle<()>) -> bool {
    retain_live_relays(entry);
    let mut relays = entry
        .relays
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if entry.stopping.load(Ordering::Relaxed) {
        drop(relays);
        relay.abort();
        note_relay_end(entry);
        return true;
    }
    relays.push(relay.abort_handle());
    false
}

pub(crate) fn note_relay_end(entry: &ForwardEntry) {
    // Saturating decrement: `abort_tasks` drains and aborts tracked relays
    // while their futures are suspended, so those relays never reach their own
    // `note_relay_end`. If a relay happened to finish just as the drain ran,
    // its own decrement and the drain's can both land. A plain `fetch_sub`
    // would then push the counter negative (and wrap on the next relay),
    // making `connectionsActive` nonsense. Clamp at zero instead.
    let _ = entry.connections_active.fetch_update(
        Ordering::Relaxed,
        Ordering::Relaxed,
        |active| (active > 0).then_some(active - 1),
    );
    retain_live_relays(entry);
}

pub(crate) fn retain_live_relays(entry: &ForwardEntry) {
    let mut relays = entry
        .relays
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    relays.retain(|handle| !handle.is_finished());
}
