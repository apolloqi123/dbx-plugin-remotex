//! Server-side copy/move (`sftp/copy`, `sftp/move`), mirroring tiny-rdm's
//! server-internal copy & paste: sources are copied/moved *into* `toDir`
//! under their own name via the session's shell (`cp -a --` / `mv -f --`).
//! Moves inside one directory first try an SFTP rename and fall back to the
//! shell. No sudo variant (tiny-rdm has none either).
//!
//! latin-1 字节保真（M17 增量①工作台、M18 MCP 工具面）：工作台 `from`/
//! `toDir` 是列表回传的 wire 形式（MCP 面为显示形式，见 [`PathForm`]）。
//! 覆盖预检（逐个裸包 LSTAT，目标按 [`PathForm`] 还原字节）与同目录 move
//! 的 RENAME 快路径（裸包 RENAME）走原始字节；底层 shell `cp`/`mv` 的
//! exec 命令串是 UTF-8 String，服务器原始字节经 shell 参数不可控——copy
//! 与跨目录 move 的执行层保持字面量发送（clean 名不受影响，转义/非 ASCII
//! 名由服务器侧报错），边界登记见 PROTOCOL。

use serde_json::Value;

use crate::filesystem::sftp::name::NameEncoding;
use crate::sessions::ssh::SshRuntime;

mod exec;
mod plan;

use exec::{execute_with, CopyExecutor, RawLane};
pub use plan::{
    build_copy_command, build_exists_probe_command, build_move_command, existing_targets,
    parse_request, same_directory, target_path, CopyMoveOutcome, CopyMoveRequest, CopyOp,
    ItemOutcome, PathForm,
};

