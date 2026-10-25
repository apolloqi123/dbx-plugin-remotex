//! Copy/move execution phase: remote exec transport, the `overwrite=false`
//! blocking precheck, the same-directory rename fast path and the per-source
//! `cp -a` / `mv -f` loop. Error mapping and result ordering are verbatim from
//! the pre-split module.

use super::*;

use std::sync::Arc;

use russh_sftp::client::SftpSession;
use tokio::sync::Mutex as AsyncMutex;

use crate::sessions::ssh::exec;
use crate::sessions::ssh::RawSftpClient;

/// Remote `cp`/`mv` commands run with this budget; recursive directory
/// copies can legitimately take a while.
const REMOTE_COPY_TIMEOUT_SECS: u64 = 300;

/// How remote commands are issued for one copy/move batch.
pub(super) enum CopyExecutor<'a> {
    /// Plugin RPC path: routed through [`SshRuntime::exec`] so session
    /// lookup, timeouts and exec bookkeeping stay in one place.
    Session(&'a SshRuntime, &'a str),
}

impl CopyExecutor<'_> {
    async fn exec(&self, command: &str) -> Result<exec::ExecOutcome, String> {
        match self {
            CopyExecutor::Session(runtime, session_id) => {
                let value = runtime
                    .exec(
                        session_id,
                        None,
                        command,
                        false,
                        Some(REMOTE_COPY_TIMEOUT_SECS),
                    )
                    .await?;
                Ok(exec::ExecOutcome {
                    output: value
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    exit_code: value.get("exitCode").and_then(Value::as_i64).unwrap_or(-1) as i32,
                })
            }
        }
    }

    async fn sftp(&self) -> Option<Arc<AsyncMutex<SftpSession>>> {
        match self {
            CopyExecutor::Session(runtime, session_id) => runtime.sftp(session_id).await.ok(),
        }
    }
}

/// latin-1 裸包车道（M17-A 工作台 / M18 MCP 工具面）：客户端 + 路径还原
/// 口径。工作台按会话新开一条 sftp 子系统通道（wire 形式还原）；MCP 工具面
/// 由调用方传入连接级裸包客户端（显示形式还原）。None = auto 语义或裸包
/// 客户端建立失败（回退字面量路径，M15 先例）。
pub(super) type RawLane = Option<(RawSftpClient, PathForm)>;

