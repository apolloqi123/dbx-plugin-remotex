//! Remote Desktop RPC handlers.

use super::*;
use crate::platform::diagnostics::*;

impl Plugin {
    pub(super) fn handle_remote_desktop(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // VNC 远程桌面会话（RFB 6143 客户端，nyaterm-parity P2 2d）：入口在
            // SSH 工作台工具栏，用户显式点击才会创建。引擎为上游 vnc-rs 0.6
            // （尽调见 docs/plans/SPIKE_VNC_SESSION.zh-CN.md）；仅声明 ZRLE+Raw 编码，
            // 帧缓冲上限 3840x2160；帧以 44 字节 patch 头（RGBA）走
            // `vnc/frame/{id}` 二进制通道，生命周期事件 `vnc/session/state`，
            // 远端剪贴板更新走 `vnc/clipboard` 事件。classic VNC-Auth 密码
            // ≤8 字节，仅建议在可信网络使用（None 认证为明文协议）。
            "vnc/start" => {
                let mut request: crate::sessions::vnc::VncStartRequest = parse(params)?;
                // 表单密码合并：请求已带密码（弹窗路径）时不覆盖。
                if request
                    .password
                    .as_deref()
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    if let Some((_, password)) = self
                        .non_ssh_creds
                        .get(request.connection_id.as_deref().unwrap_or_default())
                    {
                        request.password = Some(password);
                    }
                }
                self.runtime
                    .block_on(self.vnc.start(request, emitter.clone()))
            }

            // 键盘/指针事件转发（keysym 由前端映射后传入）；`vnc/write` 为
            // 同语义别名。
            "vnc/input" | "vnc/write" => {
                let request: crate::sessions::vnc::VncInputRequest = parse(params)?;
                self.runtime
                    .block_on(self.vnc.input(&request.session_id, request.event))?;
                Ok(json!({ "success": true }))
            }

            // 仅前端缩放（fit/stretch/actual），不改远端分辨率。
            "vnc/resize" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.vnc.resize(session_id))?;
                Ok(json!({ "success": true }))
            }

            // 手动重连：generation 计数防串话，保留帧缓冲做整幅重绘。
            "vnc/reconnect" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.vnc.reconnect(session_id, emitter.clone()))
            }

            // 本地剪贴板 → 远端（Latin-1、≤1MiB）。
            "vnc/set-clipboard" => {
                let session_id = required_string(&params, "sessionId")?;
                let text = required_string(&params, "text")?.to_string();
                self.runtime
                    .block_on(self.vnc.set_clipboard(session_id, text))?;
                Ok(json!({ "success": true }))
            }

            "vnc/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.vnc.close(session_id))?;
                Ok(json!({ "success": true }))
            }

            "vnc/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.vnc.replay(session_id, emitter))
            }

            "vnc/list" => Ok(self.runtime.block_on(self.vnc.list())),

            // RDP 远程桌面会话（RDP-2，nyaterm-parity P3-4）：引擎为 RDP-1
            // vendored IronRDP 链（0.17 lockstep）。范围（评审定案，见
            // docs/reviews/RDP_CREDSSP_REVIEW_CHECKLIST.zh-CN.md）：密码/NLA（CredSSP）
            // + TLS + 文本剪贴板 + 断线重连；不做音频/驱动器重定向/网关/UDP/
            // Kerberos。桌面帧以 44 字节 patch 头走 `rdp/frame/{id}`（与
            // vnc/frame 同族），生命周期事件 `rdp/session/state`，远端剪贴板
            // 更新走 `rdp/clipboard` 事件。安全红线：密码以 Zeroizing 持有、
            // 不进日志/审计/错误；证书策略 fail-closed（prompt 默认、120s
            // 确认窗、remember 记入 rdp-known-certs.json）；剪贴板 text-only
            // + 16 MiB 上限；认证类失败不自动重连。
            "rdp/start" => {
                rdp_start_gate(&plugin_data_dir())?;
                let mut request: crate::sessions::rdp::RdpStartRequest = parse(params)?;
                // 表单密码/用户名合并：请求已带密码（弹窗路径）时不覆盖；
                // 用户名只在请求留空时补（连接表单 username 对 telnet/rdp 可见）。
                if request
                    .password
                    .as_deref()
                    .map(str::is_empty)
                    .unwrap_or(true)
                {
                    let merge_id = request
                        .connection_id
                        .as_deref()
                        .unwrap_or_default()
                        .to_string();
                    if let Some((username, password)) = self
                        .non_ssh_creds
                        .get(request.connection_id.as_deref().unwrap_or_default())
                    {
                        log_rdp_debug(format!(
                            "rdp/start merge id={merge_id}: credentials merged (username set)"
                        ));
                        request.password = Some(password);
                        if request.username.trim().is_empty() && !username.is_empty() {
                            request.username = username;
                        }
                    } else {
                        log_rdp_debug(format!(
                            "rdp/start merge id={merge_id}: no staged credentials (password stays empty)"
                        ));
                    }
                }
                self.runtime
                    .block_on(self.rdp.start(request, emitter.clone(), &plugin_data_dir()))
            }

            // 键盘/指针事件转发（scancode + extended 位，映射同 NyaTerm）；
            // `rdp/write` 为同语义别名。
            "rdp/input" | "rdp/write" => {
                let request: crate::sessions::rdp::RdpInputRequest = parse(params)?;
                self.runtime
                    .block_on(self.rdp.input(&request.session_id, request.input))?;
                Ok(json!({ "success": true }))
            }

            // 服务端动态分辨率：resize = 以新尺寸重连（会话内 display control
            // PDU 真机不生效，见 crate::sessions::rdp::resize 文档与交付报告）。
            "rdp/resize" => {
                let request: crate::sessions::rdp::RdpResizeRequest = parse(params)?;
                self.runtime.block_on(self.rdp.resize(
                    &request.session_id,
                    request.width,
                    request.height,
                    emitter.clone(),
                ))
            }

            // 本地剪贴板 → 远端（text-only，16 MiB 上限）。
            "rdp/set-clipboard" => {
                let session_id = required_string(&params, "sessionId")?;
                let text = required_string(&params, "text")?.to_string();
                self.runtime
                    .block_on(self.rdp.set_clipboard(session_id, text))?;
                Ok(json!({ "success": true }))
            }

            // 手动重连：generation 计数防串话。
            "rdp/reconnect" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.rdp.reconnect(session_id, emitter.clone()))
            }

            // 证书确认应答（`connection/challenge` kind=rdp-certificate）。
            // 缺省 accept=false：超时/取消一律拒绝（fail-closed）。
            "rdp/certificate/resolve" => {
                let challenge_id = required_string(&params, "challengeId")?;
                let accept = params
                    .get("accept")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let remember = params
                    .get("remember")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.rdp
                    .resolve_certificate(challenge_id, accept, remember)?;
                Ok(json!({ "success": true }))
            }

            "rdp/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.rdp.close(session_id))?;
                Ok(json!({ "success": true }))
            }

            "rdp/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.rdp.replay(session_id, emitter))
            }

            "rdp/list" => Ok(self.runtime.block_on(self.rdp.list())),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
