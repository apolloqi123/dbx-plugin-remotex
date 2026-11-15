//! Expect-style terminal trigger engine (tssh「自动交互」对标，0.4.74)。
//!
//! 用户在连接上配置有序阶段规则（对齐 tssh `ExpectPattern1..N`）：PTY 输出
//! 经 `normalize_auth_prompt_text` 归一化后进入 ≤8 KiB 滚动缓冲，按序匹配
//! 正则，命中后生成应答发送计划（`TriggerDecision.segments`），由终端读
//! 循环负责分段 `tokio::sleep` 写回 channel。每阶段应答五选一：
//! - 明文 `sendText`（`\r`/`\n`/`\t` 转义、`\|` 分段停顿 `sleepMs`）；
//! - 密文引用 `sendSecretKey`（解析宿主 secret binding 槽位，自动补 `\r`）；
//! - 字面密文 `sendSecret`（tssh `ExpectSendPassN` 解密产物，自动补 `\r`）；
//! - TOTP `sendTotp`（RFC 6238 SHA1/6 位/30 s，命中时按当前时间生成，
//!   对齐 tssh `ExpectSendTotpN` / `ExpectSendEncTotpN`，自动补 `\r`）；
//! - 本地命令 `sendCommand`（shell 执行取 stdout，自动补 `\r`；
//!   对齐 tssh `ExpectSendOtpN` / `ExpectSendEncOtpN`）。
//!
//! tssh 文本形态完全兼容（0.4.77 起）：`ExpectSendPassN` /
//! `ExpectCaseSendPassN` / `ExpectSendEncTotpN` / `ExpectSendEncOtpN` 的
//! `--enc-secret` 密文按 tssh `decodeSecret` 同款算法解密（AES-256-GCM、
//! 固定内嵌密钥，见 [`decode_tssh_enc_secret`]）——与 tssh 同为混淆而非加密。
//!
//! 复位语义（契约 D3）：行尾 `$`/`#` 的 shell 提示（复用 `exec::has_shell_prompt`）
//! 即阶段游标归零；阶段超时（秒级，`now` 由调用方注入便于测试）同样归零并
//! 产出 `timeout` 决策，会话继续不断连。超时只在**序列中途**生效（已命中
//! 前序阶段、在等第 2..N 阶段）：空闲等待第一阶段是事件驱动的无限期等待，
//! 不设超时，否则闲置会话会周期性刷 timeout。case 预匹配（`casePattern`）
//! 命中不推进游标，答完继续等本阶段 pattern。
//!
//! 安全红线：应答内容只存在于发送计划里，绝不进日志、事件或错误信息；
//! `TriggersConfig` 的 `Debug` 实现对密文应答（`Secret`/`Totp`）脱敏。

use std::future::Future;
use std::pin::Pin;
use std::process::Output;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::sessions::ssh::exec::{has_shell_prompt, normalize_auth_prompt_text};

mod command;
mod engine;
mod send;
pub use crate::core::model::connection::triggers::*;

#[cfg(test)]
use crate::core::model::connection::triggers::TSSH_ENC_SECRET_KEY;
#[cfg(test)]
use aes_gcm::{aead::KeyInit, Aes256Gcm, Nonce};

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use command::*;
#[allow(unused_imports)]
pub use engine::*;
#[allow(unused_imports)]
pub use send::*;

/// A resolved answer's send plan: decision kind, the `(payload, delay_ms)`
/// segments, and the unresolved command line for `sendCommand` answers.
type AnswerPlan = (TriggerKind, Vec<(Vec<u8>, u64)>, Option<String>);
