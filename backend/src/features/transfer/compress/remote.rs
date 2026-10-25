/// 远端工具探测命令：退出码 0 且输出含标记串即认为可用。
pub fn probe_command(needs_tar: bool) -> String {
    let base = "command -v gzip >/dev/null 2>&1 && command -v gunzip >/dev/null 2>&1";
    if needs_tar {
        format!("{base} && command -v tar >/dev/null 2>&1 && echo DBX_COMPRESS_OK")
    } else {
        format!("{base} && echo DBX_COMPRESS_OK")
    }
}

/// 下载侧：远端把源文件压成任务临时 .gz（`-c` 写 stdout，源文件原样保留）。
pub fn remote_gzip_command(source: &str, target: &str) -> String {
    format!("gzip -c {} > {}", shell_quote(source), shell_quote(target))
}

/// 上传侧：远端把推送上来的 .gz 临时件还原成 plain 暂存件（随后走既有原子
/// 提交）。不用 `gunzip` 就地改写——保持「暂存件由本任务创建、提交走
/// rename」的既有原子语义。
pub fn remote_gunzip_command(source: &str, target: &str) -> String {
    format!(
        "gunzip -c {} > {}",
        shell_quote(source),
        shell_quote(target)
    )
}

/// 树下载侧：远端整目录打成单流 .tgz。相对打包（`-C <base_dir> .`），
/// 成员名形如 `./relative/path`——解包后与树扫描的相对路径直接对齐，
/// 不依赖 `--strip-components`（busybox tar 兼容面）。
pub fn remote_tar_command(base_dir: &str, target: &str) -> String {
    format!(
        "tar -czf {} -C {} .",
        shell_quote(target),
        shell_quote(base_dir)
    )
}

/// 与 exec.rs 同款单引号转义：直接复用规范实现，本模块不再自带副本
/// （`crate::sessions::ssh::exec::shell_quote`）。
pub(super) use crate::sessions::ssh::exec::shell_quote;
