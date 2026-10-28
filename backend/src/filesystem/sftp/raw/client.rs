//! Raw SFTP packet/client layer.

use super::*;

// ---------------------------------------------------------------------------
// 顺序式客户端
// ---------------------------------------------------------------------------

/// 顺序式裸包客户端：持有一条已请求 `sftp` 子系统的通道流。
pub struct RawSftp<S> {
    stream: S,
    next_id: u32,
    /// 一次超时/失步后置位：流里可能还压着迟到的回包帧，后续任何 request
    /// 都会把陈旧帧当成新回包（reply id mismatch 连环炸）。整个 client
    /// 一次性报废、报明确错误，比静默错位安全。
    poisoned: bool,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> RawSftp<S> {
    /// 握手：发送 INIT（版本 3）并校验 VERSION 回包。服务器版本高于 3 拒绝
    /// （v4+ attrs 布局不同，宁可拒绝也不静默解错）。
    pub async fn init(stream: S) -> Result<Self, String> {
        let mut client = Self {
            stream,
            next_id: 1,
            poisoned: false,
        };
        client.send(&build_init()).await?;
        let payload = tokio::time::timeout(REQUEST_TIMEOUT, client.recv())
            .await
            .map_err(|_| "SFTP raw init timed out".to_string())??;
        let (kind, _, body) = parse_response_header(&payload)?;
        if kind != FXP_VERSION {
            return Err("SFTP raw handshake got a non-VERSION reply".to_string());
        }
        if body.len() < 4 {
            return Err("SFTP raw VERSION reply is truncated".to_string());
        }
        let version = u32::from_be_bytes(body[..4].try_into().unwrap());
        if version > 3 {
            return Err(format!(
                "SFTP raw client does not support server version {version}"
            ));
        }
        Ok(client)
    }

    /// 列目录：OPENDIR → READDIR 循环（STATUS EOF 收尾）→ CLOSE。
    /// 文件名以原始字节返回，解码交给调用方（sftp_name）。
    pub async fn readdir(&mut self, path: &[u8]) -> Result<Vec<RawEntry>, String> {
        let handle = self.opendir(path).await?;
        let mut entries = Vec::new();
        loop {
            let id = self.next_id();
            let reply = self.request(build_readdir(id, &handle), id).await?;
            let (kind, _, body) = parse_response_header(&reply)?;
            match kind {
                FXP_NAME => {
                    let batch = parse_name_entries(body)?;
                    let overflow = entries.len() + batch.len() > MAX_READDIR_ENTRIES;
                    entries.extend(batch);
                    if overflow {
                        let _ = self.close(&handle).await;
                        return Err(format!(
                            "SFTP raw readdir exceeded {MAX_READDIR_ENTRIES} entries"
                        ));
                    }
                }
                FXP_STATUS => {
                    let code = parse_status_code(body)?;
                    if code == SSH_FX_EOF {
                        break;
                    }
                    let _ = self.close(&handle).await;
                    return Err(format!("SFTP raw readdir failed with status {code}"));
                }
                _ => {
                    let _ = self.close(&handle).await;
                    return Err("SFTP raw readdir got an unexpected reply".to_string());
                }
            }
        }
        self.close(&handle).await?;
        Ok(entries)
    }

    /// STAT：单个路径的 attrs。
    pub async fn stat(&mut self, path: &[u8]) -> Result<RawAttrs, String> {
        let id = self.next_id();
        let reply = self
            .request(build_string_request(FXP_STAT, id, path), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_ATTRS => parse_attrs_packet(body),
            FXP_STATUS => Err(format!(
                "SFTP raw stat failed with status {}",
                parse_status_code(body)?
            )),
            _ => Err("SFTP raw stat got an unexpected reply".to_string()),
        }
    }

    /// 读一段数据：OPEN(READ) → READ(offset) → CLOSE。短读合法（调用方按
    /// 实际长度推进 offset）；EOF 返回空 Vec。
    pub async fn read_chunk(
        &mut self,
        path: &[u8],
        offset: u64,
        len: u32,
    ) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let handle = self
            .open_handle(path, PFLAGS_READ, &RawAttrs::default())
            .await?;
        let result = async {
            let mut data = Vec::with_capacity(len as usize);
            let mut position = offset;
            // 单个 READ 只请求 READ_CHUNK（32 KiB）：一把梭 1 MiB 在部分
            // 服务端会被截断或拒绝；循环到满片/EOF 与高层 read_filled 同义。
            while (data.len() as u32) < len {
                let requested = (len - data.len() as u32).min(READ_CHUNK as u32);
                let id = self.next_id();
                let reply = self
                    .request(build_read(id, &handle, position, requested), id)
                    .await?;
                let (kind, _, body) = parse_response_header(&reply)?;
                let chunk = match kind {
                    FXP_DATA => parse_data(body)?,
                    FXP_STATUS => {
                        let code = parse_status_code(body)?;
                        if code == SSH_FX_EOF {
                            break;
                        }
                        return Err(format!("SFTP raw read failed with status {code}"));
                    }
                    _ => return Err("SFTP raw read got an unexpected reply".to_string()),
                };
                if chunk.is_empty() {
                    break;
                }
                position = position.saturating_add(chunk.len() as u64);
                data.extend_from_slice(&chunk);
            }
            Ok(data)
        }
        .await;
        self.close(&handle).await?;
        result
    }

