//! Stream session importer.

use super::*;

/// In-memory upload state. The source bytes and master password are wrapped in
/// `Zeroizing`, so removal from the state map wipes their backing buffers.
pub(super) struct ImportUpload {
    kind: String,
    main: ImportFile,
    user_config: Option<ImportFile>,
    master_password: Option<Zeroizing<String>>,
    created_at: Instant,
    reserved_bytes: u64,
}

pub(super) struct ImportFile {
    expected: u64,
    bytes: Zeroizing<Vec<u8>>,
}

impl ImportFile {
    fn new(expected: u64) -> ImportFile {
        ImportFile {
            expected,
            bytes: Zeroizing::new(Vec::with_capacity(usize::try_from(expected).unwrap_or(0))),
        }
    }
}

#[derive(Default)]
pub(super) struct ImportState {
    uploads: HashMap<String, ImportUpload>,
    reserved_bytes: u64,
}

/// Owns temporary preview data. Every state exit (finish, cancel, protocol
/// error and TTL sweep) drops `Zeroizing` source bytes and WindTerm passwords.
#[derive(Default)]
pub struct ImportStream {
    state: Mutex<ImportState>,
}

impl ImportStream {
    pub fn start(&self, params: &Value) -> Result<Value, String> {
        let kind = params
            .get("kind")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("import/preview/start: kind is required")?;
        validate_kind(kind)?;
        let main_size = required_size(params, "mainSize")?;
        let user_config_size = params.get("userConfigSize").and_then(Value::as_u64);
        if kind != "windterm" && user_config_size.is_some() {
            return Err(
                "import/preview/start: userConfigSize is only valid for WindTerm".to_string(),
            );
        }
        let reserved_bytes = main_size
            .checked_add(user_config_size.unwrap_or(0))
            .ok_or("import/preview/start: size overflow")?;
        if reserved_bytes > MAX_INPUT_BYTES as u64 {
            return Err(format!(
                "Import files exceed the {} MiB total limit",
                MAX_INPUT_BYTES / (1024 * 1024)
            ));
        }
        let master_password = params
            .get("masterPassword")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| Zeroizing::new(value.to_owned()));
        if master_password
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
        {
            return Err(
                "import/preview/start: masterPassword exceeds the 4096 byte limit".to_string(),
            );
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "import preview state is unavailable")?;
        expire_locked(&mut state, Instant::now());
        if state.uploads.len() >= MAX_IMPORT_TASKS {
            return Err("Too many active import previews".to_string());
        }
        if state.reserved_bytes.saturating_add(reserved_bytes) > MAX_IMPORT_MEMORY_BYTES {
            return Err("Active import previews exceed the process memory budget".to_string());
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        state.reserved_bytes += reserved_bytes;
        state.uploads.insert(
            task_id.clone(),
            ImportUpload {
                kind: kind.to_string(),
                main: ImportFile::new(main_size),
                user_config: user_config_size.map(ImportFile::new),
                master_password,
                created_at: Instant::now(),
                reserved_bytes,
            },
        );
        Ok(json!({ "taskId": task_id, "chunkSize": IMPORT_CHUNK_LIMIT }))
    }

