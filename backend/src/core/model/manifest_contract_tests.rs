use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn manifest() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../manifest.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read manifest at {}: {error}", path.display()));
    serde_json::from_str(&raw).expect("manifest.json must be valid JSON")
}

fn connection_fields() -> Vec<Value> {
    manifest()["contributions"]
        .as_array()
        .expect("contributions array")
        .iter()
        .find(|entry| entry["type"] == "connection-provider")
        .expect("connection-provider contribution")["fields"]
        .as_array()
        .expect("fields array")
        .clone()
}

fn field(key: &str) -> Value {
    connection_fields()
        .into_iter()
        .find(|entry| entry["key"].as_str() == Some(key))
        .unwrap_or_else(|| panic!("manifest field '{key}' missing"))
}

fn condition_one_of(entry: &Value, condition: &str) -> Option<Vec<String>> {
    entry[condition]["one_of"].as_array().map(|values| {
        values
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_string())
            .collect()
    })
}

fn condition_field<'a>(entry: &'a Value, condition: &str) -> Option<&'a str> {
    entry[condition]["field"].as_str()
}

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

/// Dialog value of a field: the stored/typed value, else the manifest default.
fn form_value(values: &BTreeMap<String, Value>, entry: &Value) -> Option<Value> {
    let key = entry["key"].as_str().unwrap_or_default();
    match values.get(key) {
        Some(value) => Some(value.clone()),
        None => entry
            .get("default")
            .filter(|value| !value.is_null())
            .cloned(),
    }
}

fn form_condition_matches(
    condition: &Value,
    values: &BTreeMap<String, Value>,
    fields: &[Value],
) -> bool {
    let Some(key) = condition["field"].as_str() else {
        return false;
    };
    let Some(target) = fields
        .iter()
        .find(|entry| entry["key"].as_str() == Some(key))
    else {
        return false;
    };
    let Some(value) = form_value(values, target) else {
        return false;
    };
    let text = match value {
        Value::Null => return false,
        Value::String(text) => text,
        other => other.to_string(),
    };
    if text.trim().is_empty() {
        return false;
    }
    condition["one_of"].as_array().is_some_and(|one_of| {
        one_of
            .iter()
            .any(|option| option.as_str() == Some(text.as_str()))
    })
}

fn form_field_visible(entry: &Value, values: &BTreeMap<String, Value>, fields: &[Value]) -> bool {
    let mut seen = vec![entry["key"].as_str().unwrap_or_default().to_string()];
    let mut current = entry;
    loop {
        let Some(condition) = current.get("visible_when").filter(|value| !value.is_null()) else {
            return true;
        };
        if !form_condition_matches(condition, values, fields) {
            return false;
        }
        let Some(target_key) = condition["field"].as_str() else {
            return true;
        };
        if seen.iter().any(|key| key == target_key) {
            return true;
        }
        seen.push(target_key.to_string());
        let Some(target) = fields
            .iter()
            .find(|candidate| candidate["key"].as_str() == Some(target_key))
        else {
            return true;
        };
        current = target;
    }
}

fn form_field_required(entry: &Value, values: &BTreeMap<String, Value>, fields: &[Value]) -> bool {
    if entry["required"].as_bool().unwrap_or(false) {
        return true;
    }
    match entry.get("required_when").filter(|value| !value.is_null()) {
        None => false,
        Some(condition) => form_condition_matches(condition, values, fields),
    }
}

fn form_value_present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

/// The field the dialog would block "save" on, if any.
fn form_blocking_field(fields: &[Value], values: &BTreeMap<String, Value>) -> Option<String> {
    fields
        .iter()
        .find(|entry| {
            form_field_visible(entry, values, fields)
                && form_field_required(entry, values, fields)
                && !form_value_present(form_value(values, entry).as_ref())
        })
        .map(|entry| entry["key"].as_str().unwrap_or_default().to_string())
}

