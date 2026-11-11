use super::*;

fn assert_unix(display: &str, number: u32, path: &str) {
    let parsed = parse_display(display).expect("unix display must parse");
    assert_eq!(parsed.number, number, "{display}");
    assert_eq!(
        parsed.server,
        DisplayServer::UnixSocket(PathBuf::from(path)),
        "{display}"
    );
}

fn assert_tcp(display: &str, number: u32, port: u16) {
    let parsed = parse_display(display).expect("tcp display must parse");
    assert_eq!(parsed.number, number, "{display}");
    assert_eq!(parsed.server, DisplayServer::TcpLoopback(port), "{display}");
}

#[test]
fn parse_display_matches_x7_forms() {
    assert_unix(":0", 0, "/tmp/.X11-unix/X0");
    assert_unix(":10.0", 10, "/tmp/.X11-unix/X10");
    assert_unix("unix:3", 3, "/tmp/.X11-unix/X3");
    assert_unix("unix:", 0, "/tmp/.X11-unix/X0");
    assert_unix(
        "unix:/private/tmp/com.apple.launchd.abc/org.xquartz",
        0,
        "/private/tmp/com.apple.launchd.abc/org.xquartz",
    );
    assert_unix("unix:/tmp/x:1", 1, "/tmp/x");
    // macOS launchd hands out DISPLAY values like
    // /private/tmp/com.apple.launchd.*/org.xquartz:0
    assert_unix(
        "/private/tmp/com.apple.launchd.abc/org.xquartz:0",
        0,
        "/private/tmp/com.apple.launchd.abc/org.xquartz",
    );
    assert_tcp("localhost:0", 0, 6000);
    assert_tcp("remote.example:2.1", 2, 6002);
    // Surrounding whitespace is trimmed before parsing: a trimmed `:7`
    // has no host part, so it is the unix form (exercises the trim path).
    assert_unix("  :7  ", 7, "/tmp/.X11-unix/X7");
}

#[test]
fn parse_display_rejects_unparsable_values() {
    assert_eq!(parse_display(""), None);
    assert_eq!(parse_display("   "), None);
    assert_eq!(parse_display(":"), None);
    assert_eq!(parse_display("host:"), None);
    assert_eq!(parse_display(":abc"), None);
    assert_eq!(parse_display(":1.2.3"), None);
    assert_eq!(parse_display(":-1"), None);
    assert_eq!(parse_display(":0x10"), None);
    assert_eq!(parse_display("unix:abc"), None);
    assert_eq!(parse_display("unix:/tmp/x:abc"), None);
    // 6000 + n would not fit a u16 anymore.
    assert_eq!(parse_display(":59536"), None);
    assert_eq!(parse_display("host:99999999999"), None);
}

#[test]
fn cookie_generation_is_unique_lowercase_hex() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
        let cookie = FakeCookie::generate().expect("CSPRNG");
        let hex = cookie.hex();
        assert_eq!(hex.len(), 32);
        assert!(hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        assert!(seen.insert(hex.to_string()), "cookie repeated");
    }
}

/// Builds an X connection setup block the way Xlib writes it.
fn setup_packet(byte_order: u8, lsb: bool, protocol: &[u8], cookie: &[u8]) -> Vec<u8> {
    let put = |value: u16| -> Vec<u8> {
        if lsb {
            value.to_le_bytes().to_vec()
        } else {
            value.to_be_bytes().to_vec()
        }
    };
    let mut out = vec![byte_order, 0, 11, 0, 0, 0];
    out.extend_from_slice(&put(protocol.len() as u16));
    out.extend_from_slice(&put(cookie.len() as u16));
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(protocol);
    out.resize(out.len() + pad4(protocol.len()) - protocol.len(), 0);
    out.extend_from_slice(cookie);
    out.resize(out.len() + pad4(cookie.len()) - cookie.len(), 0);
    out
}

