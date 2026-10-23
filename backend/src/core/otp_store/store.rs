//! Metrics support.

use super::*;

pub(crate) const STORAGE_VERSION: u64 = 1;

pub(crate) const FILE_NAME: &str = "otp-entries.json";

pub(crate) const CRYPTO_SCHEME: &str = "aead-v1";

/// 条目数量上限（防滥用，与 quick-commands 的 20 条同一思路，放宽到库形态）。
pub const MAX_ENTRIES: usize = 100;

pub(crate) const MAX_ISSUER_LEN: usize = 120;

pub(crate) const MAX_USERNAME_LEN: usize = 120;

/// base32 解码后的密钥字节上限（对齐 triggers.rs 的 TOTP 密钥 abuse cap）。
pub(crate) const MAX_SECRET_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct OtpEntry {
    pub id: String,
    /// "totp" | "hotp"
    pub otp_type: String,
    pub issuer: String,
    pub username: String,
    /// vault 密文（`base64(nonce‖ct)`）；明文永不落盘。
    pub secret_encrypted: String,
    pub algorithm: OtpAlgorithm,
    pub digits: u8,
    pub period: u64,
    /// 仅 hotp：当前计数器（生成一次 +1 并持久化）。
    pub counter: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OtpStore {
    pub entries: Vec<OtpEntry>,
    /// `connection_id -> entry_id`。
    pub bindings: HashMap<String, String>,
}

pub(crate) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// 进程内插件数据目录（`SshRuntime::new` 注入）。自动应答的取码链
/// （`ssh.rs::sudo_auth_for`）是自由函数，拿不到 `self.data_dir`，从这里读。
pub(crate) static DATA_DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

/// 注入插件数据目录（启动时一次；测试可重复注入不同 tempdir）。
pub fn init_data_dir(path: &Path) {
    let slot = DATA_DIR.get_or_init(|| RwLock::new(None));
    if let Ok(mut guard) = slot.write() {
        *guard = Some(path.to_path_buf());
    }
}

/// 已注入的插件数据目录；未注入（极端：取码先于构造）返回 None。
pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR
        .get()
        .and_then(|slot| slot.read().ok().and_then(|guard| guard.clone()))
}

/// vault 构造：沿用 Quick Sudo 的 envelope `crypto.storage` 层级
/// （缺省 keyfile 0600；keychain 需显式 opt-in），同一把 DEK 覆盖所有库。
pub fn vault_for(data_dir: &Path) -> Vault {
    let storage = std::fs::read_to_string(store_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get("crypto")
                .and_then(|crypto| crypto.get("storage"))
                .and_then(Value::as_str)
                .and_then(vault::KeyStorage::parse)
        });
    let provider = vault::resolve_provider(storage, data_dir);
    Vault::new(provider.as_ref())
}

