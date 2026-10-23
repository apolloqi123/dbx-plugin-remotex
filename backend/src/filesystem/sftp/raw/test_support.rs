use super::*;
use std::sync::{Arc, Mutex};

/// 桩服务器收到的请求帧（去掉长度前缀的 payload），供测试断言「发出的
/// 路径/数据字节」——latin-1 往返闭环的字节级证据。
pub type RequestLog = Arc<Mutex<Vec<Vec<u8>>>>;

pub fn request_log() -> RequestLog {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn recorded_requests(log: &RequestLog) -> Vec<Vec<u8>> {
    log.lock().unwrap().clone()
}

/// 内存双工流的桩服务器（先例：本文件测试模块 M14-B 起）：INIT 回
/// VERSION，随后每个请求按脚本回预置包体（自动回填请求帧里的 id），
/// 并把收到的请求 payload 记入 `requests`（None 则不记录）。
///
/// 严格一致性（CI "early eof" 复现线）：每个请求帧先过
/// [`validate_request`] 按 draft-ietf-secsh-filexfer-02 精确校验字段
/// 布局（旧宽松桩对帧内容不设防，正是 MKDIR 缺 ATTRS 这种畸形帧在
/// 本地全绿、真机 OpenSSH 上炸掉的原因）。违规时按
/// [`StrictMode`] 语义处置。
pub async fn scripted_server(
    stream: tokio::io::DuplexStream,
    replies: Vec<Vec<u8>>,
    requests: Option<RequestLog>,
) {
    scripted_server_mode(stream, replies, requests, StrictMode::Panic).await
}

/// OpenSSH 语义的负路径桩：请求布局违规 = sftp-server `fatal` ——进程
/// 退出、通道 EOF（直接断开流，绝不回包）。用于离线复现 CI 的
/// "SFTP raw read failed: early eof"（run 36146663371）。
pub async fn openssh_fatal_server(stream: tokio::io::DuplexStream, replies: Vec<Vec<u8>>) {
    scripted_server_mode(stream, replies, None, StrictMode::Fatal).await
}

/// 桩的违规处置模式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StrictMode {
    /// 测试桩自检：违规即 panic（带 hexdump），让测试用例第一时间炸在桩上。
    Panic,
    /// 模拟 OpenSSH sftp-server：解析失败即 fatal 退出（静默断流）。
    Fatal,
}

async fn scripted_server_mode(
    mut stream: tokio::io::DuplexStream,
    mut replies: Vec<Vec<u8>>,
    requests: Option<RequestLog>,
    mode: StrictMode,
) {
    let mut header = [0_u8; 4];
    loop {
        if stream.read_exact(&mut header).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(header) as usize;
        let mut payload = vec![0_u8; len];
        stream.read_exact(&mut payload).await.unwrap();
        if let Err(error) = validate_request(&payload) {
            match mode {
                StrictMode::Panic => panic!(
                    "strict stub rejected malformed request ({error}); hexdump: {}",
                    hexdump(&payload)
                ),
                StrictMode::Fatal => break, // sftp-server fatal：通道 EOF
            }
        }
        if let Some(log) = requests.as_ref() {
            log.lock().unwrap().push(payload.clone());
        }
        let (kind, body) = if payload[0] == FXP_INIT {
            (FXP_VERSION, 3_u32.to_be_bytes().to_vec())
        } else {
            let id = u32::from_be_bytes(payload[1..5].try_into().unwrap());
            let scripted = if mode == StrictMode::Fatal && replies.is_empty() {
                // fatal 模式下只承诺对合法请求维持连接；无脚本时回 STATUS 0
                // 维持会话（负路径测试只关心畸形帧触发 fatal）。
                status_body(0)
            } else {
                replies.remove(0)
            };
            let mut scripted = scripted;
            scripted[1..5].copy_from_slice(&id.to_be_bytes());
            let kind = scripted.remove(0);
            (kind, scripted)
        };
        let mut packet = vec![kind];
        packet.extend_from_slice(&body);
        let mut frame = (packet.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&packet);
        stream.write_all(&frame).await.unwrap();
        stream.flush().await.unwrap();
    }
}

