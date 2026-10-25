use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use russh::keys::decode_secret_key;
use russh::keys::ssh_key::{Algorithm, HashAlg, PrivateKey as OpensshPrivateKey};
use serde::Serialize;
use serde_json::{json, Value};

/// `connection/action` 的「从文件导入私钥」：读取并校验密钥文件，返回原文
/// 供回填表单的 private_key 字段（保存时进 vault）。识别规则与 discover
/// 一致（OpenSSH/PEM/PPK 头 + 可解码性）；加密私钥合法——口令走表单另一
/// 字段。公钥、非密钥文本、超大文件一律拒绝。
pub const IMPORT_MAX_BYTES: u64 = 64 * 1024;

pub fn read_private_key_file(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Cannot read '{}': {error}", path.display()))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".to_string());
    }
    if metadata.len() > IMPORT_MAX_BYTES {
        return Err(format!(
            "The selected file is too large to be a private key (limit {IMPORT_MAX_BYTES} bytes)"
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Cannot read '{}': {error}", path.display()))?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with("-----BEGIN") && !trimmed.starts_with("PuTTY-User-Key-File-") {
        return Err("Not a private key file (OpenSSH/PEM/PPK expected)".to_string());
    }
    if trimmed.contains(" PUBLIC KEY-----") {
        return Err("That is a public key; select the private key file".to_string());
    }
    match decode_secret_key(&text, None) {
        Ok(_) => Ok(text),
        Err(error) => {
            // 加密私钥：OpenSSH 报 KeyIsEncrypted，PKCS#1/8 带 ENCRYPTED 标记——
            // 均视为合法，口令在表单里填。
            let encrypted =
                matches!(error, russh::keys::Error::KeyIsEncrypted) || text.contains("ENCRYPTED");
            if encrypted {
                Ok(text)
            } else {
                // 与连接期同一套可操作错误分类（#21），底层错误原样保留。
                Err(super::decode_error::private_key_decode_failure(
                    &text, &error, false,
                ))
            }
        }
    }
}

/// Candidate file names OpenSSH clients try in `~/.ssh` by default.
const CONVENTIONAL_KEY_NAMES: &[&str] = &[
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    "id_ecdsa_sk",
    "id_ed25519_sk",
    "identity",
];

/// A private key found on this machine. Only metadata travels in this
/// structure; the key material itself never leaves the file it lives in.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredKey {
    pub path: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub has_passphrase: bool,
}

/// Finds private keys under `$HOME/.ssh`: the conventional `id_*`/`identity`
/// names, any `*.pem`/`*.key` file, and every `IdentityFile` mentioned in
/// `~/.ssh/config`. Files that cannot be decoded as private keys are skipped
/// without failing the whole scan, like tiny-rdm's DiscoverKeys.
pub fn discover() -> Result<Vec<DiscoveredKey>, String> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| "Home directory could not be found".to_string())?;
    Ok(discover_in(&PathBuf::from(home).join(".ssh")))
}

/// `keys/discover/options`: data source for a host-rendered key dropdown
/// (manifest `options_action`). `value` and `label` are both the absolute key
/// path: the path alone is the shortest text that still tells two same-named
/// keys apart, and a label without algorithm/fingerprint cannot stretch the
/// control. Algorithm and fingerprint stay in `keys/discover`. Only metadata
/// travels here — never key material.
pub fn discover_options() -> Result<Value, String> {
    let options: Vec<Value> = discover()?
        .into_iter()
        .map(|key| json!({ "value": key.path.clone(), "label": key.path }))
        .collect();
    Ok(json!({ "options": options }))
}

