use super::*;
use serde_json::json;

fn spec(params: Value) -> Result<(ForwardKind, String, u16, String, u16), String> {
    parse_spec(&params)
}

#[test]
fn parses_local_mapping_with_loopback_default() {
    let (kind, listen_host, listen_port, target_host, target_port) = spec(json!({
        "kind": "local", "listenPort": 8080, "targetHost": "db.internal", "targetPort": 5432
    }))
    .unwrap();
    assert_eq!(kind, ForwardKind::Local);
    assert_eq!(listen_host, "127.0.0.1");
    assert_eq!(listen_port, 8080);
    assert_eq!(target_host, "db.internal");
    assert_eq!(target_port, 5432);
}

#[test]
fn parses_remote_mapping_with_server_picked_port() {
    let (kind, listen_host, listen_port, _, _) = spec(json!({
        "kind": "remote", "listenHost": "0.0.0.0", "listenPort": 0,
        "targetHost": "127.0.0.1", "targetPort": 3000
    }))
    .unwrap();
    assert_eq!(kind, ForwardKind::Remote);
    assert_eq!(listen_host, "0.0.0.0");
    assert_eq!(listen_port, 0);
}

#[test]
fn rejects_unknown_kind_and_bad_ports() {
    let error =
        spec(json!({"kind": "unknown", "listenPort": 1, "targetHost": "h", "targetPort": 2}))
            .unwrap_err();
    assert!(error.contains("expected"), "unexpected: {error}");
    let error =
        spec(json!({"kind": "local", "listenPort": 70000, "targetHost": "h", "targetPort": 2}))
            .unwrap_err();
    assert!(error.contains("listenPort"), "unexpected: {error}");
    let error = spec(json!({"kind": "local", "listenPort": 80, "targetHost": "", "targetPort": 2}))
        .unwrap_err();
    assert!(error.contains("targetHost"), "unexpected: {error}");
    let error =
        spec(json!({"kind": "local", "listenPort": "x", "targetHost": "h", "targetPort": 2}))
            .unwrap_err();
    assert!(error.contains("listenPort"), "unexpected: {error}");
}

#[test]
fn dynamic_mapping_has_no_fixed_target() {
    let parsed = spec(json!({"kind": "dynamic", "listenPort": 0})).unwrap();
    assert_eq!(
        parsed,
        (
            ForwardKind::Dynamic,
            "127.0.0.1".to_string(),
            0,
            String::new(),
            0
        )
    );
    assert!(spec(
        json!({"kind": "dynamic", "listenPort": 1080, "targetHost": "web", "targetPort": 80})
    )
    .is_err());
    assert!(spec(json!({"kind": "dynamic", "listenHost": "*", "listenPort": 1080})).is_err());
}

#[tokio::test]
async fn socks5_connect_parses_ipv4_domain_and_ipv6() {
    for (address, expected) in [
        (vec![1, 10, 0, 0, 5], "10.0.0.5".to_string()),
        (
            vec![3, 7, b'i', b'n', b't', b'e', b'r', b'n', b'l'],
            "internl".to_string(),
        ),
        (
            {
                let mut bytes = vec![4];
                bytes.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
                bytes
            },
            "::1".to_string(),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            socks5_destination(&mut tcp).await.unwrap()
        });
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(&[5, 2, 2, 0]).await.unwrap();
        let mut reply = [0u8; 2];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [5, 0]);
        let mut request = vec![5, 1, 0];
        request.extend_from_slice(&address);
        request.extend_from_slice(&8080u16.to_be_bytes());
        client.write_all(&request).await.unwrap();
        assert_eq!(server.await.unwrap(), (expected, 8080));
    }
}

#[tokio::test]
async fn socks5_rejects_auth_only_and_udp_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        // Windows turns an immediate drop after the rejection write into a
        // mid-read RST (10054); linger briefly so the reply is delivered.
        let result = socks5_destination(&mut tcp).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        result
    });
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(&[5, 1, 2]).await.unwrap();
    let mut reply = [0u8; 2];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [5, 0xff]);
    assert!(server.await.unwrap().is_err());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        // Windows turns an immediate drop after the rejection write into a
        // mid-read RST (10054); linger briefly so the reply is delivered.
        let result = socks5_destination(&mut tcp).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        result
    });
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(&[5, 1, 0]).await.unwrap();
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [5, 0]);
    client
        .write_all(&[5, 3, 0, 1, 127, 0, 0, 1, 0, 80])
        .await
        .unwrap();
    let mut response = [0u8; 10];
    client.read_exact(&mut response).await.unwrap();
    assert_eq!(response[1], 7);
    assert!(server.await.unwrap().is_err());
}