#[test]
fn inspect_setup_accepts_matching_cookie_on_both_byte_orders() {
    let cookie = FakeCookie::generate().expect("CSPRNG");
    let expected_offset = 12 + pad4(X11_AUTH_PROTOCOL.len());
    for (byte_order, lsb) in [(0x6c_u8, true), (0x42, false)] {
        let packet = setup_packet(
            byte_order,
            lsb,
            X11_AUTH_PROTOCOL.as_bytes(),
            cookie.bytes(),
        );
        assert_eq!(
            inspect_setup(&packet, &cookie),
            SetupInspection::Accept {
                cookie_offset: expected_offset
            },
            "byte order {byte_order:#x}"
        );
        // Trailing bytes (already-relayed X traffic) are fine: only the
        // setup block itself is inspected.
        let mut extended = packet.clone();
        extended.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(
            inspect_setup(&extended, &cookie),
            SetupInspection::Accept {
                cookie_offset: expected_offset
            },
            "byte order {byte_order:#x}"
        );
    }
}

#[test]
fn inspect_setup_needs_the_whole_setup_block() {
    let cookie = FakeCookie::generate().expect("CSPRNG");
    assert_eq!(inspect_setup(&[], &cookie), SetupInspection::Incomplete);
    let packet = setup_packet(0x6c, true, X11_AUTH_PROTOCOL.as_bytes(), cookie.bytes());
    assert_eq!(
        inspect_setup(&packet[..7], &cookie),
        SetupInspection::Incomplete
    );
    assert_eq!(
        inspect_setup(&packet[..packet.len() - 1], &cookie),
        SetupInspection::Incomplete
    );
}

#[test]
fn inspect_setup_rejects_malformed_or_foreign_setups() {
    let cookie = FakeCookie::generate().expect("CSPRNG");
    assert_eq!(
        inspect_setup(&[0x00, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0], &cookie),
        SetupInspection::Reject("X setup has a bad byte-order byte")
    );
    let packet = setup_packet(0x6c, true, b"OTHER-AUTH", cookie.bytes());
    assert_eq!(
        inspect_setup(&packet, &cookie),
        SetupInspection::Reject("X setup uses a different authorization protocol")
    );
    let wrong_cookie = setup_packet(0x6c, true, X11_AUTH_PROTOCOL.as_bytes(), &[0u8; COOKIE_LEN]);
    assert_eq!(
        inspect_setup(&wrong_cookie, &cookie),
        SetupInspection::Reject("X setup cookie does not match the issued fake cookie")
    );
    let short_cookie = setup_packet(0x42, false, X11_AUTH_PROTOCOL.as_bytes(), &[1u8; 8]);
    assert_eq!(
        inspect_setup(&short_cookie, &cookie),
        SetupInspection::Reject("X setup carries an unexpected cookie length")
    );
}

