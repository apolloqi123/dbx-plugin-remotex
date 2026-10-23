//! Extended SFTP operations (stat/touch/direct-write/archive), mirroring tiny-rdm.
#![allow(dead_code)] // entry points are wired in main.rs; see the snippet at the bottom

use crate::filesystem::sftp::name as sftp_name;
use crate::filesystem::sftp::raw as sftp_raw;

mod archive;
mod common;
mod metadata;
mod symlink;
mod unique;
mod watch;
mod write;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use archive::*;
#[allow(unused_imports)]
pub use common::*;
#[allow(unused_imports)]
pub use metadata::*;
#[allow(unused_imports)]
pub use symlink::*;
#[allow(unused_imports)]
pub use unique::*;
#[allow(unused_imports)]
pub use watch::*;
#[allow(unused_imports)]
pub use write::*;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::core::exec::shell_quote;
use crate::core::model::normalize_remote_path;
use crate::filesystem::sftp::name::NameEncoding;
use crate::filesystem::sftp::raw::RawAttrs;
use crate::sessions::ssh::{
    apply_preserved_permissions, format_permissions, lookup_owner_group_names, RawSftpClient,
    SshRuntime,
};

/// Largest payload [`write_file`] accepts in one call; bigger files must go
/// through the streaming upload slot (`sftp/upload/start`).
pub const MAX_DIRECT_WRITE_SIZE: usize = 4 * 1024 * 1024;

/// Remote `tar` commands (archive, listing, extract) run with this budget.
const REMOTE_TAR_TIMEOUT_SECS: u64 = 120;

/// Upper bound for `name(1)..name(999)` collision probing in
/// [`rename_unique`]; also bounds how long the SFTP mutex is held.
pub const UNIQUE_NAME_PROBE_LIMIT: u32 = 999;

/// Longest accepted file name (chars) for the unique-name probe.
const MAX_UNIQUE_NAME_CHARS: usize = 255;

// ---------------------------------------------------------------------------
// latin-1 裸包写路径（M16）：暂存命名 / 分块写 / 原子提交
// ---------------------------------------------------------------------------

/// 裸包 WRITE 单段上限（与 raw 客户端一致；服务器兼容性最稳的切分尺寸）。
const RAW_WRITE_CHUNK: usize = sftp_raw::MAX_WRITE_CHUNK;

// —— 外部编辑器回传上传（watch/upload 后端；remote-edit 目录防逃逸）——
pub const MAX_UPLOAD_LOCAL_SIZE: u64 = 256 * 1024 * 1024;

/// Copy buffer for the streaming local→SFTP push.
const UPLOAD_LOCAL_CHUNK: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// main.rs registration
// ---------------------------------------------------------------------------
// `mod sftp_ext;` is declared next to the other module declarations. The
// match arms below drop into `Plugin::handle_request` (parameters follow the
// existing camelCase convention):
//
//     "sftp/stat" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let path = required_string(&params, "path")?;
//         self.runtime
//             .block_on(sftp_ext::stat(&self.ssh, session_id, path))
//     }
//     "sftp/exists" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let path = required_string(&params, "path")?;
//         let exists = self
//             .runtime
//             .block_on(sftp_ext::exists(&self.ssh, session_id, path))?;
//         Ok(json!({ "exists": exists }))
//     }
//     "sftp/touch" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let path = required_string(&params, "path")?;
//         self.runtime
//             .block_on(sftp_ext::touch(&self.ssh, session_id, path))?;
//         Ok(json!({ "success": true }))
//     }
//     "sftp/write" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let remote_path = required_string(&params, "remotePath")?;
//         let data_base64 = required_string(&params, "dataBase64")?;
//         self.runtime.block_on(sftp_ext::write_file(
//             &self.ssh,
//             session_id,
//             remote_path,
//             data_base64,
//         ))?;
//         Ok(json!({ "success": true }))
//     }
//     "sftp/archive" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let source_paths = params
//             .get("sourcePaths")
//             .and_then(Value::as_array)
//             .ok_or("Missing sourcePaths")?
//             .iter()
//             .map(|value| {
//                 value
//                     .as_str()
//                     .map(str::to_string)
//                     .ok_or_else(|| "sourcePaths must be strings".to_string())
//             })
//             .collect::<Result<Vec<String>, String>>()?;
//         let archive_path = required_string(&params, "archivePath")?;
//         self.runtime.block_on(sftp_ext::archive(
//             &self.ssh,
//             session_id,
//             &source_paths,
//             archive_path,
//         ))
//     }
//     "sftp/extract" => {
//         let session_id = required_string(&params, "sessionId")?;
//         let archive_path = required_string(&params, "archivePath")?;
//         let destination_path = required_string(&params, "destinationPath")?;
//         let overwrite = params
//             .get("overwrite")
//             .and_then(Value::as_bool)
//             .unwrap_or(false);
//         self.runtime.block_on(sftp_ext::extract(
//             &self.ssh,
//             session_id,
//             archive_path,
//             destination_path,
//             overwrite,
//         ))?;
//         Ok(json!({ "success": true }))
//     }