#[test]
fn normalizes_host_whitespace_and_case() {
    assert_eq!(normalize_host("  DB-01 ").unwrap(), "db-01");
    assert_eq!(normalize_host("").unwrap(), "127.0.0.1");
    assert!(normalize_host("bad host").is_err());
}

#[test]
fn accepts_ipv4_ipv6_bracketed_and_hostnames() {
    // IPv4 (case/whitespace normalized).
    assert_eq!(normalize_host("192.168.1.10").unwrap(), "192.168.1.10");
    // IPv6 bare and URL-bracketed; brackets are stripped for the bind.
    assert_eq!(normalize_host("::1").unwrap(), "::1");
    assert_eq!(normalize_host("[FE80::1]").unwrap(), "fe80::1");
    // Plain hostname and loopback name.
    assert_eq!(normalize_host("db.Internal").unwrap(), "db.internal");
    assert_eq!(normalize_host("localhost").unwrap(), "localhost");
    // The listen wildcard stays verbatim (remote-side bind everywhere).
    assert_eq!(normalize_host("*").unwrap(), "*");
}

#[test]
fn rejects_scheme_port_and_malformed_hosts() {
    // Embedded port / scheme / userinfo never become garbage lookups.
    assert!(normalize_host("10.0.0.5:8080").is_err());
    assert!(normalize_host("http://10.0.0.5").is_err());
    assert!(normalize_host("user@db").is_err());
    // Broken bracket forms.
    assert!(normalize_host("[::1").is_err());
    assert!(normalize_host("[::1]:8080").is_err());
    // Malformed hostnames and IPs.
    assert!(normalize_host("-lead.ing").is_err());
    assert!(normalize_host("end-.ing").is_err());
    assert!(normalize_host("..").is_err());
    assert!(normalize_host("999.1.1.1").is_err());
    assert!(normalize_host("fe80::1%en0").is_err()); // zone scope: bind via the interface name instead
}

#[test]
fn local_kind_rejects_wildcard_star() {
    let error = spec(json!({
        "kind": "local", "listenHost": "*", "listenPort": 80,
        "targetHost": "h", "targetPort": 1
    }))
    .unwrap_err();
    assert!(error.contains("wildcard"), "unexpected: {error}");
    // Remote keeps the sshd "bind everywhere" spelling.
    let parsed = spec(json!({
        "kind": "remote", "listenHost": "*", "listenPort": 80,
        "targetHost": "h", "targetPort": 1
    }))
    .unwrap();
    assert_eq!(parsed.1, "*");
}

#[test]
fn listen_conflicts_need_same_port_and_overlapping_host() {
    use crate::features::forwarding::forward as fwd;
    // Same host, same port.
    assert!(fwd::listen_endpoints_conflict(
        "127.0.0.1",
        8080,
        "127.0.0.1",
        8080
    ));
    // Wildcard overlaps any host on the same port.
    assert!(fwd::listen_endpoints_conflict(
        "0.0.0.0",
        8080,
        "192.168.1.5",
        8080
    ));
    assert!(fwd::listen_endpoints_conflict("::1", 8080, "*", 8080));
    // Different port or auto-pick never conflicts.
    assert!(!fwd::listen_endpoints_conflict(
        "127.0.0.1",
        8080,
        "127.0.0.1",
        8081
    ));
    assert!(!fwd::listen_endpoints_conflict(
        "127.0.0.1",
        0,
        "127.0.0.1",
        8080
    ));
    // Different concrete hosts coexist.
    assert!(!fwd::listen_endpoints_conflict(
        "127.0.0.1",
        8080,
        "192.168.1.5",
        8080
    ));
}