    /// 读整个文件（M18，MCP `sftp_read_file` 用）：OPEN(READ) 一次，READ 按
    /// [`READ_CHUNK`] 分块循环（offset 显式推进，STATUS EOF / 空 DATA 收尾），
    /// 最后 CLOSE。至多读 `cap` 字节（调用方用 `max_bytes + 1` 探测截断），
    /// `offset` 支持 TCP 式分页起点；offset 在/超过 EOF 时回空 Vec。
    pub async fn read_file(
        &mut self,
        path: &[u8],
        offset: u64,
        cap: u64,
    ) -> Result<Vec<u8>, String> {
        let handle = self
            .open_handle(path, PFLAGS_READ, &RawAttrs::default())
            .await?;
        let result = async {
            let mut data = Vec::new();
            let mut position = offset;
            while (data.len() as u64) < cap {
                let requested = (cap - data.len() as u64).min(READ_CHUNK as u64) as u32;
                let id = self.next_id();
                let reply = self
                    .request(build_read(id, &handle, position, requested), id)
                    .await?;
                let (kind, _, body) = parse_response_header(&reply)?;
                let chunk = match kind {
                    FXP_DATA => parse_data(body)?,
                    FXP_STATUS => {
                        let code = parse_status_code(body)?;
                        if code == SSH_FX_EOF {
                            break;
                        }
                        return Err(format!("SFTP raw read failed with status {code}"));
                    }
                    _ => return Err("SFTP raw read got an unexpected reply".to_string()),
                };
                if chunk.is_empty() {
                    break;
                }
                position += chunk.len() as u64;
                data.extend_from_slice(&chunk);
            }
            Ok(data)
        }
        .await;
        self.close(&handle).await?;
        result
    }

    /// LSTAT：单个路径的 attrs（不跟随符号链接，DELETE 预检语义与高层
    /// `symlink_metadata` 一致）。
    pub async fn lstat(&mut self, path: &[u8]) -> Result<RawAttrs, String> {
        let id = self.next_id();
        let reply = self
            .request(build_string_request(FXP_LSTAT, id, path), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_ATTRS => parse_attrs_packet(body),
            FXP_STATUS => Err(format!(
                "SFTP raw lstat failed with status {}",
                parse_status_code(body)?
            )),
            _ => Err("SFTP raw lstat got an unexpected reply".to_string()),
        }
    }

    /// REMOVE：删除一个文件（或符号链接）。
    pub async fn remove(&mut self, path: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self
            .request(build_string_request(FXP_REMOVE, id, path), id)
            .await?;
        self.expect_status_ok(&reply, "remove").await
    }

    /// 打开一个可写句柄：OPEN(CREAT|WRITE|TRUNC) → HANDLE。上传族/直写的
    /// 暂存文件与 touch 的新建分支都走这里；数据用 [`Self::write_chunk`]
    /// 写入，最后必须 [`Self::close`]。
    pub async fn open_write(&mut self, path: &[u8]) -> Result<Vec<u8>, String> {
        let pflags = PFLAGS_WRITE | PFLAGS_CREAT | PFLAGS_TRUNC;
        self.open_handle(path, pflags, &RawAttrs::default()).await
    }

