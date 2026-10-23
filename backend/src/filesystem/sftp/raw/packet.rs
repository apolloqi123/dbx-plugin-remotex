//! Raw SFTP packet/client layer.

use super::*;

/// v3 attrs 的传输子集（列表/下载/写侧 SETSTAT 所需）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawAttrs {
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub atime: Option<u32>,
    pub mtime: Option<u32>,
}

/// READDIR 单条目：文件名原始字节 + attrs。
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: Vec<u8>,
    pub attrs: RawAttrs,
}

// ---------------------------------------------------------------------------
// 纯编解码（单测覆盖；async 客户端只在真机上验证）
// ---------------------------------------------------------------------------

/// 装帧一个包：`u32 BE 长度 + payload`。
pub fn frame_packet(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// 构造 INIT 请求（显式版本 3 + 空扩展表）。
pub fn build_init() -> Vec<u8> {
    let mut payload = vec![FXP_INIT];
    payload.extend_from_slice(&3_u32.to_be_bytes());
    frame_packet(&payload)
}

/// 构造带 id 的简单请求：`type + id + 字符串参数`（OPEN/OPENDIR/STAT 用）。
pub fn build_string_request(kind: u8, id: u32, path: &[u8]) -> Vec<u8> {
    let mut payload = vec![kind];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
    payload.extend_from_slice(path);
    frame_packet(&payload)
}

/// 构造 READDIR 请求。
pub fn build_readdir(id: u32, handle: &[u8]) -> Vec<u8> {
    build_string_request(FXP_READDIR, id, handle)
}

/// 构造 MKDIR 请求：`type + id + 路径字符串 + ATTRS`。draft-ietf-secsh-filexfer-02
/// §5.2 规定 MKDIR 的最后一个字段是 ATTRS——不可省略（OpenSSH sftp-server
/// 的 `process_mkdir` 对缺失 attrs 走 `fatal`，进程退出即通道 EOF）。
pub fn build_mkdir(id: u32, path: &[u8], attrs: &RawAttrs) -> Vec<u8> {
    let mut payload = vec![FXP_MKDIR];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
    payload.extend_from_slice(path);
    payload.extend_from_slice(&encode_attrs(attrs));
    frame_packet(&payload)
}

/// 构造 CLOSE 请求。
pub fn build_close(id: u32, handle: &[u8]) -> Vec<u8> {
    build_string_request(FXP_CLOSE, id, handle)
}

/// 构造 READ 请求（v3 无 seek：offset 显式携带）。
pub fn build_read(id: u32, handle: &[u8], offset: u64, len: u32) -> Vec<u8> {
    let mut payload = vec![FXP_READ];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(handle.len() as u32).to_be_bytes());
    payload.extend_from_slice(handle);
    payload.extend_from_slice(&offset.to_be_bytes());
    payload.extend_from_slice(&len.to_be_bytes());
    frame_packet(&payload)
}

/// 构造 RENAME 请求（v3：oldpath + newpath 两个字符串，无 posix-rename 语义）。
pub fn build_rename(id: u32, old_path: &[u8], new_path: &[u8]) -> Vec<u8> {
    let mut payload = vec![FXP_RENAME];
    payload.extend_from_slice(&id.to_be_bytes());
    for path in [old_path, new_path] {
        payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
        payload.extend_from_slice(path);
    }
    frame_packet(&payload)
}

/// 构造 OPEN 请求（写侧用：pflags + attrs 全量字段，attrs 为空即 flags=0）。
pub fn build_open(id: u32, path: &[u8], pflags: u32, attrs: &RawAttrs) -> Vec<u8> {
    let mut payload = vec![FXP_OPEN];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
    payload.extend_from_slice(path);
    payload.extend_from_slice(&pflags.to_be_bytes());
    payload.extend_from_slice(&encode_attrs(attrs));
    frame_packet(&payload)
}