/// The lifecycle `connection` object a save would persist, built the way the
/// host builds it (config → `external_config`, secret → `connection_secrets`
/// with empty values removed, identity/password bindings hoisted).
fn connection_payload(fields: &[Value], values: &BTreeMap<String, Value>) -> Value {
    let mut external = serde_json::Map::new();
    let mut secrets = serde_json::Map::new();
    let mut name = String::from("SSH server");
    let mut host = String::new();
    let mut port: u64 = 0;
    let mut username = String::new();
    let mut password = String::new();
    let text = |value: Option<Value>| -> String {
        match value {
            Some(Value::String(text)) => text,
            Some(other) => other.to_string(),
            None => String::new(),
        }
    };
    for entry in fields {
        let key = entry["key"].as_str().unwrap_or_default().to_string();
        let value = form_value(values, entry);
        match entry["binding"].as_str().unwrap_or_default() {
            "config" => match value {
                None | Some(Value::Null) => {
                    external.remove(&key);
                }
                Some(value) => {
                    external.insert(key.clone(), value);
                }
            },
            "secret" => match value {
                None | Some(Value::Null) => {
                    secrets.remove(&key);
                }
                Some(Value::String(value)) if value.is_empty() => {
                    secrets.remove(&key);
                }
                Some(value) => {
                    secrets.insert(key.clone(), Value::String(text(Some(value))));
                }
            },
            "name" => name = text(value),
            "host" => host = text(value),
            "port" => {
                port = value
                    .as_ref()
                    .and_then(Value::as_u64)
                    .map(|port| port.min(u64::from(u16::MAX)))
                    .unwrap_or(0);
            }
            "username" => username = text(value),
            "password" => password = text(value),
            _ => {}
        }
    }
    serde_json::json!({
        "id": "form-matrix",
        "name": name,
        "host": host,
        "port": port,
        "username": username,
        "password": password,
        "external_config": external,
        "connection_secrets": secrets,
    })
}