fn xauth_record(family: u16, address: &[u8], number: &str, name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&family.to_be_bytes());
    for field in [address, number.as_bytes(), name, data] {
        out.extend_from_slice(&(field.len() as u16).to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

#[test]
fn xauthority_lookup_matches_family_and_number() {
    let data = [7u8; COOKIE_LEN];
    let mut store = Vec::new();
    // Local display record (family 256, hostname-style address).
    store.extend_from_slice(&xauth_record(
        FAMILY_LOCAL,
        b"somehost",
        "0",
        X11_AUTH_PROTOCOL.as_bytes(),
        &data,
    ));
    // A foreign display number must not leak its cookie.
    store.extend_from_slice(&xauth_record(
        FAMILY_LOCAL,
        b"somehost",
        "1",
        X11_AUTH_PROTOCOL.as_bytes(),
        &[9u8; COOKIE_LEN],
    ));
    // Wildcard family matches the number too.
    store.extend_from_slice(&xauth_record(
        FAMILY_WILD,
        &[],
        "2",
        X11_AUTH_PROTOCOL.as_bytes(),
        &[5u8; COOKIE_LEN],
    ));
    let records = parse_xauthority(&store);
    assert_eq!(records.len(), 3);
    assert_eq!(find_real_cookie(&records, 0), Some(data.to_vec()));
    assert_eq!(find_real_cookie(&records, 2), Some(vec![5u8; COOKIE_LEN]));
    // A different authorization protocol name never matches.
    let wrong_name = parse_xauthority(&xauth_record(
        FAMILY_LOCAL,
        &[],
        "0",
        b"XDM-AUTHORIZATION-1",
        &data,
    ));
    assert_eq!(find_real_cookie(&wrong_name, 0), None);
    // No record at all.
    assert_eq!(find_real_cookie(&parse_xauthority(&[]), 0), None);
}

#[test]
fn xauthority_parser_survives_truncated_records() {
    let mut store = xauth_record(
        FAMILY_LOCAL,
        &[],
        "0",
        X11_AUTH_PROTOCOL.as_bytes(),
        &[3u8; COOKIE_LEN],
    );
    // Append a record whose data field is cut short.
    let mut torn = xauth_record(
        FAMILY_LOCAL,
        &[],
        "1",
        X11_AUTH_PROTOCOL.as_bytes(),
        &[4u8; 4],
    );
    torn.truncate(torn.len() - 2);
    store.extend_from_slice(&torn);
    let records = parse_xauthority(&store);
    assert_eq!(records.len(), 1);
    assert_eq!(find_real_cookie(&records, 0), Some(vec![3u8; COOKIE_LEN]));
}

#[test]
fn gate_fails_closed_for_unarmed_sessions() {
    // Fresh gate: no session ever asked for X11 -> every channel out.
    let gate = Arc::new(X11Gate::default());
    assert!(gate.try_admit().is_none());
    // Disarming a previously armed gate closes it again (session close).
    let (gate, hex) = X11Gate::armed().expect("arm");
    assert_eq!(hex.len(), 32);
    assert!(gate.try_admit().is_some());
    gate.disarm();
    assert!(gate.try_admit().is_none());
}

#[test]
fn gate_hands_the_issued_cookie_to_the_bridge() {
    let (gate, hex) = X11Gate::armed().expect("arm");
    let (cookie, _permit) = gate.try_admit().expect("admit while armed");
    assert_eq!(cookie.hex(), hex);
    // Dropping the permit releases the slot; the gate is reusable.
    drop(_permit);
    assert!(gate.try_admit().is_some());
}

#[test]
fn gate_caps_concurrent_bridges_per_session() {
    let (gate, _hex) = X11Gate::armed().expect("arm");
    let mut permits = Vec::new();
    for _ in 0..MAX_BRIDGES_PER_SESSION {
        permits.push(gate.try_admit().expect("slot within the cap"));
    }
    assert!(gate.try_admit().is_none(), "cap must hold");
    // Releasing one slot admits the next waiting channel.
    drop(permits.pop());
    assert!(gate.try_admit().is_some());
}

#[test]
fn gate_replacement_disarms_the_previous_gate() {
    let (old, old_hex) = X11Gate::armed().expect("arm");
    let (new, new_hex) = X11Gate::armed().expect("arm");
    assert_ne!(old_hex, new_hex, "each arm draws a fresh cookie");
    let mut slot = Some(old.clone());
    replace_gate(&mut slot, new.clone());
    // The replaced gate was disarmed on the way out: a channel racing
    // the re-arm must fail closed instead of honoring the stale cookie.
    assert!(old.try_admit().is_none());
    let (cookie, _permit) = new.try_admit().expect("the newest gate admits");
    assert_eq!(cookie.hex(), new_hex);
}

#[test]
fn replacement_keeps_admit_counts_per_gate() {
    let (old, _hex) = X11Gate::armed().expect("arm");
    let _held_on_old = old.try_admit().expect("old gate admits");
    let (new, _hex) = X11Gate::armed().expect("arm");
    let mut slot = Some(old);
    replace_gate(&mut slot, new.clone());
    // The new gate starts from an empty cap: permits still held on the
    // replaced gate must not consume the new gate's slots.
    assert_eq!(new.bridges.load(Ordering::Acquire), 0);
    let mut permits = Vec::new();
    for _ in 0..MAX_BRIDGES_PER_SESSION {
        permits.push(new.try_admit().expect("full cap available on the new gate"));
    }
    assert!(new.try_admit().is_none(), "cap must hold on the new gate");
}

/// Serializes against every other global-slot test: `ACTIVE_GATE` is
/// process-wide, so exactly one test touches the arm/admit/disarm path.
#[test]
fn active_slot_arms_admits_and_disarms() {
    let hex = arm_session().expect("arm");
    let (cookie, _permit) = try_admit_active().expect("admit while armed");
    assert_eq!(cookie.hex(), hex);
    disarm_active();
    assert!(try_admit_active().is_none(), "disarm must fail closed");
}

#[test]
fn setup_buffer_rejects_a_foreign_cookie_across_chunks() {
    let (gate, hex) = X11Gate::armed().expect("arm");
    let (cookie, _permit) = gate.try_admit().expect("admit");
    assert_eq!(cookie.hex(), hex);
    // A setup block carrying a different cookie than the one issued for
    // the session is rejected even when it arrives chunked.
    let foreign = FakeCookie::generate().expect("CSPRNG");
    let packet = setup_packet(0x6c, true, X11_AUTH_PROTOCOL.as_bytes(), foreign.bytes());
    let mut buffer = SetupBuffer::default();
    let (head, tail) = packet.split_at(5);
    assert_eq!(buffer.push(head, &cookie), SetupInspection::Incomplete);
    assert_eq!(
        buffer.push(tail, &cookie),
        SetupInspection::Reject("X setup cookie does not match the issued fake cookie")
    );
}

#[test]
fn setup_buffer_bounds_a_peer_that_never_finishes() {
    let (gate, _hex) = X11Gate::armed().expect("arm");
    let (cookie, _permit) = gate.try_admit().expect("admit");
    // A header declaring a huge protocol length keeps the block
    // "incomplete" forever; the buffer must cut the channel off instead
    // of growing without bound.
    let mut header = vec![0x6c, 0, 11, 0, 0, 0];
    header.extend_from_slice(&u16::MAX.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    header.extend_from_slice(&[0, 0]);
    let mut buffer = SetupBuffer::default();
    assert_eq!(buffer.push(&header, &cookie), SetupInspection::Incomplete);
    for _ in 0..1_000 {
        match buffer.push(&[0u8; 512], &cookie) {
            SetupInspection::Incomplete => continue,
            SetupInspection::Reject(_) => return,
            SetupInspection::Accept { .. } => panic!("an unfinished block cannot accept"),
        }
    }
    panic!("the setup size bound never tripped");
}

#[test]
fn substitute_cookie_swaps_the_fake_for_the_real_one() {
    let cookie = FakeCookie::generate().expect("CSPRNG");
    let offset = 12 + pad4(X11_AUTH_PROTOCOL.len());
    let build = || setup_packet(0x6c, true, X11_AUTH_PROTOCOL.as_bytes(), cookie.bytes());
    // A matching-length real cookie replaces the fake in place.
    let real = vec![0xa5u8; COOKIE_LEN];
    let mut setup = build();
    substitute_cookie(&mut setup, offset, Some(&real));
    assert_eq!(&setup[offset..offset + COOKIE_LEN], real.as_slice());
    // No real cookie: the fake is relayed unchanged.
    let mut setup = build();
    substitute_cookie(&mut setup, offset, None);
    assert_eq!(&setup[offset..offset + COOKIE_LEN], cookie.bytes());
    // An odd-length real cookie must never corrupt the block.
    let mut setup = build();
    substitute_cookie(&mut setup, offset, Some(&[1u8; 3]));
    assert_eq!(&setup[offset..offset + COOKIE_LEN], cookie.bytes());
}

#[test]
fn pad4_matches_x_protocol_alignment() {
    assert_eq!(pad4(0), 0);
    assert_eq!(pad4(1), 4);
    assert_eq!(pad4(4), 4);
    assert_eq!(pad4(18), 20);
}

#[test]
fn enabled_from_reads_the_nested_prefs_object() {
    // Regression: the on-disk shape is `{"version": N, "prefs": {...}}`, so a
    // top-level `x11_forwarding` lookup silently returned false and X11
    // requests were never sent even with the switch on.
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert!(!enabled_from(data_dir.path()), "missing file must fail closed");
    std::fs::create_dir_all(data_dir.path()).expect("data dir");
    std::fs::write(
        crate::core::preferences::store_path(data_dir.path()),
        r#"{"version":1,"prefs":{"x11_forwarding":true}}"#,
    )
    .expect("write preferences");
    assert!(enabled_from(data_dir.path()));
    std::fs::write(
        crate::core::preferences::store_path(data_dir.path()),
        r#"{"version":1,"prefs":{"x11_forwarding":false}}"#,
    )
    .expect("write preferences");
    assert!(!enabled_from(data_dir.path()));
}
