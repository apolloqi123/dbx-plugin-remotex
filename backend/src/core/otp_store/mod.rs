//! OTP 库条目存储（P2-1）：`<plugin_data_dir>/otp-entries.json`（0600，损坏
//! 视为空——同 quick-commands.json / quick-sudo-profiles.json 的防腐政策）。
//!
//! 条目密钥经 vault 字段级 AEAD 加密落盘（复用 Quick Sudo 的
//! `totpSecret` 字段绑定，AAD = `totpSecret|<entryId>`，只调用 vault 不改它）。
//! `bindings` 段维护 `connection_id -> entry_id` 的连接绑定，供自动应答回落
//! 与工作台展示使用。列表视图永远脱敏（只回 `hasSecret`），明文密钥只经
//! [`get_decrypted_secret`] 短暂存在。
//!
//! 另含两件与"OTP 取码"同生命周期的小设施：
//! - 共享防重放缓存：`otp/generate` 与连接绑定条目的自动应答回落共用
//!   （同 time_step 已发出的 TOTP 码不重复返回，等下一周期）；HOTP 不缓存。
//! - `otp/import-qr` 的 QR 图片解码（`rqrr` + `image`，本任务唯一新增依赖；
//!   放在存储层而非算法库 `otp.rs`，后者保持零新增依赖）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::core::otp::{self, OtpAlgorithm, OtpUriParams};
use crate::core::vault::{self, Vault, FIELD_TOTP_SECRET};

mod qr;
mod store;
mod totp;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use qr::*;
#[allow(unused_imports)]
pub use store::*;
#[allow(unused_imports)]
pub use totp::*;