/// 严格按 draft-ietf-secsh-filexfer-02 校验一个去帧请求 payload 的字段
/// 布局：字段存在、顺序、长度与类型逐一核对，且必须恰好消费完
/// （多发/漏发/截断都拒绝）。客户端所有 [`super::RawSftp`] 操作都应通过。
pub fn validate_request(payload: &[u8]) -> Result<(), String> {
    if payload.is_empty() {
        return Err("empty payload".to_string());
    }
    let kind = payload[0];
    if kind == FXP_INIT {
        if payload.len() != 5 {
            return Err(format!(
                "INIT must be exactly type + u32 version, got {} bytes",
                payload.len()
            ));
        }
        return Ok(());
    }
    if payload.len() < 5 {
        return Err(format!("request type {kind} truncated before id"));
    }
    let mut cursor = 5_usize;
    match kind {
        // 单路径：STAT/LSTAT/OPENDIR/REMOVE/RMDIR/READLINK。
        FXP_STAT | FXP_LSTAT | FXP_OPENDIR | FXP_REMOVE | FXP_RMDIR | FXP_READLINK => {
            expect_string(payload, &mut cursor, "path")?;
        }
        // 单句柄：CLOSE/READDIR。
        FXP_CLOSE | FXP_READDIR => {
            expect_string(payload, &mut cursor, "handle")?;
        }
        FXP_READ => {
            expect_string(payload, &mut cursor, "handle")?;
            expect_u64(payload, &mut cursor, "offset")?;
            expect_u32(payload, &mut cursor, "len")?;
        }
        FXP_WRITE => {
            expect_string(payload, &mut cursor, "handle")?;
            expect_u64(payload, &mut cursor, "offset")?;
            expect_string(payload, &mut cursor, "data")?;
        }
        FXP_OPEN => {
            expect_string(payload, &mut cursor, "path")?;
            expect_u32(payload, &mut cursor, "pflags")?;
            expect_attrs(payload, &mut cursor, "attrs")?;
        }
        // MKDIR 与 SETSTAT 是「路径 + ATTRS」布局：MKDIR 的 attrs 正是
        // CI 现场缺失的字段（OpenSSH process_mkdir fatal 的直接原因）。
        FXP_MKDIR | FXP_SETSTAT => {
            expect_string(payload, &mut cursor, "path")?;
            expect_attrs(payload, &mut cursor, "attrs")?;
        }
        FXP_RENAME | FXP_SYMLINK => {
            expect_string(payload, &mut cursor, "first string")?;
            expect_string(payload, &mut cursor, "second string")?;
        }
        _ => return Err(format!("unexpected request type {kind}")),
    }
    if cursor != payload.len() {
        return Err(format!(
            "type {kind}: consumed {cursor} of {} bytes (missing or trailing fields)",
            payload.len()
        ));
    }
    Ok(())
}

fn expect_string(payload: &[u8], cursor: &mut usize, what: &str) -> Result<(), String> {
    skip_string(payload, cursor).map_err(|_| format!("{what} truncated"))
}

fn expect_u32(payload: &[u8], cursor: &mut usize, what: &str) -> Result<u32, String> {
    read_u32(payload, cursor).map_err(|_| format!("{what} truncated"))
}

fn expect_u64(payload: &[u8], cursor: &mut usize, what: &str) -> Result<u64, String> {
    read_u64(payload, cursor).map_err(|_| format!("{what} truncated"))
}

fn expect_attrs(payload: &[u8], cursor: &mut usize, what: &str) -> Result<(), String> {
    let (_, consumed) =
        parse_attrs(&payload[*cursor..]).map_err(|_| format!("{what} truncated"))?;
    *cursor += consumed;
    Ok(())
}

/// 测试诊断用 hexdump（空格分隔的大写十六进制，超长截断）。
pub fn hexdump(bytes: &[u8]) -> String {
    let limit = bytes.len().min(64);
    let dump = bytes[..limit]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > limit {
        format!("{dump}… (+{} bytes)", bytes.len() - limit)
    } else {
        dump
    }
}

pub fn status_body(code: u32) -> Vec<u8> {
    let mut body = vec![FXP_STATUS];
    body.extend_from_slice(&0_u32.to_be_bytes());
    body.extend_from_slice(&code.to_be_bytes());
    body
}

pub fn attrs_body(size: u64, permissions: u32) -> Vec<u8> {
    attrs_body_full(size, permissions, None, None)
}

/// 带 atime/mtime 的 attrs 回包（stat 映射测试用）。
pub fn attrs_body_full(
    size: u64,
    permissions: u32,
    atime: Option<u32>,
    mtime: Option<u32>,
) -> Vec<u8> {
    let mut body = vec![FXP_ATTRS];
    body.extend_from_slice(&0_u32.to_be_bytes()); // id 占位
    let mut flags = ATTR_SIZE | ATTR_PERMISSIONS;
    if atime.is_some() || mtime.is_some() {
        flags |= ATTR_ACMODTIME;
    }
    body.extend_from_slice(&flags.to_be_bytes());
    body.extend_from_slice(&size.to_be_bytes());
    body.extend_from_slice(&permissions.to_be_bytes());
    if flags & ATTR_ACMODTIME != 0 {
        body.extend_from_slice(&atime.unwrap_or(0).to_be_bytes());
        body.extend_from_slice(&mtime.unwrap_or(0).to_be_bytes());
    }
    body
}

pub fn handle_body(value: &[u8]) -> Vec<u8> {
    let mut body = vec![FXP_HANDLE];
    body.extend_from_slice(&0_u32.to_be_bytes()); // id 占位
    body.extend_from_slice(&(value.len() as u32).to_be_bytes());
    body.extend_from_slice(value);
    body
}

pub fn data_body(data: &[u8]) -> Vec<u8> {
    let mut body = vec![FXP_DATA];
    body.extend_from_slice(&0_u32.to_be_bytes()); // id 占位
    body.extend_from_slice(&(data.len() as u32).to_be_bytes());
    body.extend_from_slice(data);
    body
}

/// NAME 单条目回包（READLINK 用）：name + 空 longname + 空 attrs。
pub fn name_body_one(name: &[u8]) -> Vec<u8> {
    let mut body = vec![FXP_NAME];
    body.extend_from_slice(&0_u32.to_be_bytes()); // id 占位
    body.extend_from_slice(&1_u32.to_be_bytes());
    body.extend_from_slice(&(name.len() as u32).to_be_bytes());
    body.extend_from_slice(name);
    body.extend_from_slice(&0_u32.to_be_bytes()); // longname 空
    body.extend_from_slice(&0_u32.to_be_bytes()); // attrs flags=0
    body
}