/// `sftp/copy` / `sftp/move` entry for the plugin RPC. The session is
/// resolved by the caller (connection id or session id).
pub async fn run(
    runtime: &SshRuntime,
    session_id: &str,
    op: CopyOp,
    params: &Value,
    encoding: NameEncoding,
) -> Result<Value, String> {
    runtime.ensure_writable(session_id).await?;
    let request = parse_request(params)?;
    // latin-1（M17-A）：按会话新开一条裸包通道（wire 形式还原）；建立失败
    // 回退既有字面量路径（raw = None，M15 先例）。
    let raw: RawLane = match encoding {
        NameEncoding::Latin1 => runtime
            .raw_sftp_client(session_id)
            .await
            .ok()
            .map(|client| (client, PathForm::WireEscaped)),
        NameEncoding::Auto => None,
    };
    let outcome = execute_with(
        CopyExecutor::Session(runtime, session_id),
        op,
        &request,
        raw,
    )
    .await;
    Ok(outcome.into_json())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::sftp::name as sftp_name;
    use serde_json::json;

    #[test]
    fn parse_request_accepts_single_string_and_array() {
        let single = parse_request(&json!({
            "connectionId": "c1",
            "from": "/var/log/app.log",
            "toDir": "/tmp",
        }))
        .unwrap();
        assert_eq!(
            single,
            CopyMoveRequest {
                from: vec!["/var/log/app.log".to_string()],
                to_dir: "/tmp".to_string(),
                overwrite: false,
            }
        );

        let batch = parse_request(&json!({
            "connectionId": "c1",
            "from": ["/a/b.txt", " /c/d.txt ", "", "/x/../e"],
            "toDir": "/dst/",
            "overwrite": true,
        }))
        .unwrap();
        assert_eq!(batch.from, vec!["/a/b.txt", "/c/d.txt", "/e"]);
        assert_eq!(batch.to_dir, "/dst");
        assert!(batch.overwrite);
    }

    #[test]
    fn parse_request_rejects_invalid_params() {
        let error = parse_request(&json!({ "from": "/a" })).unwrap_err();
        assert_eq!(error, "Missing toDir");

        let error = parse_request(&json!({ "toDir": "/tmp" })).unwrap_err();
        assert_eq!(error, "Missing from");

        let error = parse_request(&json!({ "from": ["/a", 3], "toDir": "/tmp" })).unwrap_err();
        assert!(error.contains("array of strings"), "{error}");

        let error = parse_request(&json!({ "from": [], "toDir": "/tmp" })).unwrap_err();
        assert!(error.contains("At least one source"), "{error}");

        let error = parse_request(&json!({ "from": "  ", "toDir": "/tmp" })).unwrap_err();
        assert!(error.contains("At least one source"), "{error}");

        let error = parse_request(&json!({ "from": "/", "toDir": "/tmp" })).unwrap_err();
        assert!(error.contains("filesystem root"), "{error}");
    }

    #[test]
    fn copy_and_move_commands_quote_every_argument() {
        assert_eq!(
            build_copy_command("/var/my app.log", "/tmp/x"),
            "cp -a -- '/var/my app.log' '/tmp/x'"
        );
        assert_eq!(
            build_move_command("/var/it's", "/tmp"),
            r"mv -f -- '/var/it'\''s' '/tmp'"
        );
    }

    #[test]
    fn target_paths_keep_the_source_name() {
        assert_eq!(target_path("/tmp", "/var/log/app.log"), "/tmp/app.log");
        assert_eq!(target_path("/tmp/", "/var/log"), "/tmp/log");
        assert_eq!(target_path("/tmp", "/opt"), "/tmp/opt");
    }

    /// 目标目录形态边界（T2 补充，对标 electerm 传输幂等语义）：
    /// `parse_request` 先把 toDir 过 `normalize_remote_path`（收敛 `..`、
    /// 折叠 `//`），所以到达 `target_path` 的 toDir 已是规范化绝对路径；
    /// 本矩阵单独压 `target_path` 纯函数本身，保证调用方（copy/move 批处理）
    /// 无论传入什么形态都拿到**单个斜杠分隔**的绝对目标——拼接结果出现
    /// 空段或双斜杠会让 `cp/mv` 与探测命令语义漂移。
    #[test]
    fn target_path_handles_root_and_trailing_slashes() {
        // to_dir 为根 `/`：trim_end_matches('/') 后是空串，结果以 "/" 开头
        // 而不是变成相对路径（空串拼接会产出 "name"，copy 会落到远端 CWD）。
        assert_eq!(target_path("/", "/var/log/app.log"), "/app.log");
        // 多个尾斜杠：全部收敛，不产生 "tmp//name"。
        assert_eq!(target_path("/tmp///", "/var/log/app.log"), "/tmp/app.log");
        // 根带多尾斜杠（normalize 后不会出现，防御纯函数自身）。
        assert_eq!(target_path("///", "/etc/hosts"), "/hosts");
    }

    /// `from` 的名字段本身带 `/`（调用方没先取 basename）时，`target_path`
    /// 取的是**最后一段**名字，绝不会把整条 from 拼进目标——这是天然的
    /// 路径穿越防线：`/var/log/../etc/passwd` 只会落到 `<toDir>/passwd`，
    /// 不会逃出 toDir。但反过来说，若上游未做 normalize，`a/b` 这类输入
    /// 会静默丢前缀，语义是"取名"而不是"拒绝"。
    ///
    /// 契约（GAP-2 已登记，行为按设计保留）：`target_path` 依赖上游
    /// `parse_request` 先对 from/toDir 过 `normalize_remote_path`（`..` 收敛、
    /// 拒绝根）。本函数自身不拒绝含 `..`/多段的名字输入——它按最后一段取名，
    /// 非 basename 输入被静默改写而非报错。若未来有新调用方绕过
    /// `parse_request`，必须先在其入口补 normalize，或在这里加单段断言/
    /// 显式 Err，二者缺一不可。
    #[test]
    fn target_path_takes_only_the_final_name_segment() {
        // 非 basename 输入：落到 toDir 下最后一个段，前缀被丢而不是拼接。
        assert_eq!(target_path("/dst", "a/b/c.txt"), "/dst/c.txt");
        // `..` 段名字：`/var/log/../etc/passwd` 的最后段是 passwd，目标
        // 仍在 /dst 内，无法穿越出 toDir。
        assert_eq!(target_path("/dst", "/var/log/../etc/passwd"), "/dst/passwd");
        assert!(!target_path("/dst", "/var/log/../etc/passwd").contains(".."));
        // 空名字（from 全是斜杠）：回退为 "copy" 占位名，不产生尾斜杠目标。
        assert_eq!(target_path("/dst", "///"), "/dst/copy");
    }

    #[test]
    fn same_directory_detects_rename_candidates() {
        assert!(same_directory("/tmp/a.txt", "/tmp"));
        assert!(same_directory("/a", "/"));
        assert!(!same_directory("/var/log/app.log", "/tmp"));
        assert!(!same_directory("/tmp/sub/a.txt", "/tmp"));
    }

    #[test]
    fn probe_command_lists_every_target_with_true_tail() {
        let command = build_exists_probe_command(&["/tmp/one".to_string(), "/tmp/two".to_string()]);
        assert_eq!(
            command,
            "test -e '/tmp/one' && echo 0; test -e '/tmp/two' && echo 1; true"
        );
    }

    #[test]
    fn existing_targets_reduce_probe_output() {
        assert_eq!(
            existing_targets("1\nnoise\n0\n", 3),
            vec![true, true, false]
        );
        assert_eq!(existing_targets("", 2), vec![false, false]);
        // Out-of-range indices are ignored.
        assert_eq!(existing_targets("5", 2), vec![false, false]);
    }

    #[test]
    fn wire_targets_unescape_to_server_bytes() {
        // latin-1（M17 增量①）raw 预检/同名 move 快路径的目标字节：from 与
        // toDir 都是列表回传的 wire 形式，target_path 拼接后整条还原。
        let target = target_path("/tmp/caf%E9", "/src/%FFitem.txt");
        assert_eq!(
            sftp_name::unescape_wire(&target),
            b"/tmp/caf\xe9/\xffitem.txt".to_vec()
        );
        // clean 名（无转义）按字面量透传，行为与 auto 一致。
        let target = target_path("/tmp", "/var/log/app.log");
        assert_eq!(sftp_name::unescape_wire(&target), b"/tmp/app.log".to_vec());
        // '%' 自转义闭环：真实名字里的字面 %XX 往返不吞。
        let target = target_path("/tmp", "/src/a%2541b");
        assert_eq!(sftp_name::unescape_wire(&target), b"/tmp/a%41b".to_vec());
    }

    #[test]
    fn outcome_json_matches_the_frontend_contract() {
        let outcome = CopyMoveOutcome {
            success: false,
            results: vec![
                ItemOutcome::ok("/a/one.txt", "/dst/one.txt"),
                ItemOutcome::failed(
                    "/a/two.txt",
                    "/dst/two.txt",
                    "copy failed (exit 1): cp: cannot stat".to_string(),
                ),
            ],
        };
        let value = outcome.into_json();
        assert_eq!(value["success"], false);
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["from"], "/a/one.txt");
        assert_eq!(results[0]["to"], "/dst/one.txt");
        assert_eq!(results[0]["ok"], true);
        assert!(results[0].get("error").is_none(), "ok items carry no error");
        assert_eq!(results[1]["ok"], false);
        assert_eq!(results[1]["error"], "copy failed (exit 1): cp: cannot stat");

        let all_ok = CopyMoveOutcome {
            success: true,
            results: vec![ItemOutcome::ok("/a", "/b")],
        };
        assert_eq!(all_ok.into_json()["success"], true);
    }
}
