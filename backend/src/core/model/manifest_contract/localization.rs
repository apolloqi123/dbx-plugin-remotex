use super::*;

/// Manifest defaults must equal the parser's fallback defaults; feeding
/// them through `from_lifecycle_params` reproduces the same connection.
#[test]
fn defaults_match_parser_fallbacks() {
    let expected_defaults: &[(&str, Value)] = &[
        ("display_name", Value::from("RemoteX")),
        ("protocol", Value::from("ssh")),
        ("host", Value::from("127.0.0.1")),
        ("port", Value::from(22)),
        ("username", Value::from("root")),
        ("authentication", Value::from("password")),
        ("connect_timeout_secs", Value::from(30)),
        ("keepalive_interval_secs", Value::from(30)),
        ("terminal_keepalive_secs", Value::from(0)),
        ("triggers_enabled", Value::from(false)),
        ("sudo_source", Value::from("custom")),
        ("sudo_use_pty", Value::from(false)),
        ("set_env", Value::from("")),
        ("triggers", Value::from("")),
        ("password_command", Value::from("")),
        ("passphrase_command", Value::from("")),
        ("remote_command", Value::from("")),
        // auth_flow_mode 有意不在此列：表单默认 off（新连接不自动回
        // OTP），解析回退保持 PasswordThenOtp（存量连接不受新默认影
        // 响），两者允许分歧——见 auth_flow_mode 表单默认断言。
        ("read_only", Value::from(false)),
    ];
    for (key, expected) in expected_defaults {
        assert_eq!(
            field(key)["default"],
            *expected,
            "manifest default mismatch for '{key}'"
        );
    }

    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "defaults",
            "host": "127.0.0.1",
            "port": 22,
            "username": "root",
            "password": "secret",
            "external_config": {
                "authentication": "password",
                "connect_timeout_secs": 15,
                "keepalive_interval_secs": 30,
                "terminal_keepalive_secs": 0,
                "sudo_source": "custom",
                "sudo_use_pty": false,
                "auth_flow_mode": "password_then_otp",
                "read_only": false
            }
        }
    }))
    .unwrap();
    assert_eq!(connection.authentication, AuthenticationMethod::Password);
    assert_eq!(connection.connect_timeout_secs, 15);
    assert_eq!(connection.keepalive_interval_secs, 30);
    assert_eq!(connection.terminal_keepalive_secs, 0);
    assert!(connection.sudo_enabled());
    assert!(!connection.sudo_use_pty);
    assert_eq!(connection.auth_flow_mode, "password_then_otp");
    assert!(!connection.read_only);
}

/// The terminal activity keepalive is opt-in: absent config means off,
/// an enabled value is clamped into 5..=3600 regardless of what the
/// connection form passes through.
#[test]
fn terminal_keepalive_is_opt_in_and_clamped() {
    assert_eq!(clamp_terminal_keepalive(0), 0);
    assert_eq!(clamp_terminal_keepalive(1), 5);
    assert_eq!(clamp_terminal_keepalive(4), 5);
    assert_eq!(clamp_terminal_keepalive(30), 30);
    assert_eq!(clamp_terminal_keepalive(3600), 3600);
    assert_eq!(clamp_terminal_keepalive(100_000), 3600);

    let enabled = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "keepalive-on",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret",
            "external_config": {
                "terminal_keepalive_secs": 90,
                "keepalive_interval_secs": 60,
                "connect_timeout_secs": 20
            }
        }
    }))
    .unwrap();
    assert_eq!(enabled.terminal_keepalive_secs, 90);
    // Form-driven timeout/keepalive must win over the defaults too (the
    // parser reads external_config with a top-level fallback).
    assert_eq!(enabled.keepalive_interval_secs, 60);
    assert_eq!(enabled.connect_timeout_secs, 20);

    let off = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "keepalive-off",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret",
            "external_config": { "terminal_keepalive_secs": 0 }
        }
    }))
    .unwrap();
    assert_eq!(off.terminal_keepalive_secs, 0);
}