/// 表单“保存”判定 ⇔ 解析层接受度。宿主动画框只拦「可见 ∧ 必填 ∧ 为空」，
/// 真正的凭据规则在解析层，两者必须同向：
///   * 表单拦下、解析层却接受 → 用户被卡死（无法保存一个插件支持的配置）；
///   * 表单放行、解析层拒绝 → 只允许出现在 OR 语义（密码∨密码命令、
///     私钥路径∨私钥内容）无法在契约里表达处，且报错必须点名表单字段。
#[test]
fn form_save_state_matches_parser_acceptance() {
    let fields = connection_fields();
    let defaults: BTreeMap<String, Value> = fields
        .iter()
        .filter_map(|entry| {
            entry
                .get("default")
                .filter(|value| !value.is_null())
                .map(|value| {
                    (
                        entry["key"].as_str().unwrap_or_default().to_string(),
                        value.clone(),
                    )
                })
        })
        .collect();

    let mut cases = 0usize;
    let mut blocked_without_rejection = 0usize;
    let mut blocked_samples: Vec<String> = Vec::new();
    let mut unexplained_rejections: Vec<String> = Vec::new();
    for advanced_options in [false, true] {
        for authentication in [
            "password",
            "private-key",
            "private-key-password",
            "agent",
            "none",
        ] {
            for sudo_source in ["custom", "global", "off"] {
                for auth_flow_mode in [
                    "off",
                    "password_then_otp",
                    "password_plus_otp",
                    "password_only",
                ] {
                    for read_only in [false, true] {
                        for triggers_enabled in [false, true] {
                            for password in ["", "secret"] {
                                for password_command in ["", "echo pw"] {
                                    for password_source in ["direct", "command"] {
                                        for private_key_path in ["", "~/.ssh/id_ed25519"] {
                                            for private_key in
                                                ["", "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n"]
                                            {
                                                for private_key_passphrase in ["", "phrase"] {
                                                    let mut values = defaults.clone();
                                                    for (key, value) in [
                                                        (
                                                            "advanced_options",
                                                            json!(advanced_options),
                                                        ),
                                                        ("authentication", json!(authentication)),
                                                        ("sudo_source", json!(sudo_source)),
                                                        ("auth_flow_mode", json!(auth_flow_mode)),
                                                        ("read_only", json!(read_only)),
                                                        (
                                                            "triggers_enabled",
                                                            json!(triggers_enabled),
                                                        ),
                                                        ("password", json!(password)),
                                                        (
                                                            "password_command",
                                                            json!(password_command),
                                                        ),
                                                        ("password_source", json!(password_source)),
                                                        (
                                                            "private_key_path",
                                                            json!(private_key_path),
                                                        ),
                                                        ("private_key", json!(private_key)),
                                                        (
                                                            "private_key_passphrase",
                                                            json!(private_key_passphrase),
                                                        ),
                                                    ] {
                                                        values.insert(key.to_string(), value);
                                                    }
                                                    cases += 1;
                                                    let blocked =
                                                        form_blocking_field(&fields, &values);
                                                    let payload =
                                                        connection_payload(&fields, &values);
                                                    let parsed =
                                                        StoredConnection::from_lifecycle_params(
                                                            &json!({ "connection": payload }),
                                                        );
                                                    match (&blocked, &parsed) {
                                                        (Some(key), Ok(_)) => {
                                                            blocked_without_rejection += 1;
                                                            if blocked_samples.len() < 4 {
                                                                blocked_samples.push(format!(
                                                                "{key}: authentication={authentication} \
                                                                 password={password:?} \
                                                                 password_command={password_command:?} \
                                                                 private_key_path={private_key_path:?} \
                                                                 private_key={} \
                                                                 private_key_passphrase={private_key_passphrase:?}",
                                                                if private_key.is_empty() {
                                                                    "\"\""
                                                                } else {
                                                                    "<key>"
                                                                }
                                                            ));
                                                            }
                                                        }
                                                        (None, Err(error)) => {
                                                            let lower = error.to_lowercase();
                                                            // The message has to name the form
                                                            // fields that satisfy the rule, so a
                                                            // rejection is never a dead end.
                                                            let actionable = (lower
                                                                .contains("\"password\"")
                                                                && lower.contains(
                                                                    "\"password command\"",
                                                                ))
                                                                || (lower.contains(
                                                                    "\"private key path\"",
                                                                ) && lower.contains(
                                                                    "\"private key content\"",
                                                                ))
                                                                || lower.contains("ssh port");
                                                            if !actionable
                                                                && unexplained_rejections.len() < 4
                                                            {
                                                                unexplained_rejections
                                                                    .push(error.clone());
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    assert_eq!(
        blocked_without_rejection, 0,
        "the dialog blocks {blocked_without_rejection} configuration(s) the parser accepts \
         (dead-locked footer, no way to save). Samples: {blocked_samples:#?}"
    );
    assert!(
        unexplained_rejections.is_empty(),
        "parser rejects without naming the form field that fixes it: {unexplained_rejections:?}"
    );
    assert!(
        cases >= 15_000,
        "form/parser matrix shrank unexpectedly ({cases} cases)"
    );
}

/// 端口在表单里只受 `type: number` 约束，宿主 Rust 层与解析层各自校验
/// 1..65535；契约无 min/max 属性，所以范围提示必须留在描述里（前端契约
/// 脚本 `scripts/verify/connection-forms/verify.mjs` 断言七语都写了）。
#[test]
fn password_source_is_honored_by_the_parser() {
    let connection = |external: Value, password: Option<&str>| {
        let mut payload = json!({
            "id": "pw-source",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": external,
        });
        if let Some(password) = password {
            payload["password"] = json!(password);
        }
        StoredConnection::from_lifecycle_params(&json!({ "connection": payload }))
    };

    // 老配置（0.4.x 只有 password/password_command）与 MCP 内联参数没有
    // 来源字段：保持历史语义，两者二选一，显式密码优先、命令保留。
    let legacy = connection(
        json!({ "authentication": "password", "password_command": "echo pw" }),
        Some("stored"),
    )
    .unwrap();
    assert_eq!(legacy.password, "stored");
    assert_eq!(legacy.password_command, "echo pw");
    let legacy_command = connection(
        json!({ "authentication": "password", "password_command": "echo pw" }),
        None,
    )
    .unwrap();
    assert!(legacy_command.password.is_empty());
    assert_eq!(legacy_command.password_command, "echo pw");

    // direct：密码必填，缺失时点名两个表单字段（并给出改来源的出口）。
    let direct = connection(
        json!({ "authentication": "password", "password_source": "direct" }),
        Some("typed"),
    )
    .unwrap();
    assert_eq!(direct.password, "typed");
    let direct_missing = connection(
        json!({
            "authentication": "password",
            "password_source": "direct",
            "password_command": "echo pw"
        }),
        None,
    )
    .unwrap_err();
    assert!(
        direct_missing.contains("Password source"),
        "{direct_missing}"
    );
    assert!(direct_missing.contains("\"Password\""), "{direct_missing}");

    // command：命令必填，且存量密码让位——否则「显式密码优先」会让用户
    // 显式选择的本地命令永不执行。
    let command = connection(
        json!({
            "authentication": "password",
            "password_source": "command",
            "password_command": "echo pw"
        }),
        Some("stale"),
    )
    .unwrap();
    assert!(
        command.password.is_empty(),
        "command source must drop the stored password so the command runs"
    );
    assert_eq!(command.password_command, "echo pw");
    let command_missing = connection(
        json!({ "authentication": "password", "password_source": "command" }),
        Some("stale"),
    )
    .unwrap_err();
    assert!(
        command_missing.contains("Password command"),
        "{command_missing}"
    );

    // 非密码类认证不受来源约束（字段隐藏，值可能是残留）。
    let agent = connection(
        json!({ "authentication": "agent", "password_source": "command" }),
        None,
    )
    .unwrap();
    assert!(agent.password.is_empty());

    // 未知来源：明确报错，不静默回退成 direct。
    let unknown = connection(
        json!({ "authentication": "password", "password_source": "keychain" }),
        Some("typed"),
    )
    .unwrap_err();
    assert!(
        unknown.contains("Unsupported password source 'keychain'"),
        "{unknown}"
    );
}

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

/// 发布前的表单组合矩阵：认证方式、sudo 来源、只读开关、2FA 模式等
/// 下拉/开关的每一种交叉，在解析层要么成功且语义正确，要么干净报错
/// ——绝不 panic、绝不静默错位。表单切换认证方式会在不可见字段里留下
/// 残留凭据，这类组合必须被容忍（多余凭据被忽略而非报错），否则用户改
/// 一个下拉框就再也连不上。
#[test]
fn form_option_combinations_parse_without_conflicts() {
    let connection_with = |authentication: &str,
                           password: Option<&str>,
                           mut external_config: Value,
                           secrets: Value|
     -> Result<StoredConnection, String> {
        external_config["authentication"] = Value::String(authentication.to_string());
        let mut connection = serde_json::json!({
            "id": "combo-matrix",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": external_config
        });
        if let Some(password) = password {
            connection["password"] = Value::String(password.to_string());
        }
        if !secrets.is_null() {
            connection["connection_secrets"] = secrets;
        }
        StoredConnection::from_lifecycle_params(&serde_json::json!({ "connection": connection }))
    };

    // ① 每个认证方式的最小必需凭据组合都必须通过（含表单里显式提供
    //    的 none / agent 这两个"零凭据"选项）。
    let minimal: &[(&str, Option<&str>, Value, Value, AuthenticationMethod)] = &[
        (
            "password",
            Some("pw"),
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Password,
        ),
        (
            "private-key",
            None,
            serde_json::json!({ "private_key_path": "~/.ssh/id_ed25519" }),
            serde_json::json!({}),
            AuthenticationMethod::PrivateKey,
        ),
        (
            "private-key-password",
            Some("pw"),
            serde_json::json!({ "private_key_path": "/keys/id_ed25519" }),
            serde_json::json!({ "private_key_passphrase": "pp" }),
            AuthenticationMethod::PrivateKeyPassword,
        ),
        (
            "agent",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Agent,
        ),
        // Auto 按序回退不要求任何前置凭据：密码/私钥缺失只是对应阶段
        // 被跳过（回退链里的既有记录），解析层必须接受零凭据组合。
        (
            "auto",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Auto,
        ),
        (
            "none",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::None,
        ),
    ];
    for (authentication, password, external, secrets, expected) in minimal {
        let parsed = connection_with(authentication, *password, external.clone(), secrets.clone())
            .unwrap_or_else(|error| panic!("{authentication} minimal combo rejected: {error}"));
        assert_eq!(
            parsed.authentication, *expected,
            "minimal combo for '{authentication}' misparses"
        );
    }

    // ② 残留凭据容忍：表单里所有凭据字段都有值时（用户来回切换过
    //    认证方式），每种认证方式都必须照常解析且方法正确。
    let everything_external = serde_json::json!({ "private_key_path": "/keys/k" });
    let everything_secrets =
        serde_json::json!({ "private_key_passphrase": "pp", "totp_secret": "JBSWY3DPEHPK3PXP" });
    for (authentication, _, _, _, expected) in minimal {
        let parsed = connection_with(
            authentication,
            Some("pw"),
            everything_external.clone(),
            everything_secrets.clone(),
        )
        .unwrap_or_else(|error| {
            panic!("{authentication} leftover-credential combo rejected: {error}")
        });
        assert_eq!(
            parsed.authentication, *expected,
            "leftover-credential combo for '{authentication}' misparses"
        );
    }

    // ③ 必需凭据缺失 → 干净报错并指出缺什么。
    let missing_password = connection_with(
        "password",
        None,
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        missing_password.contains("requires a password"),
        "{missing_password}"
    );
    let missing_key = connection_with(
        "private-key",
        None,
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        missing_key.contains("requires a private key path"),
        "{missing_key}"
    );
    let half_of_private_key_password = connection_with(
        "private-key-password",
        None,
        serde_json::json!({ "private_key_path": "/keys/k" }),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        half_of_private_key_password.contains("requires a password"),
        "{half_of_private_key_password}"
    );
    let other_half = connection_with(
        "private-key-password",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        other_half.contains("requires a private key path"),
        "{other_half}"
    );

    // ④ 无法识别的认证值 → 明确报错（而非静默回落 password，那会把
    //    拼错的组合变成一次注定失败的密码登录）。
    let typo = connection_with(
        "publickey",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        typo.contains("Unsupported SSH authentication method 'publickey'"),
        "{typo}"
    );

    // ⑤ read_only × sudo_source 全矩阵：六个组合都必须可解析，
    //    sudo_enabled 只反映凭据来源——只读是正交的运行时门禁
    //    （ssh.rs 的 exec/sudo 通道负责拦截），解析层不得混淆二者。
    for read_only in [false, true] {
        for source in ["custom", "global", "off"] {
            let parsed = connection_with(
                "password",
                Some("pw"),
                serde_json::json!({ "sudo_source": source, "read_only": read_only }),
                serde_json::json!({}),
            )
            .unwrap_or_else(|error| {
                panic!("read_only={read_only} sudo_source={source} rejected: {error}")
            });
            assert_eq!(parsed.read_only, read_only, "sudo_source={source}");
            assert_eq!(
                parsed.sudo_enabled(),
                source != "off",
                "read_only={read_only} sudo_source={source}"
            );
        }
    }

    // ⑥ auth_flow_mode 交叉：sudo_source=global 时连接自身的 2FA 模式
    //    仍可解析（表单隐藏它，但登录期 keyboard-interactive 还要用）；
    //    password_only 携带 totp_secret 也合法——运行时静默忽略 OTP，
    //    不算配置冲突。
    for flow in [
        "password_only",
        "password_plus_otp",
        "password_then_otp",
        "ssh-agent-garbage",
    ] {
        for source in ["custom", "global", "off"] {
            let parsed = connection_with(
                "password",
                Some("pw"),
                serde_json::json!({
                    "sudo_source": source,
                    "auth_flow_mode": flow,
                    "totp_prompt_hint": "duo passcode"
                }),
                serde_json::json!({ "totp_secret": "JBSWY3DPEHPK3PXP" }),
            )
            .unwrap_or_else(|error| {
                panic!("auth_flow_mode={flow} sudo_source={source} rejected: {error}")
            });
            assert_eq!(parsed.auth_flow_mode, flow, "sudo_source={source}");
        }
    }

    // ⑦ 端口边界：0 与 65536 报错，1 与 65535 合法。
    for (port, ok) in [(0u64, false), (1, true), (65535, true), (65536, false)] {
        let connection = serde_json::json!({
            "id": "port-edge",
            "host": "example.com",
            "port": port,
            "username": "user",
            "password": "pw",
            "external_config": { "authentication": "password" }
        });
        let result = StoredConnection::from_lifecycle_params(
            &serde_json::json!({ "connection": connection }),
        );
        assert_eq!(
            result.is_ok(),
            ok,
            "port {port} should {}",
            if ok { "parse" } else { "be rejected" }
        );
    }

    // ⑧ connect_timeout_secs=0 钳到下限 1，而非让握手等待 0 秒后立即
    //    超时（表单允许输入 0，解析层负责收敛）。
    let zero_timeout = connection_with(
        "password",
        Some("pw"),
        serde_json::json!({ "connect_timeout_secs": 0 }),
        serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(zero_timeout.connect_timeout_secs, 1);
    assert!(zero_timeout.connect_timeout_explicit);

    // ⑧b 表单未填时跟随 manifest 默认（30s）：慢速 Windows/macOS 主机的
    //    握手+认证预算不再按旧默认 15s 一连接就打满。缺省同时标记为
    //    非显式——connection/test 的拨号预算据此对齐宿主 10s 回退截止。
    let default_timeout = connection_with(
        "password",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(default_timeout.connect_timeout_secs, 30);
    assert!(!default_timeout.connect_timeout_explicit);

    // ⑨ 跳板机不接受 none：链上每一跳都必须认证，"No authentication"
    //    只对最终会话合法。
    let none_hop = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "hop-none",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "pw",
            "external_config": {
                "authentication": "password",
                "jump_hosts": [
                    { "host": "bastion", "port": 22, "username": "ops", "authentication": "none" }
                ]
            }
        }
    }))
    .unwrap_err();
    assert!(
        none_hop.contains("Unsupported jump host authentication method 'none'"),
        "{none_hop}"
    );
}

/// 触发器（Expect）与外部密码管理器配置的解析面：external_config 的
/// triggers（对象/字符串两形态）、密文槽位解析、非法配置整连接失败
/// （D7）、password_command 让密码类认证允许留空（显式密码优先）。
#[test]
fn triggers_and_credential_commands_parse_strictly() {
    let lifecycle = |external_config: Value, secrets: Value| {
        let mut connection = serde_json::json!({
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "pw",
            "external_config": external_config,
        });
        if !secrets.is_null() {
            connection["connection_secrets"] = secrets;
        }
        StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": connection
        }))
    };

    // ① 对象形态（宿主 lifecycle / MCP 桥转发）：密文槽位解析成功。
    let parsed = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": {
                "timeoutSecs": 45,
                "sleepMs": 200,
                "passSleep": "enter",
                "stages": [
                    { "pattern": "code:", "sendSecretKey": "trigger_answer_1" },
                    { "pattern": "proceed", "sendText": "yes\\r",
                      "casePattern": "\\(y/n\\)", "caseSendText": "y" }
                ]
            },
            "password_command": "op read vault",
            "passphrase_command": "security find-password -w"
        }),
        serde_json::json!({ "trigger_answer_1": "one" }),
    )
    .unwrap();
    let triggers = parsed.triggers.expect("triggers must parse");
    assert_eq!(triggers.timeout_secs, 45);
    assert_eq!(triggers.sleep_ms, 200);
    assert_eq!(triggers.stages.len(), 2);
    assert_eq!(triggers.stages[0].answer.kind(), "secret");
    assert!(triggers.stages[1].case.is_some());
    assert_eq!(parsed.password_command, "op read vault");
    assert_eq!(parsed.passphrase_command, "security find-password -w");

    // ② 字符串形态（连接表单 textarea）同样解析。
    let parsed = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": r#"{"stages":[{"pattern":"code","sendText":"1\r"}]}"#
        }),
        Value::Null,
    )
    .unwrap();
    assert_eq!(parsed.triggers.expect("string form parses").stages.len(), 1);

    // ③ 独立开关默认关闭：旧的非空文本也必须不解析、不启用。
    let disabled_with_bad_text = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers": "{not json"
        }),
        Value::Null,
    )
    .unwrap();
    assert!(!disabled_with_bad_text.triggers_enabled);
    assert!(disabled_with_bad_text.triggers.is_none());

    // ④ 空配置仍表示关闭（即使显式打开开关也没有 engine）。
    for raw in [
        Value::Null,
        Value::from(""),
        serde_json::json!({ "stages": [] }),
    ] {
        let mut external = serde_json::json!({
            "authentication": "password"
        });
        if !raw.is_null() {
            external["triggers"] = raw;
        }
        let parsed = lifecycle(external, Value::Null).unwrap();
        assert!(parsed.triggers.is_none(), "expected disabled");
    }

    // ⑤ 非法 JSON / 未知密文槽位 → 连接失败并给出可定位的错误（D7）。
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": "{not json"
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("triggers"), "{error}");
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": { "stages": [{ "pattern": "p", "sendSecretKey": "nope" }] }
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("unknown secret slot"), "{error}");
    // 引用的槽位未填充同样失败（静默发空行比失败更难排查）。
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": { "stages": [{ "pattern": "p", "sendSecretKey": "trigger_answer_1" }] }
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("is empty"), "{error}");

    // ⑥ password_command 允许密码类认证留空密码（命令在连接期取回）。
    // 顶层 connection 无 password 字段、仅配置命令时解析成功。
    let parsed = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": {
                "authentication": "password",
                "password_command": "op read vault"
            }
        }
    }))
    .unwrap();
    assert!(parsed.password.is_empty());
    assert_eq!(parsed.password_command, "op read vault");
    // 两者都没有才拒绝。
    let error = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "password" }
        }
    }))
    .unwrap_err();
    assert!(error.contains("password_command"), "{error}");
}

