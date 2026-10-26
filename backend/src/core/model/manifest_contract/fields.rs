use super::*;

/// Credential requirements must be *satisfiable on the form*:
/// the login password is strict per `password_source` (direct → Password
/// required, command → Password command required), which the user can
/// always satisfy by typing it or switching the source; key material stays
/// either-or (path ∨ pasted content) and therefore must never become
/// form-required.
/// Anything else either dead-locks the dialog footer or lets a save through
/// that the parser then rejects.
#[test]
fn credential_requirements_are_form_satisfiable() {
    // No credential field may be *statically* required: a static flag
    // ignores the other branch (password_command / pasted key content) and
    // dead-locks the footer. Requirements have to hang off the explicit
    // source selector, which always has an alternative option.
    let selectors: Vec<&str> = ["authentication", "password_source"].to_vec();
    for entry in connection_fields() {
        let key = entry["key"].as_str().unwrap().to_string();
        let binding = entry["binding"].as_str().unwrap_or("");
        if !matches!(binding, "password" | "secret" | "config") {
            continue;
        }
        assert!(
            !entry["required"].as_bool().unwrap_or(false),
            "credential field '{key}' must not be statically required"
        );
        if let Some(source) = condition_field(&entry, "required_when") {
            assert!(
                selectors.contains(&source),
                "credential field '{key}' must be required via a user-chosen selector, not '{source}'"
            );
            let options = field(source)["options"]
                .as_array()
                .map(|options| options.len())
                .unwrap_or(0);
            let required_for = condition_one_of(&entry, "required_when").unwrap_or_default();
            assert!(
                options > required_for.len(),
                "credential field '{key}' must have an escape branch in '{source}'"
            );
        }
    }

    // private_key_path / private_key stay either-or: never form-required.
    for key in ["private_key_path", "private_key", "private_key_passphrase"] {
        assert_eq!(
            condition_field(&field(key), "required_when"),
            None,
            "{key} must stay optional (path or pasted content is enough)"
        );
    }

    // The parse-time rules are real: with neither half of an OR the parser
    // rejects, and the message names the two form fields that fix it.
    let missing_password = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "no-pw",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "password" }
        }
    }))
    .unwrap_err();
    assert!(missing_password.contains("Password"), "{missing_password}");
    assert!(
        missing_password.contains("Password command"),
        "{missing_password}"
    );
    let missing_key = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "no-key",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "private-key" }
        }
    }))
    .unwrap_err();
    assert!(missing_key.contains("Private key path"), "{missing_key}");
    assert!(missing_key.contains("Private key content"), "{missing_key}");
}

// -----------------------------------------------------------------------
// Host dialog semantics, mirrored from the host checkout so this repository
// can regression-test the form contract without importing it:
//   * `pluginFieldConditions.ts` — visibility cascades through the
//     referenced sibling; `required_when` never implies required.
//   * `ConnectionDialog.connectionConfigForSubmit` — "save" is blocked by
//     the first field that is visible, required and empty (surfaced as
//     `connection.pluginRequiredField` = "请填写{field}").
//   * `frontendPlugin.buildPluginConnectionConfig` — where each bound value
//     lands (connection/external_config/connection_secrets).
// -----------------------------------------------------------------------

#[test]
fn port_zero_is_rejected_by_parser_with_a_self_explanatory_message() {
    let error = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "port-zero",
            "host": "example.com",
            "port": 0,
            "username": "user",
            "password": "pw",
            "external_config": { "authentication": "password" }
        }
    }))
    .unwrap_err();
    assert!(error.contains("port"), "{error}");
    let description = field("port")["description"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(description.contains("65535"), "{description}");
}