/// Scans `ssh_dir` (normally `~/.ssh`) for private keys. An unreadable or
/// missing directory simply yields no results.
pub(super) fn discover_in(ssh_dir: &Path) -> Vec<DiscoveredKey> {
    let mut candidates = BTreeSet::new();

    for name in CONVENTIONAL_KEY_NAMES {
        let path = ssh_dir.join(name);
        if path.is_file() {
            candidates.insert(path);
        }
    }

    if let Ok(entries) = std::fs::read_dir(ssh_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if name.ends_with(".pub") {
                continue;
            }
            let looks_like_key =
                name.starts_with("id_") || name.ends_with(".pem") || name.ends_with(".key");
            if looks_like_key {
                candidates.insert(path);
            }
        }
    }

    if let Ok(config) = std::fs::read_to_string(ssh_dir.join("config")) {
        for path in identity_files_from_config(&config, ssh_dir) {
            if path.is_file() {
                candidates.insert(path);
            }
        }
    }

    let mut keys: Vec<DiscoveredKey> = candidates
        .iter()
        .filter_map(|path| probe_key(path))
        .collect();
    keys.sort_by(|a, b| a.path.cmp(&b.path));
    keys
}

/// Probes one file for a decodable private key, returning metadata only.
/// `None` means "not a private key we support" and the caller skips it.
fn probe_key(path: &Path) -> Option<DiscoveredKey> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with("-----BEGIN") && !trimmed.starts_with("PuTTY-User-Key-File-") {
        return None;
    }
    if trimmed.contains(" PUBLIC KEY-----") {
        return None;
    }

    match decode_secret_key(&text, None) {
        Ok(key) => Some(DiscoveredKey {
            path: path.display().to_string(),
            algorithm: algorithm_name(key.algorithm()),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            has_passphrase: false,
        }),
        Err(error) => {
            // Encrypted OpenSSH keys report KeyIsEncrypted; legacy PKCS#1 and
            // PKCS#8 envelopes carry an ENCRYPTED marker instead.
            let encrypted =
                matches!(error, russh::keys::Error::KeyIsEncrypted) || text.contains("ENCRYPTED");
            if !encrypted {
                return None;
            }
            // The OpenSSH format stores the public half in the clear, so the
            // algorithm and fingerprint survive even without the passphrase.
            if let Ok(key) = OpensshPrivateKey::from_openssh(&text) {
                return Some(DiscoveredKey {
                    path: path.display().to_string(),
                    algorithm: algorithm_name(key.algorithm()),
                    fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                    has_passphrase: true,
                });
            }
            Some(DiscoveredKey {
                path: path.display().to_string(),
                algorithm: String::new(),
                fingerprint: String::new(),
                has_passphrase: true,
            })
        }
    }
}

/// Maps an SSH algorithm identifier to the lowercase name reported by the
/// discovery API, e.g. "ssh-ed25519", "rsa", "ecdsa".
fn algorithm_name(algorithm: Algorithm) -> String {
    let name = algorithm.as_str();
    if name == "ssh-rsa" || name.starts_with("rsa-sha2-") {
        "rsa"
    } else if name.starts_with("ecdsa-") {
        "ecdsa"
    } else {
        name
    }
    .to_string()
}

/// Extracts every `IdentityFile` directive from an ssh_config file, resolved
/// against `ssh_dir` (relative paths are relative to `~/.ssh`, like ssh).
pub(super) fn identity_files_from_config(config: &str, ssh_dir: &Path) -> Vec<PathBuf> {
    config
        .lines()
        .filter_map(parse_identity_file_directive)
        .map(|value| resolve_identity_path(&value, ssh_dir))
        .collect()
}

fn parse_identity_file_directive(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (keyword, value) = if let Some((keyword, value)) = line.split_once('=') {
        (keyword.trim().to_string(), value.trim().to_string())
    } else {
        let mut parts = line.split_whitespace();
        let keyword = parts.next()?.to_string();
        (keyword, parts.collect::<Vec<_>>().join(" "))
    };
    if !keyword.eq_ignore_ascii_case("IdentityFile") {
        return None;
    }
    let value = value.trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_string())
}

fn resolve_identity_path(value: &str, ssh_dir: &Path) -> PathBuf {
    if let Some(remainder) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return ssh_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join(remainder);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        ssh_dir.join(path)
    }
}