/// 加载存储；文件缺失或损坏一律空库（坏文件不能拖垮工作台）。
pub fn load_store(data_dir: &Path) -> OtpStore {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    let Some(value) = serde_json::from_str::<Value>(&text).ok() else {
        return OtpStore::default();
    };
    let entries = value
        .get("entries")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(entry_from_json).collect())
        .unwrap_or_default();
    let bindings = value
        .get("bindings")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(connection_id, entry_id)| {
                    entry_id
                        .as_str()
                        .map(|entry_id| (connection_id.clone(), entry_id.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    OtpStore { entries, bindings }
}

/// 原子落盘（tmp + rename），Unix 上 tmp 从创建起即 0600。
pub fn save_store(data_dir: &Path, store: &OtpStore) -> Result<(), String> {
    let path = store_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = json!({
        "version": STORAGE_VERSION,
        "crypto": { "scheme": CRYPTO_SCHEME },
        "entries": store.entries.iter().map(entry_json).collect::<Vec<_>>(),
        "bindings": store.bindings,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to encode otp entries: {error}"))?;
    // tmp 名掺 uuid：固定名的 `.json.tmp` 在两个并发 save_store 时会交错写
    // 同一个 tmp，损坏的 JSON 被 rename 发布后 load_store 按"损坏视为空"返回
    // 空库——用户的 OTP 条目静默丢失。
    let tmp = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    // 先建后写：`fs::write` 会先以平台默认权限（0644）落明文密文文件、
    // 再 chmod 0600，存在明文窗口；OpenOptions.mode 让首个字节前就是 0600。
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, text)
            .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

pub(crate) fn entry_from_json(value: &Value) -> Option<OtpEntry> {
    let object = value.as_object()?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let otp_type = match string("otpType").as_str() {
        "hotp" => "hotp".to_string(),
        _ => "totp".to_string(),
    };
    let is_hotp = otp_type == "hotp";
    let counter = object.get("counter").and_then(Value::as_u64);
    Some(OtpEntry {
        id: string("id"),
        otp_type,
        issuer: string("issuer"),
        username: string("username"),
        secret_encrypted: string("secretEnc"),
        algorithm: OtpAlgorithm::parse(&string("algorithm")),
        digits: object
            .get("digits")
            .and_then(Value::as_u64)
            .and_then(|digits| u8::try_from(digits).ok())
            .filter(|digits| (6..=8).contains(digits))
            .unwrap_or(6),
        period: object
            .get("period")
            .and_then(Value::as_u64)
            .filter(|period| (1..=3600).contains(period))
            .unwrap_or(30),
        counter: if is_hotp { counter } else { None },
    })
}

pub(crate) fn entry_json(entry: &OtpEntry) -> Value {
    json!({
        "id": entry.id,
        "otpType": entry.otp_type,
        "issuer": entry.issuer,
        "username": entry.username,
        "secretEnc": entry.secret_encrypted,
        "algorithm": entry.algorithm.as_str(),
        "digits": entry.digits,
        "period": entry.period,
        "counter": entry.counter,
    })
}

/// 脱敏视图：secret 永不出库，只回 `hasSecret`。
pub fn entry_view(entry: &OtpEntry) -> Value {
    json!({
        "id": entry.id,
        "otpType": entry.otp_type,
        "issuer": entry.issuer,
        "username": entry.username,
        "hasSecret": !entry.secret_encrypted.is_empty(),
        "algorithm": entry.algorithm.as_str(),
        "digits": entry.digits,
        "period": entry.period,
        "counter": entry.counter,
    })
}

pub fn list_views(store: &OtpStore) -> Vec<Value> {
    store.entries.iter().map(entry_view).collect()
}

pub fn binding_views(store: &OtpStore) -> Value {
    Value::Object(
        store
            .bindings
            .iter()
            .map(|(connection_id, entry_id)| {
                (connection_id.clone(), Value::from(entry_id.as_str()))
            })
            .collect(),
    )
}

pub(crate) fn text_input(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}

/// 数值参数读取：JSON number 与数字字符串都接受（前端两种形态都会发）。
pub(crate) fn number_input(params: &Value, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} must be a non-negative integer")),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => text
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{key} must be an integer, got '{text}'")),
        Some(_) => Err(format!("{key} must be an integer")),
    }
}

/// 创建或更新一个条目。`secret` 非空 → 校验 base32 后重新加密；为空 → 保留
/// 旧密文（编辑 issuer/用户名不动密钥的常见路径）。新建必须带非空 secret。
/// hotp 必须带 `counter`；totp 忽略该参数。返回 (条目, 是否新建)。
pub fn save_entry(
    store: &mut OtpStore,
    params: &Value,
    vault: &Vault,
) -> Result<(OtpEntry, bool), String> {
    let otp_type_input = text_input(params, "otpType").unwrap_or_default();
    let otp_type: &str = match otp_type_input.as_str() {
        "" | "totp" => "totp",
        "hotp" => "hotp",
        other => return Err(format!("otpType must be totp or hotp, got '{other}'")),
    };
    let issuer = text_input(params, "issuer").unwrap_or_default();
    let username = text_input(params, "username").unwrap_or_default();
    if issuer.chars().count() > MAX_ISSUER_LEN {
        return Err(format!("issuer is limited to {MAX_ISSUER_LEN} characters"));
    }
    if username.chars().count() > MAX_USERNAME_LEN {
        return Err(format!(
            "username is limited to {MAX_USERNAME_LEN} characters"
        ));
    }
    let algorithm = text_input(params, "algorithm")
        .map(|value| OtpAlgorithm::parse(&value))
        .unwrap_or(OtpAlgorithm::Sha1);
    let digits = match number_input(params, "digits")? {
        None => None,
        Some(value) => Some(
            u8::try_from(value)
                .ok()
                .filter(|digits| (6..=8).contains(digits))
                .ok_or_else(|| format!("digits must be 6-8, got '{value}'"))?,
        ),
    };
    let period = match number_input(params, "period")? {
        None => None,
        Some(value) => {
            if !(1..=3600).contains(&value) {
                return Err(format!("period must be 1-3600, got '{value}'"));
            }
            Some(value)
        }
    };
    let counter_input = number_input(params, "counter")?;
    let secret_input = text_input(params, "secret").unwrap_or_default();

    let existing_id = params
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let existing = existing_id.and_then(|id| store.entries.iter().position(|entry| entry.id == id));
    // 新建 hotp 必须带起始 counter；更新时缺省沿用旧值（见下方 or 兜底）。
    if otp_type == "hotp" && counter_input.is_none() && existing.is_none() {
        return Err("hotp entries require a counter".to_string());
    }

    // 条目 id 先定下来：vault 的 AAD 绑定 `totpSecret|<entryId>`，新建的
    // 密文必须按最终 id 封装。
    let id = match existing {
        Some(index) => store.entries[index].id.clone(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let secret_encrypted = if secret_input.is_empty() {
        match existing {
            // 编辑 issuer/用户名等不动密钥：保留旧密文。
            Some(index) => store.entries[index].secret_encrypted.clone(),
            None => return Err("Missing secret".to_string()),
        }
    } else {
        let key = otp::decode_base32(&secret_input)?;
        if key.len() > MAX_SECRET_BYTES {
            return Err(format!(
                "secret exceeds {MAX_SECRET_BYTES} bytes after base32 decoding"
            ));
        }
        vault.seal(FIELD_TOTP_SECRET, &id, &normalize_secret(&secret_input))
    };

    let entry = OtpEntry {
        id,
        otp_type: otp_type.to_string(),
        issuer,
        username,
        secret_encrypted,
        algorithm,
        digits: digits.unwrap_or(6),
        period: period.unwrap_or(30),
        counter: if otp_type == "hotp" {
            counter_input.or(match existing {
                Some(index) => store.entries[index].counter,
                None => None,
            })
        } else {
            None
        },
    };
    match existing {
        Some(index) => store.entries[index] = entry.clone(),
        None => {
            if store.entries.len() >= MAX_ENTRIES {
                return Err(format!("At most {MAX_ENTRIES} OTP entries are supported"));
            }
            store.entries.push(entry.clone());
        }
    }
    let created = existing.is_none();
    Ok((entry, created))
}

/// 规范化密钥文本（大写、去空格/连字符、无填充），加密前统一形态。
pub(crate) fn normalize_secret(text: &str) -> String {
    data_encoding::BASE32_NOPAD.encode(otp::decode_base32(text).unwrap_or_default().as_slice())
}

/// 删除条目；指向它的绑定一并清除。未知 id 返回 false。
pub fn delete_entry(store: &mut OtpStore, id: &str) -> bool {
    let before = store.entries.len();
    store.entries.retain(|entry| entry.id != id);
    let removed = store.entries.len() != before;
    store.bindings.retain(|_, entry_id| entry_id != id);
    removed
}

/// 绑定连接 → 条目；条目必须存在（同一连接重复绑定即改绑）。
pub fn bind(store: &mut OtpStore, connection_id: &str, entry_id: &str) -> Result<(), String> {
    if connection_id.is_empty() {
        return Err("Missing connectionId".to_string());
    }
    if !store.entries.iter().any(|entry| entry.id == entry_id) {
        return Err(format!("OTP entry '{entry_id}' not found"));
    }
    store
        .bindings
        .insert(connection_id.to_string(), entry_id.to_string());
    Ok(())
}

/// 解绑；本来就没绑返回 false。
pub fn unbind(store: &mut OtpStore, connection_id: &str) -> bool {
    store.bindings.remove(connection_id).is_some()
}

/// 解出条目密钥（vault.open → base32 解码）。密文为空（DEK 丢失降级、
/// 保存即空）或解码失败返回 None——自动应答回落静默跳过，绝不打断连接。
pub fn get_decrypted_secret(vault: &Vault, entry: &OtpEntry) -> Option<Vec<u8>> {
    let plaintext = vault.open(FIELD_TOTP_SECRET, &entry.id, &entry.secret_encrypted);
    if plaintext.is_empty() {
        return None;
    }
    otp::decode_base32(&plaintext)
        .ok()
        .filter(|key| !key.is_empty())
}