/// 构造 WRITE 请求：`handle + offset + data`（data 长度上限见
/// [`MAX_WRITE_CHUNK`]，由调用方切分）。
pub fn build_write(id: u32, handle: &[u8], offset: u64, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![FXP_WRITE];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(handle.len() as u32).to_be_bytes());
    payload.extend_from_slice(handle);
    payload.extend_from_slice(&offset.to_be_bytes());
    payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
    payload.extend_from_slice(data);
    frame_packet(&payload)
}

/// 构造 SETSTAT 请求：路径 + attrs（touch 的时间刷新、上传暂存的权限保留）。
pub fn build_setstat(id: u32, path: &[u8], attrs: &RawAttrs) -> Vec<u8> {
    let mut payload = vec![FXP_SETSTAT];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
    payload.extend_from_slice(path);
    payload.extend_from_slice(&encode_attrs(attrs));
    frame_packet(&payload)
}

/// 构造 READLINK 请求。
pub fn build_readlink(id: u32, path: &[u8]) -> Vec<u8> {
    build_string_request(FXP_READLINK, id, path)
}

/// 构造 SYMLINK 请求。v3 wire 上两个字符串的次序在 draft 与 OpenSSH 之间
/// 历史性颠倒：OpenSSH 服务器按 `target, linkpath` 读取（russh-sftp 高层
/// 靠调用方交换参数对齐，见 `sftp_ext::symlink_create` 的考证注释）。裸包
/// 直接按 OpenSSH 次序装包：第一个字符串是链接指向，第二个是链接路径。
pub fn build_symlink(id: u32, target: &[u8], link_path: &[u8]) -> Vec<u8> {
    let mut payload = vec![FXP_SYMLINK];
    payload.extend_from_slice(&id.to_be_bytes());
    for path in [target, link_path] {
        payload.extend_from_slice(&(path.len() as u32).to_be_bytes());
        payload.extend_from_slice(path);
    }
    frame_packet(&payload)
}

/// v3 attrs 解析；返回 (attrs, 消费字节数)。EXTENDED 扩展按长度跳过。
pub fn parse_attrs(buf: &[u8]) -> Result<(RawAttrs, usize), String> {
    let mut cursor = 0;
    let flags = read_u32(buf, &mut cursor)?;
    let mut attrs = RawAttrs::default();
    if flags & ATTR_SIZE != 0 {
        attrs.size = Some(read_u64(buf, &mut cursor)?);
    }
    if flags & ATTR_UIDGID != 0 {
        // uid/gid 对列表与下载无用，跳过。
        read_u32(buf, &mut cursor)?;
        read_u32(buf, &mut cursor)?;
    }
    if flags & ATTR_PERMISSIONS != 0 {
        attrs.permissions = Some(read_u32(buf, &mut cursor)?);
    }
    if flags & ATTR_ACMODTIME != 0 {
        attrs.atime = Some(read_u32(buf, &mut cursor)?);
        attrs.mtime = Some(read_u32(buf, &mut cursor)?);
    }
    if flags & ATTR_EXTENDED != 0 {
        let count = read_u32(buf, &mut cursor)?;
        for _ in 0..count {
            skip_string(buf, &mut cursor)?;
            skip_string(buf, &mut cursor)?;
        }
    }
    Ok((attrs, cursor))
}

/// v3 attrs 编码（[`parse_attrs`] 的逆）：只编码 Some 的字段；时间字段按
/// v3 固定为 atime+mtime 成对出现（缺省一侧按 0 补位）。
pub fn encode_attrs(attrs: &RawAttrs) -> Vec<u8> {
    let mut flags = 0_u32;
    if attrs.size.is_some() {
        flags |= ATTR_SIZE;
    }
    if attrs.permissions.is_some() {
        flags |= ATTR_PERMISSIONS;
    }
    if attrs.atime.is_some() || attrs.mtime.is_some() {
        flags |= ATTR_ACMODTIME;
    }
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&flags.to_be_bytes());
    if let Some(size) = attrs.size {
        out.extend_from_slice(&size.to_be_bytes());
    }
    if let Some(permissions) = attrs.permissions {
        out.extend_from_slice(&permissions.to_be_bytes());
    }
    if flags & ATTR_ACMODTIME != 0 {
        out.extend_from_slice(&attrs.atime.unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&attrs.mtime.unwrap_or(0).to_be_bytes());
    }
    out
}

