use super::*;

/// spawn_blocking 用的进度回调：入参是「已处理的源/压缩流字节数」。
pub type ProgressSink = Arc<dyn Fn(u64) + Send + Sync>;

/// 上传压缩预压的运行时吞吐守卫参数（一次成型的一次性检查点）：处理满
/// `min_bytes` 且耗时满 `min_elapsed` 时测一次速率，低于 `floor_mib_s` 即
/// 以 [`RATE_GUARD_ERROR`] 中止（调用方回退普通推送）。文件小于检查点字节
/// 数时守卫自然不触发——小文件压缩的绝对开销可忽略。
#[derive(Clone, Copy, Debug)]
pub struct RateGuard {
    pub min_bytes: u64,
    pub min_elapsed: std::time::Duration,
    pub floor_mib_s: u64,
}

impl RateGuard {
    /// 冒烟/常规口径的默认守卫（8 MiB / 1s / 20 MiB/s）。
    pub fn default_floor() -> Self {
        RateGuard {
            min_bytes: RATE_GUARD_MIN_BYTES,
            min_elapsed: RATE_GUARD_MIN_ELAPSED,
            floor_mib_s: RATE_GUARD_FLOOR_MIB_S,
        }
    }
}

/// 把本地文件流式 gzip 压缩到目标路径。同步阻塞实现——调用方包在
/// `spawn_blocking` 里。`progress` 收「已消费的源字节数」；`cancelled` 置位
/// 时以 `Interrupted` 中止并删除半成品目标；`rate_guard` 命中时以
/// `Other(RATE_GUARD_ERROR)` 中止（同样清理半成品）。返回压缩后体积。
pub fn compress_file_to(
    source: &Path,
    target: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(u64),
    rate_guard: Option<&RateGuard>,
) -> std::io::Result<u64> {
    let started = std::time::Instant::now();
    let mut reader = std::fs::File::open(source)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let writer = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)?;
    let mut encoder = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
    let mut buffer = vec![0_u8; STREAM_BUFFER_SIZE];
    let mut consumed = 0_u64;
    let abort_reason: Option<&'static str> = loop {
        if cancelled.load(Ordering::Acquire) {
            break Some("transfer cancelled");
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break None;
        }
        encoder
            .write_all(&buffer[..read])
            .map_err(|error| std::io::Error::other(format!("gzip compress failed: {error}")))?;
        consumed += read as u64;
        if let Some(guard) = rate_guard {
            let elapsed = started.elapsed();
            if consumed >= guard.min_bytes && elapsed >= guard.min_elapsed {
                // MiB/s（u128 防溢出）；速率不足即「CPU 不行」，一次性检查
                // 后不再重复判（后续只会更快或更慢，早停早回退）。
                let mib_s = u128::from(consumed) * 1_000_000
                    / (elapsed.as_micros().max(1) * 1_048_576).max(1);
                if mib_s < u128::from(guard.floor_mib_s) {
                    break Some(RATE_GUARD_ERROR);
                }
            }
        }
        progress(consumed);
    };
    if let Some(reason) = abort_reason {
        // finish() 会尝试把缓冲尾写完，失败无所谓——半成品整个删掉。
        let _ = encoder.finish();
        let _ = std::fs::remove_file(target);
        return Err(if reason == RATE_GUARD_ERROR {
            std::io::Error::other(RATE_GUARD_ERROR)
        } else {
            std::io::Error::new(std::io::ErrorKind::Interrupted, reason)
        });
    }
    encoder
        .finish()
        .map_err(|error| std::io::Error::other(format!("gzip finish failed: {error}")))?;
    Ok(std::fs::metadata(target)?.len())
}

/// 把 gzip 文件流式解压到目标路径（MultiGzDecoder 兼容多成员 .gz，与
/// `gzip` 命令输出兼容）。返回解压出的总字节数。
pub fn decompress_file_to(
    source: &Path,
    target: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(u64),
) -> std::io::Result<u64> {
    let reader = std::io::BufReader::new(std::fs::File::open(source)?);
    let mut decoder = flate2::read::MultiGzDecoder::new(reader);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)?;
    let mut buffer = vec![0_u8; STREAM_BUFFER_SIZE];
    let mut written = 0_u64;
    let aborted = loop {
        if cancelled.load(Ordering::Acquire) {
            break true;
        }
        let read = decoder.read(&mut buffer)?;
        if read == 0 {
            break false;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| std::io::Error::other(format!("gunzip write failed: {error}")))?;
        written += read as u64;
        progress(written);
    };
    if aborted {
        let _ = std::fs::remove_file(target);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "transfer cancelled",
        ));
    }
    writer.flush()?;
    Ok(written)
}

/// 把 tar.gz 归档解包到目标根目录（树下载压缩通道的本地端）。仅普通文件
/// 与目录——符号/硬链接与特殊文件跳过，与普通树管线「不跟随 symlink」的
/// 语义对齐；tar crate 的 `unpack_in` 自带路径穿越防护（`..`/绝对路径条
/// 目报错）。`progress` 收已解包的普通文件累计字节。返回累计字节。
pub fn extract_tar_gz(
    archive: &Path,
    root: &Path,
    cancelled: &AtomicBool,
    progress: &dyn Fn(u64),
) -> std::io::Result<u64> {
    std::fs::create_dir_all(root)?;
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    let mut written = 0_u64;
    for entry in archive.entries()? {
        if cancelled.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "transfer cancelled",
            ));
        }
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        if !matches!(
            entry_type,
            tar::EntryType::Regular | tar::EntryType::Directory
        ) {
            continue;
        }
        let size = entry.header().size().unwrap_or(0);
        // unpack_in 对穿越/绝对路径条目静默跳过（Ok(false)）——只有真实
        // 落盘的条目才计入进度与字节。
        if entry.unpack_in(root)? && entry_type == tar::EntryType::Regular {
            written += size;
            progress(written);
        }
    }
    Ok(written)
}
