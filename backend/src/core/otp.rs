//! OTP 算法中心库（P2-1）：RFC 4226 HOTP / RFC 6238 TOTP 与
//! `otpauth://` URI 的解析/生成。零新增 crate 依赖——HMAC 走既有
//! `hmac`/`sha1`/`sha2`，base32 走既有 `data-encoding`。
//!
//! 取码路径的唯一权威实现：`otp/generate` 协议、连接绑定的 OTP 库条目
//! 自动应答回落（`ssh.rs::sudo_auth_for`）都经由本模块，保证与
//! `triggers.rs` 内联的 tssh 兼容 TOTP（SHA1/6 位/30 s 默认）产出一致。
//! QR 图片解码（`rqrr`/`image` 新依赖）刻意不放在这里——见 `otp_store.rs`。

use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

/// TOTP/HOTP 摘要算法。URI/存储里的大小写不敏感名称见 [`OtpAlgorithm::parse`]，
/// 未知或缺省一律回落 SHA1（RFC 6238 / Google Authenticator 默认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl OtpAlgorithm {
    /// 解析算法名（"SHA1"/"SHA256"/"SHA512"，大小写不敏感；缺省/未知 → SHA1）。
    pub fn parse(value: &str) -> OtpAlgorithm {
        match value.trim().to_ascii_uppercase().as_str() {
            "SHA256" => OtpAlgorithm::Sha256,
            "SHA512" => OtpAlgorithm::Sha512,
            _ => OtpAlgorithm::Sha1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OtpAlgorithm::Sha1 => "SHA1",
            OtpAlgorithm::Sha256 => "SHA256",
            OtpAlgorithm::Sha512 => "SHA512",
        }
    }
}

/// RFC 4226 动态截断：取摘要末字节低 4 位为偏移，读 4 字节大端并屏蔽符号位，
/// 再对 10^digits 取模。`digits` 由调用方约束在 1..=9（协议解析限 6..=8；
/// 9 以上会溢出 u32，这里防御性夹到 9）。
pub fn hotp(algorithm: OtpAlgorithm, secret: &[u8], counter: u64, digits: u8) -> u32 {
    let digits = digits.clamp(1, 9);
    let message = counter.to_be_bytes();
    let digest: Vec<u8> = match algorithm {
        OtpAlgorithm::Sha1 => {
            let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        OtpAlgorithm::Sha256 => {
            let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        OtpAlgorithm::Sha512 => {
            let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(secret)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
    };
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | digest[offset + 3] as u32;
    binary % 10_u32.pow(u32::from(digits))
}

/// 指定墙钟时刻的 TOTP：返回 `(码, 剩余秒数)`。码左补零到 `digits` 位；
/// 剩余秒数 = `period - unix_secs % period`（RFC 6238 步长语义，
/// counter = unix_secs / period）。`period` 为 0 时按 1 处理（防御；解析层
/// 已拒绝非正周期）。
pub fn totp_at(
    algorithm: OtpAlgorithm,
    secret: &[u8],
    period: u64,
    unix_secs: u64,
    digits: u8,
) -> (String, u64) {
    let period = period.max(1);
    let counter = unix_secs / period;
    let code = hotp(algorithm, secret, counter, digits);
    (
        format!("{code:0width$}", width = digits as usize),
        period - unix_secs % period,
    )
}

/// `otpauth://` URI 解析产物（协议回传给前端预填，不入库）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpUriParams {
    /// "totp" | "hotp"
    pub otp_type: String,
    /// 发行方（`issuer` 参数优先，其次 label 的 `Issuer:account` 前缀）。
    pub issuer: String,
    /// 解码后的 label（URI 路径段，通常是 `Issuer:account` 或账号）。
    pub label: String,
    /// 规范化（去空格/连字符、大写、补齐填充后重编码）的 base32 密钥。
    pub secret_base32: String,
    pub algorithm: OtpAlgorithm,
    pub digits: u8,
    pub period: u64,
    /// 仅 hotp 有值；totp 忽略该参数。
    pub counter: Option<u64>,
}

/// 解析 `otpauth://totp/…` / `otpauth://hotp/…` URI。要求精确前缀；
/// `secret` 必填且必须是合法 base32（容忍空格/连字符与大小写）；
/// `algorithm` 未知值回落 SHA1；`digits`（默认 6）限 6..=8；
/// `period`（默认 30）限 1..=3600；hotp 缺 `counter` 报错。
pub fn parse_otpauth_uri(uri: &str) -> Result<OtpUriParams, String> {
    let uri = uri.trim();
    let (otp_type, rest) = if let Some(rest) = uri.strip_prefix("otpauth://totp/") {
        ("totp", rest)
    } else if let Some(rest) = uri.strip_prefix("otpauth://hotp/") {
        ("hotp", rest)
    } else {
        return Err("expected an otpauth://totp/ or otpauth://hotp/ URI".to_string());
    };
    let (raw_label, query) = rest.split_once('?').unwrap_or((rest, ""));

    let mut secret_base32 = String::new();
    let mut issuer = String::new();
    let mut digits: u8 = 6;
    let mut period: u64 = 30;
    let mut counter: Option<u64> = None;
    let mut algorithm = OtpAlgorithm::Sha1;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            "secret" => secret_base32 = normalize_base32(&value)?,
            "issuer" => issuer = value,
            // 未知算法值与既有 exec::parse_otpauth 一致回落 SHA1。
            "algorithm" => algorithm = OtpAlgorithm::parse(&value),
            "digits" => {
                digits = value
                    .parse::<u8>()
                    .ok()
                    .filter(|digits| (6..=8).contains(digits))
                    .ok_or_else(|| format!("otpauth digits must be 6-8, got '{value}'"))?;
            }
            "period" => {
                period = value
                    .parse::<u64>()
                    .ok()
                    .filter(|period| (1..=3600).contains(period))
                    .ok_or_else(|| format!("otpauth period must be 1-3600, got '{value}'"))?;
            }
            "counter" => {
                if !value.is_empty() && value.parse::<u64>().is_err() {
                    return Err(format!("otpauth counter must be an integer, got '{value}'"));
                }
                // totp 忽略 counter；hotp 缺失在下方统一报错。
                counter = (otp_type == "hotp").then(|| value.parse().ok()).flatten();
            }
            _ => {}
        }
    }

    if secret_base32.is_empty() {
        return Err("otpauth URI is missing the secret parameter".to_string());
    }
    let label = percent_decode(raw_label);
    let issuer = if issuer.is_empty() {
        // Google Authenticator 约定：label = "Issuer:account"。
        label
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        issuer
    };
    if otp_type == "hotp" && counter.is_none() {
        return Err("otpauth hotp URI requires a counter parameter".to_string());
    }
    Ok(OtpUriParams {
        otp_type: otp_type.to_string(),
        issuer,
        label,
        secret_base32,
        algorithm,
        digits,
        period,
        counter,
    })
}