/// 解包一个已去帧的响应：返回 (type, id_or_none, body)。
pub fn parse_response_header(payload: &[u8]) -> Result<(u8, Option<u32>, &[u8]), String> {
    if payload.is_empty() {
        return Err("SFTP raw response is empty".to_string());
    }
    let kind = payload[0];
    if kind == FXP_VERSION {
        return Ok((kind, None, &payload[1..]));
    }
    if payload.len() < 5 {
        return Err("SFTP raw response is truncated".to_string());
    }
    let id = u32::from_be_bytes(payload[1..5].try_into().unwrap());
    Ok((kind, Some(id), &payload[5..]))
}

/// 解析 NAME 包体：`count + 条目(name bytes, longname bytes, attrs)`。
pub fn parse_name_entries(body: &[u8]) -> Result<Vec<RawEntry>, String> {
    let mut cursor = 0;
    let count = read_u32(body, &mut cursor)?;
    if count as usize > MAX_READDIR_ENTRIES {
        return Err(format!("SFTP raw readdir batch too large: {count}"));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = read_string_bytes(body, &mut cursor)?;
        skip_string(body, &mut cursor)?; // longname
        let (attrs, consumed) = parse_attrs(&body[cursor..])?;
        cursor += consumed;
        entries.push(RawEntry { name, attrs });
    }
    Ok(entries)
}

/// 解析 STATUS 包体：`code (+ lang/tag 字符串，容错缺省)`。
pub fn parse_status_code(body: &[u8]) -> Result<u32, String> {
    let mut cursor = 0;
    read_u32(body, &mut cursor)
}

/// 解析 HANDLE 包体：handle 原始字节。
pub fn parse_handle(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut cursor = 0;
    read_string_bytes(body, &mut cursor)
}

/// 解析 DATA 包体：数据字节。
pub fn parse_data(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut cursor = 0;
    read_string_bytes(body, &mut cursor)
}

/// 解析 ATTRS 包体。
pub fn parse_attrs_packet(body: &[u8]) -> Result<RawAttrs, String> {
    let (attrs, _) = parse_attrs(body)?;
    Ok(attrs)
}

/// 从裸包客户端的错误串里提取 STATUS 码（错误串形如
/// `"SFTP raw lstat failed with status 2"`）：调用方据此区分
/// [`SSH_FX_NO_SUCH_FILE`]（存在性语义）与其他失败。非错误串/无码返回 None。
pub fn error_status(error: &str) -> Option<u32> {
    error
        .rsplit_once("status ")
        .and_then(|(_, tail)| tail.trim().parse().ok())
}

pub(crate) fn read_u32(buf: &[u8], cursor: &mut usize) -> Result<u32, String> {
    if buf.len() < *cursor + 4 {
        return Err("SFTP raw packet is truncated".to_string());
    }
    let value = u32::from_be_bytes(buf[*cursor..*cursor + 4].try_into().unwrap());
    *cursor += 4;
    Ok(value)
}

pub(crate) fn read_u64(buf: &[u8], cursor: &mut usize) -> Result<u64, String> {
    if buf.len() < *cursor + 8 {
        return Err("SFTP raw packet is truncated".to_string());
    }
    let value = u64::from_be_bytes(buf[*cursor..*cursor + 8].try_into().unwrap());
    *cursor += 8;
    Ok(value)
}

pub(crate) fn read_string_bytes(buf: &[u8], cursor: &mut usize) -> Result<Vec<u8>, String> {
    let len = read_u32(buf, cursor)? as usize;
    if buf.len() < *cursor + len {
        return Err("SFTP raw packet is truncated".to_string());
    }
    let value = buf[*cursor..*cursor + len].to_vec();
    *cursor += len;
    Ok(value)
}

pub(crate) fn skip_string(buf: &[u8], cursor: &mut usize) -> Result<(), String> {
    read_string_bytes(buf, cursor).map(|_| ())
}