pub(super) async fn execute_with(
    executor: CopyExecutor<'_>,
    op: CopyOp,
    request: &CopyMoveRequest,
    mut raw: RawLane,
) -> CopyMoveOutcome {
    let targets: Vec<String> = request
        .from
        .iter()
        .map(|source| target_path(&request.to_dir, source))
        .collect();

    // latin-1：from/toDir 按调用方面（工作台 wire 形式 / MCP 显示形式）整条
    // 还原字节。裸包客户端可用时，覆盖预检（逐个裸包 LSTAT）与同目录 move
    // 的 RENAME 快路径都走字节保真；None（auto 或建立失败）回退既有字面量
    // 路径（M15 先例）。
    //
    // 设计边界（登记，PROTOCOL 同步）：底层执行仍是远端服务器侧 `cp -a` /
    // `mv -f` shell 命令——SSH exec 的命令串是 UTF-8 String，含转义（%XX）
    // /非 ASCII 的路径只能按字面量拼接，服务器原始字节经 shell 参数不可控，
    // 故 copy 与跨目录 move 的执行层不做字节保真迁移：clean 名（无转义）
    // 行为不变，非 ASCII 名由服务器侧报错（`cp: cannot stat` 类），预检/
    // 改名快路径已迁移的部分保证覆盖判定与同目录移动正确。

    // With overwrite disabled, block every item whose target already exists
    // before running anything. latin-1 + 裸包车道：逐个裸包 LSTAT（按
    // PathForm 口径还原目标字节）；其余走一轮 shell 探测（整批一个往返）。
    let mut blocked = vec![false; targets.len()];
    if !request.overwrite {
        let mut probed = false;
        if let Some((client, form)) = raw.as_mut() {
            for (index, target) in targets.iter().enumerate() {
                match crate::filesystem::sftp::ext::raw_exists_decision(
                    client.lstat(&form.decode(target)).await,
                ) {
                    Ok(true) => blocked[index] = true,
                    Ok(false) => {}
                    Err(error) => {
                        // 无法判定占用时按占用拦下：放行会让后续 cp -a
                        // 静默覆盖既有目标——overwrite 保护优先于可用性。
                        eprintln!(
                            "[ssh] sftp {} existence probe errored for {target}: {error}; blocking the item",
                            op.label()
                        );
                        blocked[index] = true;
                    }
                }
            }
            probed = true;
        }
        if !probed {
            match executor.exec(&build_exists_probe_command(&targets)).await {
                Ok(outcome) if outcome.exit_code == 0 => {
                    blocked = existing_targets(&outcome.output, targets.len());
                }
                Ok(outcome) => {
                    // fail-closed：overwrite=false 的整批占用预检失败后继续
                    // 逐项执行，后续 cp -a 会静默覆盖既有目标——整批拒绝，
                    // 让调用方在可确认的状态下重试。
                    let reason = format!(
                        "existence probe failed (exit {}): {}; refusing to continue with overwrite protection unverified",
                        outcome.exit_code,
                        outcome.output.trim()
                    );
                    eprintln!("[ssh] sftp {} {reason}", op.label());
                    return CopyMoveOutcome {
                        success: false,
                        results: request
                            .from
                            .iter()
                            .zip(targets.iter())
                            .map(|(source, target)| {
                                ItemOutcome::failed(source, target, reason.clone())
                            })
                            .collect(),
                    };
                }
                Err(error) => {
                    let reason = format!(
                        "existence probe failed: {error}; refusing to continue with overwrite protection unverified"
                    );
                    eprintln!("[ssh] sftp {} {reason}", op.label());
                    return CopyMoveOutcome {
                        success: false,
                        results: request
                            .from
                            .iter()
                            .zip(targets.iter())
                            .map(|(source, target)| {
                                ItemOutcome::failed(source, target, reason.clone())
                            })
                            .collect(),
                    };
                }
            }
        }
    }

    // Only moves use the SFTP-rename fast path; the channels are opened
    // lazily and their absence simply disables the optimization.
    let sftp = match op {
        CopyOp::Move => executor.sftp().await,
        CopyOp::Copy => None,
    };

    let mut results = Vec::with_capacity(request.from.len());
    for (index, source) in request.from.iter().enumerate() {
        let target = &targets[index];
        if blocked[index] {
            results.push(ItemOutcome::failed(
                source,
                target,
                format!("target already exists: {target} (pass overwrite to replace)"),
            ));
            continue;
        }

        // Fast path: same-directory moves are a plain rename. latin-1 优先
        // 走裸包 RENAME（按 PathForm 口径还原字节，字节保真；SFTPv3 不覆盖
        // 已存在目标，撞名/跨设备失败与高层快路径同样回落 shell mv）。
        if op == CopyOp::Move && same_directory(source, &request.to_dir) {
            let attempted = match raw.as_mut() {
                Some((client, form)) => Some(
                    client
                        .rename(&form.decode(source), &form.decode(target))
                        .await,
                ),
                None => match &sftp {
                    Some(sftp) => Some(
                        sftp.lock()
                            .await
                            .rename(source.clone(), target.clone())
                            .await
                            .map_err(|error| error.to_string()),
                    ),
                    None => None,
                },
            };
            match attempted {
                Some(Ok(())) => {
                    results.push(ItemOutcome::ok(source, target));
                    continue;
                }
                Some(Err(error)) => {
                    eprintln!(
                        "[ssh] sftp rename fast path failed for {source} -> {target}: {error}; falling back to shell mv"
                    );
                }
                None => {}
            }
        }

        let command = match op {
            CopyOp::Copy => build_copy_command(source, target),
            CopyOp::Move => build_move_command(source, target),
        };
        match executor.exec(&command).await {
            Ok(outcome) if outcome.exit_code == 0 => {
                results.push(ItemOutcome::ok(source, target));
            }
            Ok(outcome) => {
                let detail = outcome.output.trim();
                results.push(ItemOutcome::failed(
                    source,
                    target,
                    format!(
                        "{} failed (exit {}): {detail}",
                        op.label(),
                        outcome.exit_code
                    ),
                ));
            }
            Err(error) => {
                results.push(ItemOutcome::failed(
                    source,
                    target,
                    format!("{} failed: {error}", op.label()),
                ));
            }
        }
    }

    CopyMoveOutcome {
        success: results.iter().all(|item| item.ok),
        results,
    }
}