/// 由解析产物重建 `otpauth://` URI（`otp/import-qr` 预填回显与解析/生成
/// 往返测试共用）。label 做百分号编码，参数按 Google Authenticator 约定排列。
/// W2b 的 OTP 面板（展示二维码/导出）接线前暂无调用方；保留完整 URI 能力。
#[allow(dead_code)]
pub fn to_otpauth_uri(params: &OtpUriParams) -> String {
    let mut uri = format!(
        "otpauth://{}/{}?secret={}&issuer={}&algorithm={}&digits={}&period={}",
        params.otp_type,
        percent_encode(&params.label),
        params.secret_base32,
        percent_encode(&params.issuer),
        params.algorithm.as_str(),
        params.digits,
        params.period,
    );
    if params.otp_type == "hotp" {
        uri.push_str(&format!(
            "&counter={}",
            params
                .counter
                .map(|value| value.to_string())
                .unwrap_or_default()
        ));
    }
    uri
}

/// 容错 base32 解码（对齐 authenticator 与 triggers.rs 接受的形态）：
/// 丢弃空格/连字符、大小写折叠、补齐缺失的 `=` 填充。返回原始 HMAC 密钥字节。
pub fn decode_base32(text: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = text
        .chars()
        .filter(|character| !matches!(character, ' ' | '-'))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    let unpadded = cleaned.trim_end_matches('=');
    if unpadded.is_empty() {
        return Err("base32 secret is empty".to_string());
    }
    let mut padded = unpadded.to_string();
    if !unpadded.len().is_multiple_of(8) {
        padded.push_str(&"=".repeat(8 - (unpadded.len() % 8)));
    }
    data_encoding::BASE32
        .decode(padded.as_bytes())
        // 绝不回显 `text`：这是 TOTP 长期密钥（等同第二因素私钥），错误串会
        // 冒泡到 RPC error → 前端横幅与审计日志。只报期望字符集与长度特征。
        .map_err(|_| {
            format!(
                "invalid base32 secret (expected A-Z2-7, got {} characters)",
                unpadded.chars().count()
            )
        })
}

/// 规范化 base32 密钥文本：合法性与 [`decode_base32`] 一致，返回大写无填充
/// 规范形态（存储与回显统一用这个）。
fn normalize_base32(text: &str) -> Result<String, String> {
    let decoded = decode_base32(text)?;
    Ok(data_encoding::BASE32_NOPAD.encode(&decoded))
}

/// RFC 3986 保留集之外原样保留；等价于 `encodeURIComponent` 的常用子集
/// （otpauth label/issuer 只需要这一档，避免为编码再引 crate）。
#[allow(dead_code)]
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// `%XX` 百分号解码；非法序列原样保留（宽松，避免给粘贴数据报无谓错误）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let Some(hex) = bytes.get(index + 1..index + 3) {
                if let Ok(value) = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16) {
                    decoded.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests;
