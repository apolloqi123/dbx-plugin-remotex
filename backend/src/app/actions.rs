//! Connection-form action helpers shared by the connection RPC surface.

use super::*;

pub(super) fn trigger_input_format(raw: Option<&Value>) -> &'static str {
    match raw {
        None | Some(Value::Null) => "empty",
        Some(Value::Object(_)) => "json",
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                "empty"
            } else if serde_json::from_str::<serde_json::Map<String, Value>>(trimmed).is_ok() {
                "json"
            } else if trimmed.lines().any(|line| {
                line.trim_start_matches(['#', '!', ' '])
                    .starts_with("Expect")
            }) {
                "tssh"
            } else {
                "json"
            }
        }
        Some(_) => "invalid",
    }
}

/// DBX Host sends connection actions as `{ "action": { "id": "..." } }`.
/// Accept the pre-Host-API-1.1 string form as well so older callers remain
/// compatible while the plugin follows the current host contract.
pub(super) fn connection_action_id(params: &Value) -> Result<&str, String> {
    let action = params
        .get("action")
        .ok_or_else(|| "Missing action".to_string())?;
    action
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| action.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing action id".to_string())
}

/// 「从文件导入私钥」动作核心：`pick` 抽象系统文件框（生产传
/// `crate::filesystem::local::fs::pick_file`，单测注入固定返回），让取消臂的「整体 null 安静
/// 返回」契约可脱离真实对话框单测。桌面门禁留在分派臂边界。
pub(super) fn action_import_private_key(
    pick: &mut dyn FnMut() -> Result<Option<String>, String>,
) -> Result<Value, String> {
    match pick()? {
        // 取消选文件：宿主契约 fieldValues 拒绝 null，
        // 整体返回 null（安静无操作）而不是空回填对象。
        None => Ok(json!(null)),
        Some(path) => {
            let content = crate::core::keys::read_private_key_file(std::path::Path::new(&path))?;
            Ok(json!({
                "message": format!("Imported private key from {path}"),
                "fieldValues": { "private_key": content },
            }))
        }
    }
}

/// 「从终端会话文件导入」动作核心（picker 注入理由同上）；`path` 参数跳过
/// 系统对话框，供冒烟/自测注入固定文件（宿主从不携带；载荷仍只有脱敏
/// 字段，与预览同边界）。
pub(super) fn action_import_sessions(
    params: &Value,
    pick_source: &mut dyn FnMut() -> Result<Option<usize>, String>,
    pick_file: &mut dyn FnMut(
        &crate::filesystem::local::fs::ImportSourceSpec,
    ) -> Result<Option<String>, String>,
) -> Result<Value, String> {
    // `path`（宿主从不携带，冒烟/单测注入）跳过两段对话框、直接读盘嗅探；
    // `source` 只跳过来源选择段，文件框照常弹出（可再与 path 组合）。
    let (kind, bytes) = match params
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(path) => crate::features::import::read_session_file(std::path::Path::new(path))?,
        None => {
            let source = match params
                .get("source")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                Some(kind) => crate::filesystem::local::fs::import_source_by_kind(kind)
                    .ok_or_else(|| format!("Unknown import source: {kind}"))?,
                None => {
                    let Some(index) = pick_source()? else {
                        // 取消来源选择：同取消选文件，整体 null 安静返回。
                        return Ok(json!(null));
                    };
                    crate::filesystem::local::fs::IMPORT_SOURCES
                        .get(index)
                        .ok_or_else(|| "Import source selection is out of range".to_string())?
                }
            };
            let Some(path) = pick_file(source)? else {
                // 取消选文件：同 import-private-key，整体 null 安静返回。
                return Ok(json!(null));
            };
            crate::features::import::read_session_file(std::path::Path::new(&path))?
        }
    };
    let sessions = match crate::features::import::parse_uploaded(kind, &bytes, None, None) {
        Ok(sessions) => sessions,
        Err(error) if error == crate::features::import::WINDTERM_MASTER_PASSWORD_REQUIRED => {
            return Err(format!(
                "{error} — use the import wizard in the workbench, which can ask for it"
            ));
        }
        Err(error) => return Err(error),
    };
    let Some(session) = sessions.first() else {
        return Err(
            "No SSH sessions were found in the selected file — expected a MobaXterm/Xshell/WindTerm/SecureCRT/FinalShell/Electerm/Termius export or an OpenSSH config"
                .to_string(),
        );
    };
    let total = sessions.len();
    let name = session.name.trim();
    let name = if name.is_empty() {
        session.host.as_str()
    } else {
        name
    };
    let message = if total > 1 {
        format!("Imported '{name}' (1 of {total} sessions); use the workbench import wizard for the rest")
    } else {
        format!("Imported '{name}'")
    };
    Ok(json!({
        "message": message,
        "fieldValues": crate::features::import::session_field_values(session),
    }))
}

