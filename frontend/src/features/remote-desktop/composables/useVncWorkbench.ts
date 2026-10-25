import type { Ref } from "vue";
import { loadLastConnectParams } from "../../connections/lib/connectLastParams";
import type { VncConnectOptions } from "../components/VncConnectDialog.vue";

/** 连接上下文（host/port 直启 VNC 用）。 */
export interface VncConnectionContext {
  host?: string;
  port?: number;
}

export interface VncWorkbenchOptions {
  connection: Ref<VncConnectionContext>;
  startVncSession: (connectOptions: VncConnectOptions) => Promise<boolean>;
}

/** VNC 连接路由（连接记录直启）：只适配 App 的连接上下文与
 * useVncSession 的启动出口，会话生命周期仍由 useVncSession 持有。
 *
 * App.vue 只做装配：模板同名绑定经解构透出。 */
export function useVncWorkbench(options: VncWorkbenchOptions) {
  const { connection, startVncSession } = options;

// —— VNC 会话生命周期（nyaterm-parity P2 2d，与 Telnet 同款互斥展示）——
// 画面走 VncSurface 画布；帧/输入/剪贴板各走独立通道。连接/关闭/输入/帧
// 入口已收口在 composables/useVncSession，App.vue 只保留连接路由与装配。

// —— 连接驱动的会话启动（对标 Tabby profile 打开）：非 SSH 连接从宿主
// 连接表单直启各自会话；参数 = 连接 host/port + 上次使用记忆的偏好。
// 凭据不随连接持久化，需要自动登录/VNC 密码时从工具栏弹窗进入；直启
// 失败回落弹窗（预填 host/port，可改后重试）。
async function startVncFromConnection(): Promise<boolean> {
  const conn = connection.value;
  if (!conn.host) return false;
  const last = loadLastConnectParams<VncConnectOptions>("vnc-connect-last");
  return startVncSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 5900,
    scaleMode: last.scaleMode === "stretch" || last.scaleMode === "actual" ? last.scaleMode : "fit",
  });
}

  return {
    startVncFromConnection,
  };
}