    pub fn append(&self, task_id: &str, part: &str, frame: &[u8]) -> Result<u64, String> {
        let result = (|| {
            if frame.len() < 8 {
                return Err("import preview chunk is missing its 8-byte offset".to_string());
            }
            let offset = u64::from_be_bytes(frame[..8].try_into().expect("slice length checked"));
            let payload = &frame[8..];
            if payload.len() > IMPORT_CHUNK_LIMIT {
                return Err(format!(
                    "import preview chunk of {} bytes exceeds the {} byte limit",
                    payload.len(),
                    IMPORT_CHUNK_LIMIT
                ));
            }
            let mut state = self
                .state
                .lock()
                .map_err(|_| "import preview state is unavailable")?;
            expire_locked(&mut state, Instant::now());
            let upload = state
                .uploads
                .get_mut(task_id)
                .ok_or("import preview task was not found")?;
            let file = match part {
                "main" => &mut upload.main,
                "user-config" => upload
                    .user_config
                    .as_mut()
                    .ok_or("import preview task has no user.config stream")?,
                _ => return Err("unknown import preview binary stream".to_string()),
            };
            let expected_offset = file.bytes.len() as u64;
            if offset != expected_offset {
                return Err(format!("import preview chunk offset {offset} does not match expected {expected_offset}"));
            }
            let next_offset = offset
                .checked_add(payload.len() as u64)
                .ok_or("import preview chunk offset overflow")?;
            if next_offset > file.expected {
                return Err("import preview chunk exceeds declared file size".to_string());
            }
            file.bytes.extend_from_slice(payload);
            Ok(next_offset)
        })();
        if result.is_err() {
            self.clear(task_id);
        }
        result
    }

    pub fn finish(&self, task_id: &str) -> Result<Value, String> {
        let upload = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "import preview state is unavailable")?;
            expire_locked(&mut state, Instant::now());
            remove_upload(&mut state, task_id).ok_or("import preview task was not found")?
        };
        if upload.main.bytes.len() as u64 != upload.main.expected {
            return Err("import preview main file is incomplete".to_string());
        }
        if let Some(config) = &upload.user_config {
            if config.bytes.len() as u64 != config.expected {
                return Err("import preview user.config file is incomplete".to_string());
            }
        }
        let sessions = parse_uploaded(
            &upload.kind,
            &upload.main.bytes,
            upload
                .user_config
                .as_ref()
                .map(|config| config.bytes.as_slice()),
            upload
                .master_password
                .as_deref()
                .map(|value| value.as_str()),
        )?;
        let visible = sessions.len().min(MAX_PREVIEW_SESSIONS);
        let selected = (0..visible).collect::<Vec<_>>();
        let normalized = normalized_export(&upload.kind, &sessions, &selected)?;
        let preview = normalized["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, session)| preview_view(index, session))
            .collect::<Vec<_>>();
        Ok(
            json!({ "sourceKind": upload.kind, "sessions": preview, "totalSessions": sessions.len(), "truncated": sessions.len() > visible, "export": normalized }),
        )
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        self.clear(task_id)
    }

    /// Deterministic testable TTL sweep; every public operation invokes it.
    #[cfg(test)]
    pub fn expire_before(&self, now: Instant) -> bool {
        self.state
            .lock()
            .map(|mut state| expire_locked(&mut state, now) > 0)
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(super) fn active_task_count(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.uploads.len())
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(super) fn reserved_bytes(&self) -> u64 {
        self.state
            .lock()
            .map(|state| state.reserved_bytes)
            .unwrap_or(0)
    }

    fn clear(&self, task_id: &str) -> bool {
        self.state
            .lock()
            .map(|mut state| {
                expire_locked(&mut state, Instant::now());
                remove_upload(&mut state, task_id).is_some()
            })
            .unwrap_or(false)
    }
}

pub(super) fn remove_upload(state: &mut ImportState, task_id: &str) -> Option<ImportUpload> {
    let upload = state.uploads.remove(task_id)?;
    state.reserved_bytes = state.reserved_bytes.saturating_sub(upload.reserved_bytes);
    Some(upload)
}

pub(super) fn expire_locked(state: &mut ImportState, now: Instant) -> usize {
    let expired = state
        .uploads
        .iter()
        .filter(|(_, upload)| now.saturating_duration_since(upload.created_at) >= IMPORT_TASK_TTL)
        .map(|(task_id, _)| task_id.clone())
        .collect::<Vec<_>>();
    for task_id in &expired {
        let _ = remove_upload(state, task_id);
    }
    expired.len()
}