    /// 写一段数据到句柄：WRITE(handle, offset, data) → STATUS 0。单段长度
    /// 不得超过 [`MAX_WRITE_CHUNK`]（由调用方切分）。
    pub async fn write_chunk(
        &mut self,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<(), String> {
        if data.len() > MAX_WRITE_CHUNK {
            return Err(format!(
                "SFTP raw write chunk too large: {} > {MAX_WRITE_CHUNK}",
                data.len()
            ));
        }
        let id = self.next_id();
        let reply = self
            .request(build_write(id, handle, offset, data), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_STATUS => {
                let code = parse_status_code(body)?;
                if code == 0 {
                    return Ok(());
                }
                Err(format!("SFTP raw write failed with status {code}"))
            }
            _ => Err("SFTP raw write got an unexpected reply".to_string()),
        }
    }

    /// SETSTAT：按 attrs 子集改写路径属性（touch 的 utime 语义、上传提交的
    /// 权限保留）。服务器拒绝/不支持时返回 Err，由调用方决定是否容忍。
    pub async fn setstat(&mut self, path: &[u8], attrs: &RawAttrs) -> Result<(), String> {
        let id = self.next_id();
        let reply = self.request(build_setstat(id, path, attrs), id).await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_STATUS => {
                let code = parse_status_code(body)?;
                if code == 0 {
                    return Ok(());
                }
                Err(format!("SFTP raw setstat failed with status {code}"))
            }
            _ => Err("SFTP raw setstat got an unexpected reply".to_string()),
        }
    }

