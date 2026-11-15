//! Local RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_local(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // 本地终端：sidecar 所在机器上的交互式登录 shell。入口在 SSH
            // 工作台内，用户显式点击才会创建（默认行为零改变）；注入的
            // shell integration 只做装饰/cwd 跟踪，绝不执行其数据。
            "local/terminal/start" => {
                let request: crate::sessions::local::LocalTerminalStartRequest = parse(params)?;
                self.runtime
                    .block_on(self.local.start(request, emitter.clone()))
            }

            "local/terminal/resize" => {
                let session_id = required_string(&params, "sessionId")?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                self.runtime
                    .block_on(self.local.resize(session_id, cols, rows))?;
                Ok(json!({ "success": true }))
            }

            "local/terminal/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let replay = self.runtime.block_on(self.local.replay(
                    session_id,
                    after_sequence,
                    emitter,
                ))?;
                Ok(replay)
            }

            "local/session/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.local.close(session_id))?;
                Ok(json!({ "success": true }))
            }

            // 本地终端录制：与 ssh/recording 同形（同一 asciicast 目录与
            // 回放链），录制器挂在本地 PTY 泵上，shell 退出随会话槽位释放。
            "local/recording/start" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.local.recording_start(session_id, &self.ssh.data_dir()))
            }

            "local/recording/stop" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.local.recording_stop(session_id))
            }

            "local/recording/event" => {
                let session_id = required_string(&params, "sessionId")?;
                let data = params.get("data").cloned().unwrap_or(json!({}));
                self.runtime
                    .block_on(self.local.recording_event(session_id, &data))
            }

            "local/session/list" => Ok(self.runtime.block_on(self.local.list())),

            // 本地终端 shell 发现：工作台选择器用（多平台 shell 设置）。
            "local/shells/list" => Ok(self.local.shells()),

            // PR-A4 generic launch-options contract: picker entries for the dock "+".
            // locale = 宿主 UI 语言（宿主侧 options_action 契约；旧宿主不传，回落英文）；
            // 描述展示解析后的默认 Shell（配置的 localShell 优先，否则自动探测链）。
            // 已显式配置 localShell 时折叠发现分组——只留配置的那一个入口。
            "local/terminal/launch-options" => {
                let configured =
                    crate::sessions::local::resolve_configured_shell(&plugin_data_dir());
                let default_shell = configured.clone().unwrap_or_else(|| {
                    crate::sessions::local::resolve_default_shell(&plugin_data_dir())
                });
                Ok(self.local.launch_options(
                    params.get("locale").and_then(Value::as_str).unwrap_or(""),
                    &default_shell,
                    configured.as_deref(),
                ))
            }

            // 本机落盘能力探测：无宿主 fileTransfer API 时前端据此决定
            // 走 sidecar 下载目录落盘还是浏览器 <a download> 兜底。
            "local/capabilities" => {
                let data_dir = plugin_data_dir();
                let downloads_dir = crate::filesystem::local::downloads::downloads_base_dir(
                    |key| std::env::var_os(key),
                    &data_dir,
                );
                Ok(json!({
                    "canSaveLocal": crate::filesystem::local::downloads::can_save_local(|key| std::env::var_os(key)),
                    "downloadsDir": downloads_dir.to_string_lossy(),
                    "platform": crate::filesystem::local::downloads::platform_name(),
                }))
            }

            // 通用本机落盘：不经过 SFTP 传输链的本地产物（录制 GIF 导出等）。
            // 与下载共用目录语义；targetDir 缺省落下载目录，必须绝对路径。
            "local/saveFile" => {
                let name = required_string(&params, "name")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                let data = BASE64_STANDARD
                    .decode(data_base64)
                    .map_err(|error| format!("dataBase64 is not valid base64: {error}"))?;
                let target_dir = params.get("targetDir").and_then(Value::as_str);
                let conflict = params.get("conflict").and_then(Value::as_str);
                crate::filesystem::local::downloads::save_local_file(
                    &plugin_data_dir(),
                    name,
                    &data,
                    target_dir,
                    conflict,
                )
            }

            // 背景图（P2-9）：桌面形态落盘 <plugin_data_dir>/wallpaper（≤8MiB，
            // png/jpeg/webp 魔数校验）；web/docker 形态 sidecar 存储不在本机时，
            // 前端对 set 失败降级为仅本次会话内存态。
            "local/wallpaper/get" => {
                Ok(crate::core::preferences::load_wallpaper(&plugin_data_dir()))
            }

            "local/wallpaper/set" => {
                crate::core::preferences::save_wallpaper(&plugin_data_dir(), &params)
            }

            "local/wallpaper/clear" => {
                Ok(crate::core::preferences::clear_wallpaper(&plugin_data_dir()))
            }

            // 应用内目录选择器的本机浏览：只列目录（永不返回文件内容）；
            // drives 供 Windows「此电脑」盘符页，其他平台为空。
            "local/fs/browse" => {
                let path = params.get("path").and_then(Value::as_str);
                crate::filesystem::local::fs::browse_local_dir(path, &plugin_data_dir())
            }

            "local/fs/drives" => {
                Ok(json!({ "drives": crate::filesystem::local::fs::list_local_drives() }))
            }

            // 「询问我」冲突策略的预检：目标目录下同名文件是否已存在。
            "local/fs/exists" => {
                let dir = required_string(&params, "dir")?;
                let name = required_string(&params, "name")?;
                crate::filesystem::local::fs::target_exists(dir, name)
            }

            // 在文件管理器中定位已完成的下载。只允许 reveal 传输历史里
            // 记录过的 localPath，不能成为任意路径打开原语。目标文件已被
            // 移走/改名时回落到其父目录，再退到插件的下载目录（配置或
            // 默认），而不是让文件管理器落到系统的文档目录（issue #18）。
            "local/reveal" => {
                let path = required_string(&params, "path")?;
                let data_dir = plugin_data_dir();
                let history = crate::features::transfer::history::load_history(&data_dir);
                let recorded = std::path::PathBuf::from(&path);
                crate::filesystem::local::downloads::reveal_validated(&history, &recorded)?;
                let target = crate::filesystem::local::downloads::reveal_target(
                    &recorded,
                    &crate::filesystem::local::downloads::reveal_download_dir(&data_dir),
                );
                crate::filesystem::local::downloads::reveal_in_file_manager(&target)?;
                Ok(json!({ "success": true }))
            }

            // 在默认应用中打开已完成的本机下载；同样只允许打开传输历史中
            // 记录过的路径，不能成为任意路径执行原语。
            "local/open" => {
                let path = required_string(&params, "path")?;
                let history = crate::features::transfer::history::load_history(&plugin_data_dir());
                crate::filesystem::local::downloads::open_validated(
                    &history,
                    std::path::Path::new(path),
                )?;
                Ok(json!({ "success": true }))
            }

            // 按扩展名映射的编辑器打开（`local/editors/list` 提供目录）：
            // editorId 走知名编辑器目录，custom 走用户自定义命令行（argv
            // 化、不过 shell）；路径 allowlist 与 local/open 同源。
            "local/editors/list" => Ok(crate::filesystem::local::editors::editors_list_payload()),

            "local/open-with" => {
                let path = required_string(&params, "path")?;
                let spec = crate::filesystem::local::editors::launch_spec_from_params(&params)?;
                let history = crate::features::transfer::history::load_history(&plugin_data_dir());
                crate::filesystem::local::editors::open_with_validated(
                    &history,
                    std::path::Path::new(&path),
                    &spec,
                )?;
                Ok(json!({ "success": true }))
            }
            // 原生拖拽指针探针（SFTP 拖放落点校准）：只返回光标与宿主窗口
            // 几何；宿主转发坐标被多除了一次缩放比例，插件用同一时刻的
            // 真实光标反解 iframe 偏移。非 macOS 返回空样本。
            "local/pointer/sample" => {
                let sample = crate::platform::pointer::sample();
                Ok(json!({
                    "platform": sample.platform,
                    "cursor": sample.cursor.map(|point| json!({ "x": point.x, "y": point.y })),
                    "window": sample.window.map(|rect| json!({
                        "x": rect.x,
                        "y": rect.y,
                        "width": rect.width,
                        "height": rect.height,
                    })),
                }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