pub(super) fn required_size(params: &Value, field: &str) -> Result<u64, String> {
    params
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("import/preview/start: {field} is required"))
}

pub(super) fn validate_kind(kind: &str) -> Result<(), String> {
    match kind {
        "moba" | "xshell" | "windterm" | "securecrt" | "finalshell" | "electerm" | "termius"
        | "sshconfig" => Ok(()),
        _ => Err(format!("Unknown import kind: {kind}")),
    }
}

/// kind → 解析器分发。流式预览（`import/preview/finish`）与连接表单的
/// 「导入会话」动作（`connection/action import-sessions`）共用同一套解析器，
/// 两边的容量上限与脱敏语义因此必然一致。
pub fn parse_uploaded(
    kind: &str,
    file: &[u8],
    user_config: Option<&[u8]>,
    master_password: Option<&str>,
) -> Result<Vec<ImportedSession>, String> {
    match kind {
        "moba" => parse_moba_ini(
            std::str::from_utf8(file).map_err(|_| "MobaXterm file is not valid UTF-8")?,
        ),
        "xshell" => parse_xshell(file),
        "windterm" => parse_windterm(file, user_config, master_password),
        "securecrt" => parse_securecrt(
            std::str::from_utf8(file).map_err(|_| "SecureCRT file is not valid UTF-8")?,
        ),
        "finalshell" => parse_finalshell(file),
        "electerm" => parse_electerm(file),
        "termius" => parse_termius(file),
        "sshconfig" => parse_ssh_config(
            std::str::from_utf8(file).map_err(|_| "OpenSSH config file is not valid UTF-8")?,
        ),
        _ => Err(format!("Unknown import kind: {kind}")),
    }
}

/// 按扩展名嗅探导入来源，供连接表单的「导入会话」动作（`connection/action
/// import-sessions`）在无 UI 的场景下替代向导的来源选择。无扩展名/未识别
/// 扩展名一律回落 OpenSSH config——`~/.ssh/config` 没有扩展名是常态，选中
/// 的文件到底是不是会话文件交给解析器判定。扩展名大小写不敏感。
pub fn sniff_kind(file_name: &str, bytes: &[u8]) -> &'static str {
    let lower = file_name.to_lowercase();
    if lower.ends_with(".mxtsessions") {
        "moba"
    } else if lower.ends_with(".xts") {
        "xshell"
    } else if lower.ends_with(".sessions") {
        "windterm"
    } else if lower.ends_with(".xml") {
        "securecrt"
    } else if lower.ends_with(".zip") {
        "finalshell"
    } else if lower.ends_with(".json") {
        sniff_json_kind(bytes)
    } else {
        "sshconfig"
    }
}

/// `.json` 是 Electerm 与 Termius 共用的扩展名，按顶层形态分流：Termius 有
/// `hosts` 数组（顶层或 `data` 下），Electerm 是顶层数组或 `bookmarks` 对象。
/// 坏 JSON 也归 Electerm，让解析器报出与向导一致的原文错误。
pub(super) fn sniff_json_kind(bytes: &[u8]) -> &'static str {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => {
            if value.get("hosts").is_some()
                || value
                    .get("data")
                    .and_then(|data| data.get("hosts"))
                    .is_some()
            {
                "termius"
            } else {
                "electerm"
            }
        }
        Err(_) => "electerm",
    }
}