    /// READLINK：返回链接指向的原始字节（NAME 包第一条目的文件名字段）。
    pub async fn readlink(&mut self, path: &[u8]) -> Result<Vec<u8>, String> {
        let id = self.next_id();
        let reply = self.request(build_readlink(id, path), id).await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_NAME => {
                let entries = parse_name_entries(body)?;
                entries
                    .into_iter()
                    .next()
                    .map(|entry| entry.name)
                    .ok_or_else(|| "SFTP raw readlink got an empty NAME reply".to_string())
            }
            FXP_STATUS => Err(format!(
                "SFTP raw readlink failed with status {}",
                parse_status_code(body)?
            )),
            _ => Err("SFTP raw readlink got an unexpected reply".to_string()),
        }
    }

    /// SYMLINK：创建 `link_path` → `target`。字符串按 OpenSSH 次序装包
    /// （见 [`build_symlink`]），与高层 `symlink(target, link_path)` 的
    /// 生产行为一致。
    pub async fn symlink(&mut self, target: &[u8], link_path: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self
            .request(build_symlink(id, target, link_path), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_STATUS => {
                let code = parse_status_code(body)?;
                if code == 0 {
                    return Ok(());
                }
                Err(format!("SFTP raw symlink failed with status {code}"))
            }
            _ => Err("SFTP raw symlink got an unexpected reply".to_string()),
        }
    }

    /// MKDIR：创建目录。v3 wire 上 MKDIR 必须携带 ATTRS 字段
    /// （draft-ietf-secsh-filexfer-02 §5.2）；缺该字段时 OpenSSH 的
    /// sftp-server 在 `decode_attrib` 处 fatal 退出（通道 EOF，CI smoke 的
    /// "SFTP raw read failed: early eof" 即此根因）。空 attrs（flags=0）时
    /// OpenSSH 按 0777 & umask 建目录，与高层客户端缺省行为一致。
    pub async fn mkdir(&mut self, path: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self
            .request(build_mkdir(id, path, &RawAttrs::default()), id)
            .await?;
        self.expect_status_ok(&reply, "mkdir").await
    }

    /// RMDIR：删除空目录。
    pub async fn rmdir(&mut self, path: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self
            .request(build_string_request(FXP_RMDIR, id, path), id)
            .await?;
        self.expect_status_ok(&reply, "rmdir").await
    }

    /// RENAME：单个路径重命名/移动（SFTPv3 不覆盖已存在目标）。
    pub async fn rename(&mut self, old_path: &[u8], new_path: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self
            .request(build_rename(id, old_path, new_path), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_STATUS => {
                let code = parse_status_code(body)?;
                if code == 0 {
                    return Ok(());
                }
                Err(format!("SFTP raw rename failed with status {code}"))
            }
            _ => Err("SFTP raw rename got an unexpected reply".to_string()),
        }
    }

    /// 「STATUS 回包 → 0 即成功」的公共骨架（REMOVE/MKDIR/RMDIR）。
    async fn expect_status_ok(&mut self, reply: &[u8], op: &str) -> Result<(), String> {
        let (reply_kind, _, body) = parse_response_header(reply)?;
        match reply_kind {
            FXP_STATUS => {
                let code = parse_status_code(body)?;
                if code == 0 {
                    return Ok(());
                }
                Err(format!("SFTP raw {op} failed with status {code}"))
            }
            _ => Err(format!("SFTP raw {op} got an unexpected reply")),
        }
    }

    async fn opendir(&mut self, path: &[u8]) -> Result<Vec<u8>, String> {
        let id = self.next_id();
        let reply = self
            .request(build_string_request(FXP_OPENDIR, id, path), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_HANDLE => parse_handle(body),
            FXP_STATUS => Err(format!(
                "SFTP raw opendir failed with status {}",
                parse_status_code(body)?
            )),
            _ => Err("SFTP raw opendir got an unexpected reply".to_string()),
        }
    }

    /// CLOSE：释放句柄（open_write/open 的收尾；STATUS 0 或 EOF 均视为成功）。
    pub async fn close(&mut self, handle: &[u8]) -> Result<(), String> {
        let id = self.next_id();
        let reply = self.request(build_close(id, handle), id).await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        if kind == FXP_STATUS {
            let code = parse_status_code(body)?;
            if code == 0 || code == SSH_FX_EOF {
                return Ok(());
            }
            return Err(format!("SFTP raw close failed with status {code}"));
        }
        Err("SFTP raw close got an unexpected reply".to_string())
    }

    /// OPEN 的公共骨架：任意 pflags + attrs → HANDLE。
    async fn open_handle(
        &mut self,
        path: &[u8],
        pflags: u32,
        attrs: &RawAttrs,
    ) -> Result<Vec<u8>, String> {
        let id = self.next_id();
        let reply = self
            .request(build_open(id, path, pflags, attrs), id)
            .await?;
        let (kind, _, body) = parse_response_header(&reply)?;
        match kind {
            FXP_HANDLE => parse_handle(body),
            FXP_STATUS => Err(format!(
                "SFTP raw open failed with status {}",
                parse_status_code(body)?
            )),
            _ => Err("SFTP raw open got an unexpected reply".to_string()),
        }
    }

    /// 发一收一：发送帧后读取响应帧，校验回包 id 匹配。超时/IO 失败/错位
    /// 回包都会置 poisoned——顺序流一旦失步无法自愈，后续请求直接报错。
    async fn request(&mut self, frame: Vec<u8>, id: u32) -> Result<Vec<u8>, String> {
        if self.poisoned {
            return Err("SFTP raw client is out of sync after an earlier failure".to_string());
        }
        self.send(&frame).await?;
        let payload = match tokio::time::timeout(REQUEST_TIMEOUT, self.recv()).await {
            Ok(Ok(payload)) => payload,
            Ok(Err(error)) => {
                self.poisoned = true;
                return Err(error);
            }
            Err(_) => {
                self.poisoned = true;
                return Err("SFTP raw request timed out".to_string());
            }
        };
        let (_, reply_id, _) = match parse_response_header(&payload) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        if reply_id != Some(id) {
            self.poisoned = true;
            return Err("SFTP raw reply id mismatch".to_string());
        }
        Ok(payload)
    }

    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    pub(crate) async fn send(&mut self, frame: &[u8]) -> Result<(), String> {
        self.stream
            .write_all(frame)
            .await
            .map_err(|error| format!("SFTP raw write failed: {error}"))?;
        self.stream
            .flush()
            .await
            .map_err(|error| format!("SFTP raw flush failed: {error}"))
    }

    pub(crate) async fn recv(&mut self) -> Result<Vec<u8>, String> {
        let mut length = [0_u8; 4];
        self.stream
            .read_exact(&mut length)
            .await
            .map_err(|error| format!("SFTP raw read failed: {error}"))?;
        let len = u32::from_be_bytes(length) as usize;
        if len == 0 || len > MAX_PACKET_LEN {
            return Err(format!("SFTP raw packet length out of range: {len}"));
        }
        let mut payload = vec![0_u8; len];
        self.stream
            .read_exact(&mut payload)
            .await
            .map_err(|error| format!("SFTP raw read failed: {error}"))?;
        Ok(payload)
    }
}
