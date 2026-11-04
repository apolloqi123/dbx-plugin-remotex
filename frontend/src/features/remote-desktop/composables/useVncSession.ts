import { computed, nextTick, onScopeDispose, ref, watch, type Ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import type { VncConnectOptions } from "../components/VncConnectDialog.vue";
import { matchWindowDesktopSize } from "../lib/desktopAutoSize";
import type { VncInputEvent } from "../lib/vncFrame";
import type { RdpPointerEvent } from "../lib/rdpFrame";
import { useDesktopClipboard } from "./useDesktopClipboard";

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
  /** vnc/pointer（CursorPseudo）：与 RDP 共用服务端光标形状。 */
  applyPointer(pointer: RdpPointerEvent): void;
  /** 剪贴板同步完成后向远端补发 Ctrl+V（宿主粘贴一次即生效）。 */
  sendPasteShortcut?(): void;
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
  /** 最近一次连接参数：仅存活于本次 webview 内存，用于退出覆盖层的重连。 */
  const lastVncOptions = ref<VncConnectOptions | null>(null);
  /** 远端当前桌面尺寸（帧头回读）：自动匹配窗口据此去重，工具栏显示当前档位。 */
  const vncDesktopSize = ref<{ width: number; height: number } | null>(null);
  /** 分辨率菜单 / 自动匹配开关（会话内有效；VNC 桌面属于服务端共享状态，不持久化）。 */
  const VNC_RESOLUTION_PRESETS: ReadonlyArray<{ width: number; height: number }> = [
    { width: 1280, height: 800 },
    { width: 1600, height: 900 },
    { width: 1920, height: 1080 },
    { width: 2560, height: 1440 },
  ];
  const vncAutoFit = ref(true);
  const vncResizing = ref(false);
  const vncResolutionOpen = ref(false);
  const vncCustomWidth = ref("");
  const vncCustomHeight = ref("");
  let vncReconnectInFlight = false;
  let autoResizeTimer: ReturnType<typeof setTimeout> | undefined;
  let autoResizeInFlight = false;
  let pendingAutoSize: { width: number; height: number } | null = null;
  let vncDropUnsubscribe: (() => void) | null = null;
  let vncDropNoticeAt = 0;
  const vncClipboard = useDesktopClipboard({
    t,
    showError,
    showNotice,
    sessionId: computed(() => vncSession.value?.sessionId ?? null),
    active: () => vncState.value === "running" && vncSession.value !== null,
    setRemoteText: async (sessionId, text) => {
      await rpc("vnc/set-clipboard", { sessionId, text });
    },
    sendPasteShortcut: () => vncSurface.value?.sendPasteShortcut?.(),
    receivedNoticeKey: "vnc.clipboardReceived",
    accessDeniedNoticeKey: "vnc.clipboardAccessDenied",
  });
  const isVncMode = computed(() => vncSession.value !== null);
  const vncTarget = computed(() => (vncSession.value ? `${vncSession.value.host}:${vncSession.value.port}` : ""));

  function clearAutoResizeTimer() {
    if (autoResizeTimer !== undefined) {
      clearTimeout(autoResizeTimer);
      autoResizeTimer = undefined;
    }
  }

  /** 自动匹配窗口：请求 RFB SetDesktopSize，由服务端确认后重绘。
   * 仅 fit 缩放生效；失败/不支持时静默保留原分辨率（前端仍按 fit 缩放）。 */
  async function applyVncAutoSize(width: number, height: number) {
    const sessionId = vncSession.value?.sessionId;
    if (!vncAutoFit.value || !sessionId || vncScaleMode.value !== "fit" || vncState.value !== "running") return;
    if (autoResizeInFlight) {
      // Keep only the newest target while the previous request is in flight.
      pendingAutoSize = { width, height };
      return;
    }
    const target = matchWindowDesktopSize(width, height);
    const current = vncDesktopSize.value;
    if (current && Math.abs(current.width - target.width) <= 8 && Math.abs(current.height - target.height) <= 8) return;
    autoResizeInFlight = true;
    try {
      await rpc("vnc/resize", { sessionId, width: target.width, height: target.height });
    } catch {
      // Automatic renegotiation is best-effort; keep the current desktop.
    } finally {
      autoResizeInFlight = false;
      const next = pendingAutoSize;
      pendingAutoSize = null;
      if (next) void applyVncAutoSize(next.width, next.height);
    }
  }

  function scheduleVncAutoSize(cssWidth: number, cssHeight: number) {
    if (!vncAutoFit.value || vncScaleMode.value !== "fit") return;
    clearAutoResizeTimer();
    const target = matchWindowDesktopSize(cssWidth, cssHeight);
    autoResizeTimer = setTimeout(() => {
      autoResizeTimer = undefined;
      void applyVncAutoSize(target.width, target.height);
    }, 400);
  }

  /** VncSurface 画布尺寸变化（侧栏开合/窗口缩放）。 */
  function onVncSurfaceResize(size: { width: number; height: number }) {
    scheduleVncAutoSize(size.width, size.height);
  }

  /** RFB has no file clipboard: surface a clear notice instead of silently
   *  swallowing an OS drop while a VNC desktop is active. */
  function startVncDropGuard() {
    if (vncDropUnsubscribe) return;
    const transfer = (window as unknown as {
      dbxPlugin?: { fileTransfer?: { onDrop?: (listener: (files: unknown[]) => void) => () => void } };
    }).dbxPlugin?.fileTransfer;
    if (!transfer?.onDrop) return;
    const unsubscribe = transfer.onDrop((files) => {
      if (!files.length || vncState.value !== "running") return;
      if (Date.now() - vncDropNoticeAt <= 8000) return;
      vncDropNoticeAt = Date.now();
      showNotice(t("vnc.clipboardFilesUnsupported"));
    });
    vncDropUnsubscribe = typeof unsubscribe === "function" ? unsubscribe : null;
  }

  function stopVncDropGuard() {
    const unsubscribe = vncDropUnsubscribe;
    vncDropUnsubscribe = null;
    unsubscribe?.();
  }

  function vncResolutionLabel(width: number, height: number): string {
    return `${width}×${height}`;
  }

  function isVncResolutionActive(width: number, height: number): boolean {
    return vncDesktopSize.value?.width === width && vncDesktopSize.value?.height === height;
  }

  const vncCanResize = computed(() => vncSession.value !== null && vncState.value === "running");

  /** 手动分辨率：关闭自动匹配，直接向服务器请求 SetDesktopSize。 */
  async function applyVncResolution(width: number, height: number) {
    const sessionId = vncSession.value?.sessionId;
    if (!sessionId || vncResizing.value || vncState.value !== "running") return;
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 640 || width > 3840 || height < 480 || height > 2160) {
      showError(new Error(t("vnc.sizeError")), "terminal");
      return;
    }
    vncAutoFit.value = false;
    clearAutoResizeTimer();
    vncResizing.value = true;
    try {
      await rpc("vnc/resize", { sessionId, width, height });
      vncResolutionOpen.value = false;
    } catch (cause) {
      showError(cause, "terminal");
    } finally {
      vncResizing.value = false;
    }
  }

  async function applyVncCustomResolution() {
    const width = Number.parseInt(vncCustomWidth.value, 10);
    const height = Number.parseInt(vncCustomHeight.value, 10);
    await applyVncResolution(width, height);
  }

  /** “匹配窗口”：恢复自动模式，并立即按当前画布协商一次。 */
  function enableVncAutoFit() {
    vncAutoFit.value = true;
    vncScaleMode.value = "fit";
    vncResolutionOpen.value = false;
    clearAutoResizeTimer();
    const rect = (vncSurface.value?.$el as HTMLElement | undefined)?.getBoundingClientRect();
    if (rect && rect.width > 0 && rect.height > 0) {
      scheduleVncAutoSize(rect.width, rect.height);
    }
  }

  // 首次画面到达后按画布尺寸协商一次远端分辨率（服务端支持时）。
  watch(
    () => [vncSession.value?.sessionId ?? null, vncState.value] as const,
    ([sessionId, state], previous) => {
      if (!sessionId || state !== "running") {
        vncClipboard.stop();
        stopVncDropGuard();
        return;
      }
      if (previous?.[0] !== sessionId) {
        vncClipboard.reset();
        vncDropNoticeAt = 0;
      }
      vncClipboard.start();
      void vncClipboard.syncNow();
      startVncDropGuard();
      const rect = (vncSurface.value?.$el as HTMLElement | undefined)?.getBoundingClientRect();
      if (rect && rect.width > 0 && rect.height > 0) scheduleVncAutoSize(rect.width, rect.height);
    },
    { immediate: true },
  );

  // 退出态统一入口：error 为 null 表示 sidecar 未带原因（主动关闭），
  // 非空时在退出覆盖层展示（认证失败/服务端强制 Tight 等）。
  function markVncClosed(error: string | null) {
    if (!vncSession.value || vncState.value === "closed") return;
    vncState.value = "closed";
    vncResolutionOpen.value = false;
    vncClipboard.stop();
    clearAutoResizeTimer();
    pendingAutoSize = null;
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
      lastVncOptions.value = { ...connectOptions };
      vncState.value = "connecting";
      vncError.value = "";
      vncDesktopSize.value = null;
      vncAutoFit.value = true;
      vncResizing.value = false;
      vncResolutionOpen.value = false;
      pendingAutoSize = null;
      vncClipboard.reset();
      vncClipboard.stop();
      clearAutoResizeTimer();
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
    lastVncOptions.value = null;
    vncDesktopSize.value = null;
    vncAutoFit.value = true;
    vncResizing.value = false;
    vncResolutionOpen.value = false;
    pendingAutoSize = null;
    vncClipboard.reset();
    vncClipboard.stop();
    clearAutoResizeTimer();
    if (!sessionId) return;
    await rpc("vnc/close", { sessionId }).catch(() => undefined);
    getTerminal()?.focus();
  }

  /** 退出覆盖层重连：用最近一次参数重新 vnc/start。webview 重建后没有参数
   * 时回落到连接弹窗（host/port 由弹窗的 last-params 记忆预填）。 */
  async function reconnectVncSession() {
    if (vncReconnectInFlight) return;
    const options = lastVncOptions.value;
    if (!options) {
      vncDialogOpen.value = true;
      return;
    }
    vncReconnectInFlight = true;
    try {
      vncError.value = "";
      await startVncSession(options);
    } finally {
      vncReconnectInFlight = false;
    }
  }

  function sendVncInput(event: VncInputEvent) {
    const sessionId = vncSession.value?.sessionId;
    if (!sessionId) return;
    void rpc("vnc/input", { sessionId, ...event }).catch(() => undefined);
  }

  /** 只写远端剪贴板（工具栏“粘贴到剪贴板”），由用户在远端自行 Ctrl+V。 */
  function sendVncClipboard(text: string): Promise<boolean> {
    return vncClipboard.setRemoteClipboard(text);
  }

  /** Ctrl/Cmd+V（或原生 paste 事件携带的文本）：同步后向远端补发一次 Ctrl+V。 */
  function pasteVncClipboard(nativeText?: string): Promise<boolean> {
    return vncClipboard.pasteToRemote(nativeText);
  }

  /** 远端 ServerCutText → 宿主剪贴板（写入失败会提示并在下次轮询重试）。 */
  function receiveVncClipboard(text: string) {
    void vncClipboard.receiveRemoteText(text);
  }

  /** 帧通道入口（App.vue handleBinary 的 vnc/frame 分支）：会话隔离校验后
   * 把解码好的 44 字节补丁交给画布，首个桌面帧把状态推到 running。 */
  function handleVncFrame(sessionId: string, payload: Uint8Array) {
    if (!vncSession.value || sessionId !== vncSession.value.sessionId) return;
    if (payload.byteLength >= 16) {
      const view = new DataView(payload.buffer, payload.byteOffset, 16);
      vncDesktopSize.value = {
        width: view.getUint32(8, true),
        height: view.getUint32(12, true),
      };
    }
    if (vncState.value !== "running") vncState.value = "running";
    vncSurface.value?.acceptFrame(payload);
  }

  /** 宿主恢复 tab 时接管活会话（vnc/list 命中）：画布重放当前桌面。 */
  async function reattachVncSession(session: { sessionId: string; host?: string; port?: number }) {
    vncClipboard.reset();
    vncSession.value = { sessionId: session.sessionId, host: session.host || "", port: session.port || 5900 };
    vncState.value = "running";
    await rpc("vnc/replay", { sessionId: session.sessionId });
  }

  // 卸载兜底：画布实例随组件销毁释放，阻断迟到帧对已卸载 surface 的写入
  // （会话 RPC 清理由 workbench/close 的 ownedSessions 契约统一负责）。
  onScopeDispose(() => {
    vncSurface.value = null;
    lastVncOptions.value = null;
    vncClipboard.stop();
    stopVncDropGuard();
    clearAutoResizeTimer();
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
    reconnectVncSession,
    sendVncInput,
    sendVncClipboard,
    pasteVncClipboard,
    receiveVncClipboard,
    handleVncFrame,
    reattachVncSession,
    onVncSurfaceResize,
    vncDesktopSize,
    VNC_RESOLUTION_PRESETS,
    vncAutoFit,
    vncResizing,
    vncCanResize,
    vncResolutionOpen,
    vncCustomWidth,
    vncCustomHeight,
    vncResolutionLabel,
    isVncResolutionActive,
    applyVncResolution,
    applyVncCustomResolution,
    enableVncAutoFit,
  };
}