/// 连接表单「导入会话」动作的读盘入口：与流式预览同一个 64 MiB 总预算，
/// 超限直接拒绝而不是读一半。返回嗅探出的 kind 与原始字节，调用方喂给
/// [`parse_uploaded`]。
pub fn read_session_file(path: &std::path::Path) -> Result<(&'static str, Vec<u8>), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Cannot read '{}': {error}", path.display()))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".to_string());
    }
    if metadata.len() > MAX_INPUT_BYTES as u64 {
        return Err(format!(
            "Import files exceed the {} MiB total limit",
            MAX_INPUT_BYTES / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Cannot read '{}': {error}", path.display()))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((sniff_kind(&file_name, &bytes), bytes))
}

/// 把一条解析结果映射为连接表单 `fieldValues`（键 = manifest 字段 key）。凭据
/// 结构性缺位，与预览/导出模型同一条红线：密码永不回填（`authentication`
/// 只声明类别，密码留空由宿主在连接时问），私钥只回路径不回内容。
pub fn session_field_values(session: &ImportedSession) -> Value {
    let mut values = json!({
        "display_name": preview_text(&session.name),
        "host": preview_text(&session.host),
        "port": session.port,
        "username": preview_text(&session.username),
        "authentication": auth_kind(&session.auth),
    });
    if let ImportedAuth::PrivateKey {
        path: Some(path), ..
    } = &session.auth
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            values["private_key_path"] = json!(trimmed);
        }
    }
    values
}

/// Builds a portable, normalized export with metadata only. Passwords, private
/// key contents and passphrases are structurally absent even when source files
/// held them in plaintext.
pub fn normalized_export(
    source_kind: &str,
    sessions: &[ImportedSession],
    selected: &[usize],
) -> Result<Value, String> {
    validate_kind(source_kind)?;
    if selected.len() > MAX_PREVIEW_SESSIONS {
        return Err(format!(
            "An export may contain at most {MAX_PREVIEW_SESSIONS} sessions"
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::with_capacity(selected.len());
    for index in selected {
        if !seen.insert(*index) {
            continue;
        }
        let session = sessions
            .get(*index)
            .ok_or_else(|| format!("Selected preview index {index} is invalid"))?;
        rows.push(normalized_session(session));
    }
    Ok(json!({ "schemaVersion": 1, "sourceKind": source_kind, "sessions": rows }))
}

pub(super) fn normalized_session(session: &ImportedSession) -> Value {
    let key_path = match &session.auth {
        ImportedAuth::PrivateKey { path, .. } => path.as_deref().unwrap_or_default(),
        _ => "",
    };
    json!({
        "name": preview_text(&session.name),
        "host": preview_text(&session.host),
        "port": session.port,
        "username": preview_text(&session.username),
        "groupPath": session.group_path.iter().map(|part| preview_text(part)).collect::<Vec<_>>(),
        "description": preview_text(&session.description),
        "auth": {
            "kind": auth_kind(&session.auth),
            "hasSecret": has_secret(&session.auth),
            "keyPath": preview_text(key_path),
            "secretNote": session.secret_note,
        },
    })
}

pub(super) fn preview_view(index: usize, session: &Value) -> Value {
    json!({
        "index": index,
        "name": session["name"],
        "host": session["host"],
        "port": session["port"],
        "username": session["username"],
        "groupPath": session["groupPath"].as_array().map(|parts| parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("/")).unwrap_or_default(),
        "description": session["description"],
        "authKind": session["auth"]["kind"],
        "hasSecret": session["auth"]["hasSecret"],
        "secretNote": session["auth"]["secretNote"],
    })
}

pub(super) fn preview_text(value: &str) -> String {
    // All-source shared exit: hostile config / JSON fields may carry ANSI ESC
    // sequences; the preview table, the sanitized export JSON and log viewers
    // are escape-capable downstream consumers, so strip before the length cap.
    let value = crate::sessions::ssh::exec::strip_ansi_control_sequences(value);
    if value.len() <= MAX_PREVIEW_TEXT_BYTES {
        return value;
    }
    let mut end = MAX_PREVIEW_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// Removes the obsolete pre-streaming connection store at startup. It is not
/// migrated because the new contract never retains imported connections.
pub fn remove_legacy_store(data_dir: &Path) -> Result<(), String> {
    let legacy = data_dir.join(LEGACY_STORE_FILE);
    match std::fs::remove_file(legacy) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Failed to remove legacy import store".to_string()),
    }
}