/// Visible-when pairing: pure Quick Sudo knobs follow the form's sudo
/// source selection; the 2FA quartet hides under `global` (the bound
/// profile owns the whole credential source, login-time
/// keyboard-interactive included) and stays visible for custom/off where
/// the connection's own values still serve login-time 2FA.
#[test]
fn quick_sudo_visibility_pairing() {
    // 协议门控（M9）把 SSH 字段的 visible_when 升级为 all_of（protocol=ssh
    // 叠加原条件）；断言改为在全部子句中查找目标门控字段。
    fn gated_on_one_of(entry: &Value, condition: &str, target: &str) -> Option<Vec<String>> {
        let condition_value = &entry[condition];
        let mut clauses: Vec<&Value> = Vec::new();
        if condition_value.get("field").is_some() {
            clauses.push(condition_value);
        }
        clauses.extend(condition_value["all_of"].as_array()?.iter());
        for clause in clauses {
            if clause["field"].as_str() == Some(target) {
                return Some(
                    clause["one_of"]
                        .as_array()?
                        .iter()
                        .map(|value| value.as_str().unwrap_or_default().to_string())
                        .collect(),
                );
            }
        }
        None
    }
    for key in ["sudo_password", "sudo_use_pty"] {
        let entry = field(key);
        assert_eq!(
            gated_on_one_of(&entry, "visible_when", "sudo_source"),
            Some(vec!["custom".to_string()]),
            "{key} must be gated on sudo_source=custom (protocol gate stacks alongside)"
        );
    }
    let profile = field("sudo_profile");
    assert_eq!(
        gated_on_one_of(&profile, "visible_when", "sudo_source"),
        Some(vec!["global".to_string()]),
        "sudo_profile must be gated on sudo_source=global"
    );
    let key = "auth_flow_mode";
    assert_eq!(
        gated_on_one_of(&field(key), "visible_when", "sudo_source"),
        Some(vec!["custom".to_string(), "off".to_string()]),
        "{key} must hide under global (profile owns the source) and stay visible for custom/off"
    );
    // password_prompt_hint 是 2FA 三件套之一（TOTP 密钥 / OTP 提示词 /
    // 密码提示词）：跟随 sudo 来源，且只在自动回码的两种 OTP 模式下出现
    // ——2FA 关闭时整组折叠，不再残留一行孤零零的密码提示词。
    assert_eq!(
        field("password_prompt_hint")["visible_when"],
        serde_json::json!({
            "all_of": [
                { "field": "protocol", "one_of": ["ssh"] },
                { "field": "sudo_source", "one_of": ["custom", "off"] },
                { "field": "auth_flow_mode", "one_of": ["password_then_otp", "password_plus_otp"] },
            ]
        }),
        "password_prompt_hint must follow the sudo source and fold with the 2FA trio when OTP auto-answer is off"
    );
    for key in ["totp_secret", "totp_prompt_hint"] {
        assert_eq!(
            gated_on_one_of(&field(key), "visible_when", "auth_flow_mode"),
            Some(vec![
                "password_then_otp".to_string(),
                "password_plus_otp".to_string()
            ])
        );
    }
}

/// 私钥凭据是「路径或内容」二选一：只贴内容、只给路径都能解析，两者
/// 全缺才在连接时报错；口令（passphrase）对未加密密钥保持可选。
#[test]
fn private_key_accepts_path_or_content() {
    let entry = field("private_key_path");
    assert!(
        entry["required_when"].is_null(),
        "private_key_path must stay optional: pasted key content is a valid alternative"
    );
    assert!(field("private_key_passphrase")["required_when"].is_null());

    let content_only = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "content-only",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "private-key" },
            "connection_secrets": { "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\n..." }
        }
    }))
    .unwrap();
    assert!(content_only.private_key_path.is_empty());
    assert!(content_only.private_key.starts_with("-----BEGIN"));

    // 多行凭据必须原样保留：PEM/PPK 内容不允许被 trim 破坏。
    let multiline = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "multiline-key",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "private-key" },
            "connection_secrets": { "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----\n" }
        }
    }))
    .unwrap();
    assert!(multiline
        .private_key
        .ends_with("-----END OPENSSH PRIVATE KEY-----\n"));

    let neither = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "no-key",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "private-key" },
            "connection_secrets": {}
        }
    }))
    .unwrap_err();
    assert!(
        neither.contains("requires a private key path or key content"),
        "{neither}"
    );
}

/// The passphrase stays optional for unencrypted keys (private-key-password
/// mode must not be rejected for a missing passphrase).
#[test]
fn private_key_password_fallback_does_not_require_passphrase() {
    let without_passphrase = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "plain-key",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": {
                "authentication": "private-key-password",
                "private_key_path": "/keys/id_ed25519"
            },
            "connection_secrets": {}
        }
    }))
    .unwrap_err();
    assert!(
        !without_passphrase.contains("private key path"),
        "unencrypted private-key-password keys must not be rejected for a missing passphrase"
    );
}