/// Every manifest field must land in the store the model reads it from:
/// `secret` bindings come from `connection_secrets`, `config` bindings
/// from `external_config` and are consumed by the parser.
#[test]
fn bindings_match_parse_surfaces() {
    // private_key 现在被解析面消费（表单粘贴的私钥内容 / 外部工具写入的
    // 存量 secret），从 connection_secrets 读取。
    // trigger_answer_1/2 是触发器密文槽位，经 triggers 解析面消费（sendSecretKey 解析）。
    let secret_keys = [
        "private_key",
        "private_key_passphrase",
        "sudo_password",
        "totp_secret",
        "trigger_answer_1",
        "trigger_answer_2",
    ];
    let config_keys = [
        "protocol",
        "authentication",
        "private_key_path",
        "agent_socket",
        "advanced_options",
        "connect_timeout_secs",
        "keepalive_interval_secs",
        "terminal_keepalive_secs",
        "set_env",
        "triggers_enabled",
        "triggers",
        "password_command",
        "password_source",
        "passphrase_command",
        "remote_command",
        "sudo_source",
        "sudo_profile",
        "sudo_use_pty",
        "sudo_whitelist",
        "read_only",
        "auth_flow_mode",
        "password_prompt_hint",
        "totp_prompt_hint",
        // M32-B：serial/rdp 连接配置由前端 openSession 路由读取（不进
        // StoredConnection 凭据解析），但同为 external_config 键面。
        "serial_port",
        "serial_baud",
        "serial_data_bits",
        "serial_parity",
        "serial_stop_bits",
        "serial_backspace",
        "rdp_domain",
        "rdp_resolution",
        "rdp_certificate_policy",
        "rdp_clipboard",
    ];
    let mut seen_secret: Vec<String> = Vec::new();
    let mut seen_config: Vec<String> = Vec::new();
    for entry in connection_fields() {
        match entry["binding"].as_str().unwrap_or("") {
            "secret" => seen_secret.push(entry["key"].as_str().unwrap().to_string()),
            "config" => seen_config.push(entry["key"].as_str().unwrap().to_string()),
            "name" | "host" | "port" | "username" | "password" => {
                let key = entry["key"].as_str().unwrap();
                assert!(
                    matches!(
                        key,
                        "display_name" | "host" | "port" | "username" | "password"
                    ),
                    "unexpected binding for field '{key}'"
                );
            }
            other => panic!("unexpected binding '{other}' in manifest"),
        }
    }
    seen_secret.sort_unstable();
    seen_config.sort_unstable();
    let mut expected_secret: Vec<String> =
        secret_keys.iter().map(|value| value.to_string()).collect();
    expected_secret.sort_unstable();
    let mut expected_config: Vec<String> =
        config_keys.iter().map(|value| value.to_string()).collect();
    expected_config.sort_unstable();
    assert_eq!(
        seen_secret, expected_secret,
        "secret bindings must match connection_secrets keys"
    );
    assert_eq!(
        seen_config, expected_config,
        "config bindings must match external_config keys consumed by the parser"
    );
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

/// Manifest defaults must equal the parser's fallback defaults; feeding
/// them through `from_lifecycle_params` reproduces the same connection.
#[test]
fn defaults_match_parser_fallbacks() {
    let expected_defaults: &[(&str, Value)] = &[
        ("display_name", Value::from("SSH server")),
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