/// telnet 表单凭据 → 声明式自动登录：提示词/成败正则全部走内置默认词表
/// （`DeclarativeAutoLogin` 的 None 语义），用户名为空时只应答密码提问。
pub(super) fn telnet_form_auto_login(
    username: &str,
    password: &str,
) -> crate::sessions::telnet::AutoLoginSpec {
    crate::sessions::telnet::AutoLoginSpec {
        rules: None,
        secrets: Vec::new(),
        declarative: Some(crate::sessions::telnet::DeclarativeAutoLogin {
            username: (!username.is_empty()).then_some(username.to_string()),
            password: Some(password.to_string()),
            username_prompt_regex: None,
            password_prompt_regex: None,
            success_regex: None,
            failure_regex: None,
            max_retries: None,
        }),
    }
}

/// `connection/protocol` 的响应组装：本地 Shell 目标（`io.dbx.remotex.local`）
/// 优先——宿主侧要求每个 connection provider 可打开（声明 workbench 或
/// filesystem provider），本地目标打开的是 Terminal 工作台，webview 需要据
/// protocol="local" 路由到 local/terminal/start 而非误拨 SSH；shell 透传连接
/// 表单值（空 = 前端回落偏好/默认 Shell 解析链），cwd/env 同样透传连接表单
/// 配置（`env` 为净化后的 name/value 行数组，空数组 = 未配置）。非本地连接按序
/// 回落：非 SSH 凭据注册表的会话协议（telnet/vnc/rdp/serial）→ 已登记的 SSH 连接
/// （`ssh`，宿主 lifecycle 推送先于 webview 查询，首个查询即可命中）→ 空串（宿主
/// 尚未推送该连接，调用方短重试）。
pub(super) fn connection_protocol_payload(
    local_target: Option<crate::sessions::local::target::LocalTarget>,
    stored_protocol: Option<String>,
    ssh_connection: bool,
) -> Value {
    match local_target {
        Some(target) => json!({
            "protocol": "local",
            "shell": target.shell,
            "cwd": target.cwd,
            "env": target
                .env
                .iter()
                .map(|(name, value)| json!({ "name": name, "value": value }))
                .collect::<Vec<_>>(),
        }),
        None => {
            let protocol = match stored_protocol {
                Some(protocol) => protocol,
                None if ssh_connection => "ssh".to_string(),
                None => String::new(),
            };
            json!({ "protocol": protocol })
        }
    }
}

/// Reads an optional non-negative integer parameter, falling back to the
/// default when the key is absent or not a `u64` (negative/invalid).
pub(super) async fn tcp_probe(host: &str, port: u16) -> Result<(), String> {
    const TCP_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    match tokio::time::timeout(
        TCP_PROBE_TIMEOUT,
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!("connect {host}:{port}: {error}")),
        Err(_) => Err(format!("connect {host}:{port} timed out after 10s")),
    }
}
