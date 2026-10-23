import { computed, nextTick, onScopeDispose, ref, type Ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import type { VncConnectOptions } from "../components/VncConnectDialog.vue";
import type { VncInputEvent } from "../lib/vncFrame";

/** VNC 会话状态机（与 sidecar vnc/session/state 事件一一对应）。 */
export type VncSessionPhase = "idle" | "connecting" | "running" | "closed";

export interface VncSessionInfo {
  sessionId: string;
  host: string;
  port: number;
}

/** VncSurface 暴露的会话侧句柄（acceptFrame/reset + 组件 $el 用于首帧前聚焦）。 */
export interface VncSurfaceHandle {
  acceptFrame(frame: Uint8Array): boolean;
  reset(): void;
  $el?: any;
}

/** VNC 会话生命周期域：连接/关闭/输入/剪贴板/帧入口/事件态收口。
 *
 * 与 useRdpSession 同构：App.vue 只做装配（模板同名绑定经解构透出），
 * vnc/start、vnc/close、vnc/input、vnc/set-clipboard、vnc/replay 的 RPC
 * 字符串与载荷保持原样；连接失败仍走 showError(cause, "terminal")。VNC 无
 * 证书/认证挑战弹窗（classic 密码由连接表单透传，认证结果经
 * vnc/session/state 事件回落退出覆盖层）。 */
export function useVncSession(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  connectionId: Ref<string>;
  workbenchId: Ref<string>;
  runtimeEndpoint: Ref<{ host?: string; port?: number }>;
  isDisposed: () => boolean;
  getTerminal: () => { focus(): void } | undefined;
}) {
  const {
    t,
    showError,
    showNotice,
    connectionId,
    workbenchId,
    runtimeEndpoint,
    isDisposed,
    getTerminal,
  } = options;

  // VNC 会话（nyaterm-parity P2 2d）：与 SSH/本地/Telnet/串口同款互斥展示，
  // 并入 localUiMode。与终端会话不同，VNC 画面走 VncSurface 画布（xterm
  // 仍然挂着但被画布盖住），帧从 vnc/frame/{id} 二进制通道解码成 patch。
  const vncSession = ref<VncSessionInfo | null>(null);
  const vncDialogOpen = ref(false);
  const vncState = ref<VncSessionPhase>("idle");
  const vncError = ref("");
  const vncScaleMode = ref<VncConnectOptions["scaleMode"]>("fit");
  const vncSurface = ref<VncSurfaceHandle | null>(null);
  const isVncMode = computed(() => vncSession.value !== null);
  const vncTarget = computed(() => (vncSession.value ? `${vncSession.value.host}:${vncSession.value.port}` : ""));

  // 退出态统一入口：error 为 null 表示 sidecar 未带原因（主动关闭），
  // 非空时在退出覆盖层展示（认证失败/服务端强制 Tight 等）。
  function markVncClosed(error: string | null) {
    if (!vncSession.value || vncState.value === "closed") return;
    vncState.value = "closed";
    if (error !== null) vncError.value = error;
  }

  async function startVncSession(connectOptions: VncConnectOptions): Promise<boolean> {
    // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
    if (vncSession.value && vncState.value !== "closed") await closeVncSession();
    try {
      const info = await rpc<{ sessionId: string; host: string; port: number }>("vnc/start", {
        connectionId: connectionId.value || undefined,
        workbenchId: workbenchId.value,
        host: connectOptions.host,
        port: connectOptions.port,
        runtimeHost: runtimeEndpoint.value.host || connectOptions.host,
        runtimePort: runtimeEndpoint.value.port || connectOptions.port,
        scaleMode: connectOptions.scaleMode,
        ...(connectOptions.password ? { password: connectOptions.password } : {}),
      });
      if (isDisposed()) {
        void rpc("vnc/close", { sessionId: info.sessionId }).catch(() => undefined);
        return false;
      }
      vncSession.value = { sessionId: info.sessionId, host: info.host, port: info.port };
      vncScaleMode.value = connectOptions.scaleMode;
      vncState.value = "connecting";
      vncError.value = "";
      vncSurface.value?.reset();
      await nextTick();
      vncSurface.value?.$el?.querySelector("canvas")?.focus();
      return true;
    } catch (cause) {
      showError(cause, "terminal");
      return false;
    }
  }

  async function closeVncSession() {
    const sessionId = vncSession.value?.sessionId;
    vncSession.value = null;
    vncState.value = "idle";
    vncError.value = "";
    if (!sessionId) return;
    await rpc("vnc/close", { sessionId }).catch(() => undefined);
    getTerminal()?.focus();
  }

  function sendVncInput(event: VncInputEvent) {
    const sessionId = vncSession.value?.sessionId;
    if (!sessionId) return;
    void rpc("vnc/input", { sessionId, ...event }).catch(() => undefined);
  }

  function sendVncClipboard(text: string) {
    const sessionId = vncSession.value?.sessionId;
    if (!sessionId || !text) return;
    void rpc("vnc/set-clipboard", { sessionId, text }).catch(() => undefined);
    showNotice(t("vnc.clipboardSent"));
  }

  /** 帧通道入口（App.vue handleBinary 的 vnc/frame 分支）：会话隔离校验后
   * 把解码好的 44 字节补丁交给画布，首个桌面帧把状态推到 running。 */
  function handleVncFrame(sessionId: string, payload: Uint8Array) {
    if (!vncSession.value || sessionId !== vncSession.value.sessionId) return;
    if (vncState.value !== "running") vncState.value = "running";
    vncSurface.value?.acceptFrame(payload);
  }

  /** 宿主恢复 tab 时接管活会话（vnc/list 命中）：画布重放当前桌面。 */
  async function reattachVncSession(session: { sessionId: string; host?: string; port?: number }) {
    vncSession.value = { sessionId: session.sessionId, host: session.host || "", port: session.port || 5900 };
    vncState.value = "running";
    await rpc("vnc/replay", { sessionId: session.sessionId });
  }

  // 卸载兜底：画布实例随组件销毁释放，阻断迟到帧对已卸载 surface 的写入
  // （会话 RPC 清理由 workbench/close 的 ownedSessions 契约统一负责）。
  onScopeDispose(() => {
    vncSurface.value = null;
  });

  return {
    vncSession,
    vncDialogOpen,
    vncState,
    vncError,
    vncScaleMode,
    vncSurface,
    isVncMode,
    vncTarget,
    markVncClosed,
    startVncSession,
    closeVncSession,
    sendVncInput,
    sendVncClipboard,
    handleVncFrame,
    reattachVncSession,
  };
}
