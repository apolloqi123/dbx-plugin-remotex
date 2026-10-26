use serde::{Deserialize, Serialize};

pub(crate) mod connection;
pub(crate) mod name_encoding;
mod paths;
mod terminal;

#[cfg(test)]
mod manifest_contract;
#[cfg(test)]
mod tests;
use serde_json::Value;

pub const TERMINAL_REPLAY_LIMIT: usize = 2 * 1024 * 1024;
pub const TRANSFER_CHUNK_SIZE: usize = 256 * 1024;
pub const MAX_TRANSFER_SIZE: u64 = 16 * 1024 * 1024 * 1024;

pub use connection::*;
pub use name_encoding::NameEncoding;
pub use paths::*;
pub use terminal::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub uri: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Entry owner, names preferred over numeric ids (same semantics as
    /// `sftp/stat`); only populated when the listing requests it. Missing
    /// stays absent so the UI renders "-".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 显示名不可忠实还原（M14-B）：wire 名含 U+FFFD（上游 lossy 解码已把
    /// 非法字节替换掉）。传输始终走 wire/uri；true 时 UI 提示该名字节级
    /// 不可还原，可在设置 → 传输切换文件名编码后重列。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lossy: bool,
}
