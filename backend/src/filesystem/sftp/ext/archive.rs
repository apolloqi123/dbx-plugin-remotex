//! Extended SFTP operation layer.

use super::*;

/// `sftp/archive` — packs multiple remote paths into `archive_path` with a
/// remote `tar -czf`. The archive is built at `<archive_path>.tmp` and renamed
/// into place only after tar succeeds. Returns `{path, size}`.
pub async fn archive(
    runtime: &SshRuntime,
    session_id: &str,
    source_paths: &[String],
    archive_path: &str,
) -> Result<Value, String> {
    runtime.ensure_writable(session_id).await?;
    let sources = clean_source_paths(source_paths)?;
    let target = normalize_remote_path(archive_path)?;
    let parent = common_parent_dir(&sources);
    let relatives = sources
        .iter()
        .map(|source| relative_to_parent(source, &parent))
        .collect::<Vec<_>>();
    // 暂存名掺 uuid：确定性 `{target}.tmp` 在两个并发归档到同一目标时共享
    // 同一文件——两个 tar 互相截断产出损坏归档，且一方的失败清理会删掉
    // 另一方正在写的 tmp。uuid 隔离后清理只可能删到自己的那份。
    let temporary = format!("{}.{}.tmp", target, Uuid::new_v4());
    let command = build_archive_command(&temporary, &parent, &relatives);
    let outcome = runtime
        .exec(
            session_id,
            None,
            &command,
            false,
            Some(REMOTE_TAR_TIMEOUT_SECS),
        )
        .await?;
    let sftp = runtime.sftp(session_id).await?;
    if let Err(error) = check_exec_success(&outcome, "archive") {
        let _ = sftp.lock().await.remove_file(temporary).await;
        return Err(error);
    }
    // tar -czf would have replaced an existing archive; mirror that by
    // dropping a stale target before the rename (plain SFTP rename does not
    // overwrite).
    let session = sftp.lock().await;
    if session.metadata(target.clone()).await.is_ok() {
        session.remove_file(target.clone()).await.map_err(|error| {
            format!("SFTP archive target already exists and could not be replaced: {error}")
        })?;
    }
    if let Err(error) = session.rename(temporary.clone(), target.clone()).await {
        let _ = session.remove_file(temporary).await;
        return Err(sftp_error(error));
    }
    let size = session
        .metadata(target.clone())
        .await
        .ok()
        .and_then(|metadata| metadata.size)
        .unwrap_or(0);
    Ok(json!({ "path": target, "size": size }))
}

/// `sftp/extract` — unpacks a remote `.tar.gz`/`.tgz`/`.tar` archive into
/// `destination_path` after `mkdir -p`. With `overwrite` disabled the member
/// listing is compared against the destination first and any name collision
/// aborts before anything is written. `.zip` is unsupported.
///
/// `sudo`（文件夹整包上传车道配套）：列档与解包都经 sudo 编排（root 读归档、
/// 写登录用户不可写的目标目录）；撞名预检跳过——整包车道的语义就是解包覆盖，
/// 逐文件的 rename/ask 策略作用于归档文件名本身。
pub async fn extract(
    runtime: &SshRuntime,
    session_id: &str,
    archive_path: &str,
    destination_path: &str,
    overwrite: bool,
    sudo: bool,
) -> Result<(), String> {
    runtime.ensure_writable(session_id).await?;
    let archive = normalize_remote_path(archive_path)?;
    let destination = normalize_remote_path(destination_path)?;
    let compressed = tar_z_flag(&archive)?;
    if sudo {
        // 列档先行仍是归档校验（坏档/空档在 sudo tar -t 的非零退出或空输出
        // 上暴露），再解包。
        let listing = crate::features::security::sudo::fs::sudo_exec(
            runtime,
            session_id,
            &build_tar_list_command(&archive, compressed),
            REMOTE_TAR_TIMEOUT_SECS,
        )
        .await?;
        if listing.trim().is_empty() {
            return Err("Archive is empty or could not be listed".to_string());
        }
        if contains_unsafe_members(&listing) {
            return Err(
                "Archive contains absolute or '..' members that would escape the destination"
                    .to_string(),
            );
        }
        crate::features::security::sudo::fs::sudo_exec(
            runtime,
            session_id,
            &build_extract_command(&archive, &destination, compressed),
            REMOTE_TAR_TIMEOUT_SECS,
        )
        .await?;
        return Ok(());
    }
    // Listing first validates the archive and yields its top-level entries.
    let listing = runtime
        .exec(
            session_id,
            None,
            &build_tar_list_command(&archive, compressed),
            false,
            Some(REMOTE_TAR_TIMEOUT_SECS),
        )
        .await?;
    check_exec_success(&listing, "archive listing")?;
    let members = listing
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if contains_unsafe_members(members) {
        return Err(
            "Archive contains absolute or '..' members that would escape the destination"
                .to_string(),
        );
    }
    let top_entries = unique_top_level_entries(members);
    if top_entries.is_empty() {
        return Err("Archive is empty or could not be listed".to_string());
    }
    if !overwrite {
        let sftp = runtime.sftp(session_id).await?;
        let session = sftp.lock().await;
        for entry in &top_entries {
            let candidate = format!("{}/{}", destination.trim_end_matches('/'), entry);
            if session.symlink_metadata(candidate).await.is_ok() {
                return Err(format!(
                    "Destination already contains '{entry}'; pass overwrite to replace it"
                ));
            }
        }
    }
    let outcome = runtime
        .exec(
            session_id,
            None,
            &build_extract_command(&archive, &destination, compressed),
            false,
            Some(REMOTE_TAR_TIMEOUT_SECS),
        )
        .await?;
    check_exec_success(&outcome, "extract")
}