#[test]
fn interface_rows_order_loopback_v4_v6_and_dedupe() {
    use std::net::IpAddr;
    let ip = |v: &str| v.parse::<IpAddr>().unwrap();
    let rows = interface_rows(vec![
        ("en0".to_string(), ip("192.168.1.10"), false),
        ("utun3".to_string(), ip("::1"), true),
        ("lo0".to_string(), ip("127.0.0.1"), true),
        ("en1".to_string(), ip("192.168.1.10"), false), // duplicate IP across ifaces
    ]);
    let list = rows["interfaces"].as_array().unwrap();
    let addrs: Vec<&str> = list
        .iter()
        .map(|row| row["addr"].as_str().unwrap())
        .collect();
    // Both loopbacks lead (v4 before v6 inside the group), then the rest.
    assert_eq!(addrs, vec!["127.0.0.1", "::1", "192.168.1.10"]);
    assert!(list[0]["isLoopback"].as_bool().unwrap());
    assert_eq!(list[0]["name"].as_str().unwrap(), "lo0");
    assert!(list[1]["isLoopback"].as_bool().unwrap());
    assert!(!list[2]["isLoopback"].as_bool().unwrap());
}

#[test]
fn describes_both_directions() {
    assert_eq!(
        describe(ForwardKind::Local, "127.0.0.1", 8080, "db", 5432),
        "127.0.0.1:8080 -> db:5432"
    );
    assert_eq!(
        describe(ForwardKind::Remote, "127.0.0.1", 80, "web", 3000),
        "127.0.0.1:80 <- web:3000"
    );
}

#[test]
fn remote_table_lookup_matches_exact_and_port_fallback() {
    use crate::features::forwarding::forward as fwd;
    let table: RemoteForwardTable = Mutex::new(HashMap::new());
    let entry = |listen_port: u16| {
        Arc::new(ForwardEntry {
            id: format!("fwd-{listen_port}"),
            session_id: "s".to_string(),
            connection_id: "c".to_string(),
            kind: ForwardKind::Remote,
            listen_host: "127.0.0.1".to_string(),
            listen_port,
            target_host: "127.0.0.1".to_string(),
            target_port: 3000,
            transport: None,
            bound_port: AtomicU32::new(listen_port as u32),
            state: Mutex::new(ForwardState::Active),
            error: Mutex::new(None),
            connections_total: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            bytes_up: AtomicU64::new(0),
            bytes_down: AtomicU64::new(0),
            listener_task: Mutex::new(None),
            relays: Mutex::new(Vec::new()),
            emitter: None,
            stopping: AtomicBool::new(false),
        })
    };
    fwd::register_remote_target(&table, "127.0.0.1", 8080, "web", 3000, entry(8080));
    // Exact match on the requested host.
    let hit = fwd::lookup_remote_target(&table, "127.0.0.1", 8080).unwrap();
    assert_eq!(hit.host, "web");
    assert_eq!(hit.port, 3000);
    // Server may report a different bound form for the same port: the
    // port-only fallback must still resolve it.
    let hit = fwd::lookup_remote_target(&table, "0.0.0.0", 8080).unwrap();
    assert_eq!(hit.host, "web");
    // Unknown port misses.
    assert!(fwd::lookup_remote_target(&table, "127.0.0.1", 9999).is_none());
}

#[test]
fn note_relay_end_saturates_at_zero() {
    let entry = ForwardEntry {
        id: "fwd-test".to_string(),
        session_id: "s".to_string(),
        connection_id: "c".to_string(),
        kind: ForwardKind::Local,
        listen_host: "127.0.0.1".to_string(),
        listen_port: 8080,
        target_host: "web".to_string(),
        target_port: 3000,
        transport: None,
        bound_port: AtomicU32::new(8080),
        state: Mutex::new(ForwardState::Active),
        error: Mutex::new(None),
        connections_total: AtomicU64::new(0),
        connections_active: AtomicI64::new(0),
        bytes_up: AtomicU64::new(0),
        bytes_down: AtomicU64::new(0),
        listener_task: Mutex::new(None),
        relays: Mutex::new(Vec::new()),
        emitter: None,
        stopping: AtomicBool::new(false),
    };
    // Simulate the double-decrement race: a relay ends normally and the
    // stop-drain path also accounted for it (or a stale counter reached zero).
    note_relay_end(&entry);
    assert_eq!(entry.connections_active.load(Ordering::Relaxed), 0);
    note_relay_end(&entry);
    assert_eq!(entry.connections_active.load(Ordering::Relaxed), 0);

    // A real active relay still decrements exactly once.
    entry.connections_active.store(2, Ordering::Relaxed);
    note_relay_end(&entry);
    assert_eq!(entry.connections_active.load(Ordering::Relaxed), 1);
    note_relay_end(&entry);
    assert_eq!(entry.connections_active.load(Ordering::Relaxed), 0);
}