/// Trims, drops blanks and normalizes every source path; errors when nothing
/// usable remains.
pub(crate) fn clean_source_paths(source_paths: &[String]) -> Result<Vec<String>, String> {
    let mut cleaned = Vec::new();
    for source in source_paths {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            continue;
        }
        cleaned.push(normalize_remote_path(trimmed)?);
    }
    if cleaned.is_empty() {
        return Err("At least one source path is required".to_string());
    }
    Ok(cleaned)
}

/// Longest common ancestor directory of the given absolute paths, ported from
/// tiny-rdm's `commonParentDir` with one fix: the last component of a sole
/// path is dropped, so archiving a single file still yields a usable `-C`
/// directory instead of the file itself.
pub(crate) fn common_parent_dir(paths: &[String]) -> String {
    let ancestors = paths
        .iter()
        .map(|path| {
            let mut components: Vec<&str> = path.trim_end_matches('/').split('/').collect();
            components.pop();
            components
        })
        .collect::<Vec<_>>();
    let mut common = ancestors.first().cloned().unwrap_or_default();
    for ancestor in &ancestors[1..] {
        let limit = common.len().min(ancestor.len());
        let mut shared = 0;
        while shared < limit && common[shared] == ancestor[shared] {
            shared += 1;
        }
        common.truncate(shared);
    }
    if common.len() <= 1 {
        return "/".to_string();
    }
    format!("/{}", common[1..].join("/"))
}

/// Path of `path` relative to its ancestor `parent` (result of
/// [`common_parent_dir`); `"."` as a defensive fallback, like tiny-rdm.
pub(crate) fn relative_to_parent(path: &str, parent: &str) -> String {
    let prefix = if parent == "/" { "" } else { parent };
    let relative = path.strip_prefix(&format!("{prefix}/")).unwrap_or(path);
    if relative.is_empty() {
        ".".to_string()
    } else {
        relative.to_string()
    }
}

/// `tar -czf <tmp> -C <parent> -- <rel...>` with every argument shell-quoted.
/// The `--` terminates option parsing: GNU tar still reads operands as
/// options, so a remote file named `--checkpoint-action=exec=…` would
/// otherwise execute on the server during archiving.
pub(crate) fn build_archive_command(temporary: &str, parent: &str, relatives: &[String]) -> String {
    let members = relatives
        .iter()
        .map(|relative| shell_quote(relative))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "tar -czf {} -C {} -- {}",
        shell_quote(temporary),
        shell_quote(parent),
        members
    )
}

/// `tar -t[z]f <archive>` — member listing used by the overwrite pre-check.
pub(crate) fn build_tar_list_command(archive: &str, compressed: bool) -> String {
    let z_flag = if compressed { "z" } else { "" };
    format!("tar -t{z_flag}f {}", shell_quote(archive))
}

/// `mkdir -p <dest> && tar -x[z]f <archive> -C <dest> --`. The trailing `--`
/// keeps archive-internal member names from being parsed as tar options.
pub(crate) fn build_extract_command(archive: &str, destination: &str, compressed: bool) -> String {
    let z_flag = if compressed { "z" } else { "" };
    format!(
        "mkdir -p {} && tar -x{z_flag}f {} -C {} --",
        shell_quote(destination),
        shell_quote(archive),
        shell_quote(destination)
    )
}

/// Whether the archive name calls for the gzip `z` flag; `.zip` and anything
/// else are rejected, matching tiny-rdm's tar-only support.
pub(crate) fn tar_z_flag(archive: &str) -> Result<bool, String> {
    let name = archive.to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Ok(true)
    } else if name.ends_with(".tar") {
        Ok(false)
    } else if name.ends_with(".zip") {
        Err(
            "unsupported archive type: .zip archives are not supported, re-pack as .tar.gz"
                .to_string(),
        )
    } else {
        Err(format!(
            "unsupported archive type '{archive}': only .tar.gz, .tgz and .tar are supported"
        ))
    }
}

/// Distinct top-level entry names from a `tar -t` listing. `./dir/file`,
/// `dir/` and `name` all reduce to their first component; `..` members and
/// blank lines are ignored.
pub(crate) fn unique_top_level_entries(listing: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for line in listing.lines() {
        let trimmed = line.trim().trim_start_matches("./");
        let name = trimmed.split('/').next().unwrap_or_default();
        if name.is_empty() || name == ".." {
            continue;
        }
        if seen.insert(name.to_string()) {
            entries.push(name.to_string());
        }
    }
    entries
}

/// Whether any `tar -t` member would escape the extraction destination: an
/// absolute path or a `..` segment. GNU tar strips leading `/` on extraction
/// but does not neutralize `..`, so an archive containing either must be
/// refused instead of unpacked (catastrophic under the sudo lane, where the
/// extraction runs as root).
pub(crate) fn contains_unsafe_members(listing: &str) -> bool {
    listing.lines().any(|line| {
        let trimmed = line.trim().trim_start_matches("./");
        trimmed.starts_with('/') || trimmed.split('/').any(|segment| segment == "..")
    })
}

/// Turns an exec outcome (`{success, output, exitCode}`) into an error when
/// the remote command failed.
pub(crate) fn check_exec_success(outcome: &Value, operation: &str) -> Result<(), String> {
    let exit_code = outcome
        .get("exitCode")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if exit_code == 0 {
        Ok(())
    } else {
        let output = outcome
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        Err(format!(
            "{operation} exited with status {exit_code}: {output}"
        ))
    }
}
