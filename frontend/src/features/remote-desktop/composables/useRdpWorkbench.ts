// RDP 会话工作台专属逻辑（P1 隔离收口）：SSH 工具在 RDP 连接上整类隐藏后，
// 远程桌面自己需要的操作面——缩放三档（纯模板态，留守 App.vue）+ 大文本粘贴
// 双通道（写远端剪贴板 / 按键注入）+ 分辨率入口（预设/窗口比例/自定义）。
//
// 依赖单向：本模块只吃注入的 refs/回调（rdpSession/rdpSurface/sendRdpInput/
// sendRdpClipboard/persistState...），不 import App.vue，也不 import 任何
import { rpc } from "../../../shared/lib/rpc";
// SSH 内部符号——App.vue 是唯一消费方。
import { computed, ref, watch, type Ref } from "vue";
import type { RdpConnectOptions } from "../components/RdpConnectDialog.vue";
import { matchWindowDesktopSize } from "../lib/desktopAutoSize";
import { rdpErrorKindKey, type RdpInputEvent, type RdpSessionStateView } from "../lib/rdpFrame";

/** RDP 会话内用户选择的桌面尺寸（持久化到 workbenchState 的 rdpWidth/Height，
 * 恢复 tab 时优先于连接表单的 rdp_resolution——会话内显式选择是更近的意图）。*/
export interface RdpPersistedSize {
  width: number;
  height: number;
}

/** 连接上下文（host/port/rdp_* 直启 RDP 用）。 */
export interface RdpConnectionContext {
  host?: string;
  port?: number;
  username?: string;
  rdpDomain?: string;
  rdpWidth?: number;
  rdpHeight?: number;
  rdpCertificatePolicy?: "prompt" | "strict" | "accept-temporarily";
  rdpClipboard?: boolean;
}

export function useRdpWorkbench(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  persistState: () => void;
  isRdpMode: Ref<boolean>;
  rdpSession: Ref<{ sessionId: string } | null>;
  rdpSurface: Ref<{ $el?: any } | null>;
  sendRdpInput: (event: RdpInputEvent) => void;
  sendRdpClipboard: (text: string) => void;
  rdpState: Ref<RdpSessionStateView>;
  rdpDialogOpen: Ref<boolean>;
  rdpScaleMode: Ref<"fit" | "stretch" | "actual">;
  startRdpSession: (connectOptions: RdpConnectOptions) => Promise<boolean>;
  // 装配依赖（声明/装配在本组合式函数之后）：一律 getter 惰性接入；函数内按
  // 原名解构，迁出代码保持逐行原样。
  getConnection: () => Ref<RdpConnectionContext>;
  getStoredConnectionProtocol: () => Ref<string>;
  sftpPaneOpen: Ref<boolean>;
  closeRightPanel: () => void;
  getBatchBarOpen: () => Ref<boolean>;
}) {
  const {
    t, showError, showNotice, persistState, isRdpMode, rdpSession, rdpSurface, sendRdpInput, sendRdpClipboard,
    rdpState, rdpDialogOpen, rdpScaleMode, startRdpSession,
    getConnection, getStoredConnectionProtocol, sftpPaneOpen, closeRightPanel, getBatchBarOpen,
  } = options;

  // —— 大文本粘贴（与连接表单同一组文案）：写入远端剪贴板（快，远端 Ctrl+V）
  // 或按键注入（慢，剪贴板被禁时兜底，4k 字符/次为后端上限，分片发送）。——
  const rdpPasteOpen = ref(false);
  const rdpPasteText = ref("");
  const rdpTypingBusy = ref(false);
  const rdpTypingSent = ref(0);
  const rdpTypingTotal = ref(0);
  // 文件传输提示弹层：RDP 直接走 CLIPRDR 文件通道，按钮只解释剪贴板用法，
  // 不再打开 VNC 的 SSH/SFTP 双栏窗口。
  const rdpFileHintOpen = ref(false);

  function rdpWriteRemoteClipboard() {
    const text = rdpPasteText.value;
    if (!text) return;
    sendRdpClipboard(text);
  }

  async function rdpTypeAsKeystrokes() {
    const text = rdpPasteText.value;
    if (!text || rdpTypingBusy.value) return;
    rdpTypingBusy.value = true;
    rdpTypingSent.value = 0;
    rdpTypingTotal.value = text.length;
    const chunkSize = 512;
    try {
      for (let offset = 0; offset < text.length; offset += chunkSize) {
        if (!isRdpMode.value) break;
        sendRdpInput({ kind: "unicode", text: text.slice(offset, offset + chunkSize) });
        rdpTypingSent.value = Math.min(offset + chunkSize, text.length);
        await new Promise((resolve) => setTimeout(resolve, 120));
      }
    } finally {
      rdpTypingBusy.value = false;
    }
  }

  // —— 分辨率入口（P0-2）：预设 + 自定义宽高，走 rdp/resize。后端把新尺寸
  // 排队给当前引擎（MS-RDPEDISP 显示器布局 + 服务端 Deactivate-Reactivation），
  // 会话内生效，不再经历 connecting 覆盖层；仅无引擎（拨号/退避）时留待下代。
  // 选择持久化进 workbenchState（rdpWidth/rdpHeight），恢复 tab 时优先于
  // 连接表单。 ——
  const RDP_RESOLUTION_PRESETS: ReadonlyArray<{ width: number; height: number }> = [
    { width: 1280, height: 800 },
    { width: 1600, height: 900 },
    { width: 1920, height: 1080 },
    { width: 2560, height: 1440 },
  ];

  /** Remote resolution for "match window": the pane's CSS size (not its
   * physical pixels — a DPR-sized desktop made Windows text far too small on
   * Retina panes), aspect preserved, inside the comfortable display range. */
  function targetRdpSize(cssWidth: number, cssHeight: number): { width: number; height: number } {
    return matchWindowDesktopSize(cssWidth, cssHeight);
  }

  function autoRdpDesktopSize(): { width: number; height: number } {
    if (typeof window === "undefined") return { width: 1920, height: 1080 };
    const heap =
      document.querySelector<HTMLElement>(".terminal-workspace .terminal-pane") ??
      document.querySelector<HTMLElement>(".terminal-workspace .terminal-layout-pane") ??
      document.querySelector<HTMLElement>(".terminal-workspace");
    const rect = heap?.getBoundingClientRect();
    const cssWidth = rect && rect.width > 0 ? rect.width : window.innerWidth || 1440;
    const cssHeight = rect && rect.height > 0 ? rect.height : Math.max(480, (window.innerHeight || 900) - 96);
    return targetRdpSize(cssWidth, cssHeight);
  }

  const rdpResolutionOpen = ref(false);
  const rdpCustomWidth = ref("");
  const rdpCustomHeight = ref("");
  const rdpResizing = ref(false);
  const rdpDesktopSize = ref<{ width: number; height: number } | null>(null);
  const rdpPersistedSize = ref<RdpPersistedSize | null>(null);
  /** 自动匹配画布：默认开启，用户手选固定分辨率后关闭。 */
  const rdpAutoFit = ref(true);
  let autoResizeTimer: ReturnType<typeof setTimeout> | undefined;

  function clearAutoResizeTimer() {
    if (autoResizeTimer !== undefined) {
      clearTimeout(autoResizeTimer);
      autoResizeTimer = undefined;
    }
  }

  /** Sidebar/window changes resize the pane; renegotiate the remote desktop
   * after a short debounce so fit mode keeps filling without letterboxing.
   * The backend applies this live (no reconnect), so the canvas never drops
   * into the connecting overlay. */
  async function applyAutoRdpSize(width: number, height: number) {
    const sessionId = rdpSession.value?.sessionId;
    if (!rdpAutoFit.value || rdpScaleMode.value !== "fit" || !sessionId || rdpResizing.value || rdpState.value.state !== "running") return;
    const current = rdpDesktopSize.value;
    if (current && Math.abs(current.width - width) <= 8 && Math.abs(current.height - height) <= 8) return;
    rdpResizing.value = true;
    try {
      await rpc("rdp/resize", { sessionId, width, height });
    } catch (cause) {
      showError(cause, "terminal");
    } finally {
      rdpResizing.value = false;
    }
  }

  function scheduleAutoRdpSize(cssWidth: number, cssHeight: number) {
    if (!rdpAutoFit.value) return;
    clearAutoResizeTimer();
    const target = targetRdpSize(cssWidth, cssHeight);
    autoResizeTimer = setTimeout(() => {
      autoResizeTimer = undefined;
      void applyAutoRdpSize(target.width, target.height);
    }, 400);
  }

  function onRdpSurfaceResize(size: { width: number; height: number }) {
    scheduleAutoRdpSize(size.width, size.height);
  }

  function enableRdpAutoFit() {
    rdpAutoFit.value = true;
    rdpScaleMode.value = "fit";
    // Drop the manual size from workbench state so reopening stays automatic.
    rdpPersistedSize.value = null;
    persistState();
    const element = rdpSurface.value?.$el as HTMLElement | undefined;
    const rect = element?.getBoundingClientRect();
    if (rect && rect.width > 0 && rect.height > 0) {
      const target = targetRdpSize(rect.width, rect.height);
      clearAutoResizeTimer();
      void applyAutoRdpSize(target.width, target.height);
    }
  }

  // A sidebar/window change during connecting is not resizable yet; re-check
  // the pane once the session reaches running so the first frame already fits.
  watch(
    () => rdpState.value.state,
    (state) => {
      if (state !== "running") return;
      const element = rdpSurface.value?.$el as HTMLElement | undefined;
      const rect = element?.getBoundingClientRect();
      if (rect && rect.width > 0 && rect.height > 0) scheduleAutoRdpSize(rect.width, rect.height);
    },
  );

  /** Fallback dialog seed: saved connection values win, persisted session size
   * overrides the form resolution like the direct-start path. */
  const rdpDialogSeed = computed<RdpConnectOptions | null>(() => {
    const conn = getConnection().value;
    if (!conn.host) return null;
    const auto = autoRdpDesktopSize();
    return {
      host: conn.host,
      port: conn.port && conn.port > 0 ? conn.port : 3389,
      username: conn.username || "",
      ...(conn.rdpDomain ? { domain: conn.rdpDomain } : {}),
      width: rdpPersistedSize.value?.width ?? conn.rdpWidth ?? auto.width,
      height: rdpPersistedSize.value?.height ?? conn.rdpHeight ?? auto.height,
      certificatePolicy: conn.rdpCertificatePolicy ?? "prompt",
      clipboard: conn.rdpClipboard ?? true,
      scaleMode: "fit",
    };
  });

  function rdpResolutionLabel(width: number, height: number): string {
    return `${width}×${height}`;
  }

  function isRdpResolutionActive(width: number, height: number): boolean {
    return rdpDesktopSize.value?.width === width && rdpDesktopSize.value?.height === height;
  }

  /** “窗口比例”预设：按画布容器当前可视区宽高比生成偶数宽高（高度上限 1440，
   * 再夹进 640x480..3840x2160 契约门限），保持与视口一致的宽高比。 */
  function rdpWindowProportionPreset(): { width: number; height: number } {
    const holder = rdpSurface.value?.$el as HTMLElement | undefined;
    const rect = holder?.getBoundingClientRect();
    const ratio = rect && rect.height > 0 ? rect.width / rect.height : 16 / 9;
    const height = Math.min(1440, Math.max(720, Math.round(1440 / ratio / 2) * 2));
    const width = Math.max(640, Math.min(3840, Math.round((height * ratio) / 2) * 2));
    return { width, height: Math.max(480, Math.min(2160, height)) };
  }

  async function applyRdpResolution(width: number, height: number) {
    const sessionId = rdpSession.value?.sessionId;
    if (!sessionId || rdpResizing.value) return;
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 640 || width > 3840 || height < 480 || height > 2160) {
      showError(new Error(t("rdp.sizeError")), "terminal");
      return;
    }
    // Manual choice wins over the pane matcher until the user re-enables it.
    rdpAutoFit.value = false;
    clearAutoResizeTimer();
    rdpResizing.value = true;
    try {
      await rpc("rdp/resize", { sessionId, width, height });
      rdpPersistedSize.value = { width, height };
      persistState();
      showNotice(t("rdp.resolution.applied", { width, height }));
      rdpResolutionOpen.value = false;
    } catch (cause) {
      showError(cause, "terminal");
    } finally {
      rdpResizing.value = false;
    }
  }

  async function applyRdpCustomResolution() {
    const width = Number.parseInt(rdpCustomWidth.value, 10);
    const height = Number.parseInt(rdpCustomHeight.value, 10);
    await applyRdpResolution(width, height);
  }

// RDP 路由重入保护（与 sidecar 同 workbenchId 幂等查重双保险）：reconnect
// 梯子/宿主 replay 并发触发的 openSession 会同时抵达 RDP 路由，两次 rdp/start
// 会让 Windows 单会话策略互踢会话 → 前端持有的旧 sessionId 收到断开事件 →
// 重连梯子再触发，工作台卡死在"连接中"。启动期间置位吞掉并发重入。
let rdpRouteInFlight = false;
async function routeRdpConnection(): Promise<void> {
  if (rdpRouteInFlight) return;
  rdpRouteInFlight = true;
  try {
    const conn = getConnection().value;
    // A saved RDP connection already carries host/username: connect directly.
    // Only missing required fields fall back to the (prefilled) dialog; a
    // start failure must stay in the RDP error overlay instead of nagging.
    if (!conn.host || !conn.username?.trim()) {
      rdpDialogOpen.value = true;
      return;
    }
    await startRdpFromConnection();
  } finally {
    rdpRouteInFlight = false;
  }
}

// 对话框发起的 RDP（回落/手动路径）：不经 openSession 的协议查询，表单协议
// 无从判定——发起前把 tab 级协议置为 rdp，会话断开/关闭后连接级门控仍成立。
async function connectViaRdpDialog(options: RdpConnectOptions): Promise<boolean> {
  const storedConnectionProtocol = getStoredConnectionProtocol();
  storedConnectionProtocol.value = "rdp";
  return startRdpSession(options);
}

// —— RDP 会话生命周期（nyaterm-parity P3-4，与 VNC 同款互斥展示）——
// 画面走 RdpSurface 画布；帧/输入/剪贴板/指针各走独立通道，断线重连由
// sidecar 退避梯子驱动（reconnecting 态展示进度），graceful close/终态错误
// 落退出覆盖层（带 errorKind 友好文案 + 手动 rdp/reconnect 出口）。

/** 退出覆盖层主文案：errorKind 友好化（raw error 作细节行展示）。 */
const rdpClosedTitle = computed(() => {
  const kind = rdpErrorKindKey(rdpState.value.errorKind);
  return kind ? t(`rdp.error.${kind}`) : t("rdp.closed");
});

// M32-B：RDP 连接路由——host/port 走连接绑定字段，domain/分辨率/证书策略/
// 剪贴板走 rdp_* config 字段；凭据（密码）不在连接配置内，由 RDP 会话自身的
// NLA/证书提示流兜底。失败回落 RdpConnectDialog（上次参数预填）。
async function startRdpFromConnection(): Promise<boolean> {
  const connection = getConnection();
  const batchBarOpen = getBatchBarOpen();
  const conn = connection.value;
  if (!conn.host) return false;
  // RDP 连接不渲染 SFTP 面板/分栏/批量发送栏：若同 tab 之前是 SSH 会话残留
  // 的开启态，进入 RDP 会话前收掉，避免隐藏门控下的幽灵分栏占位。
  if (sftpPaneOpen.value) closeRightPanel();
  if (batchBarOpen.value) batchBarOpen.value = false;
  const persisted = rdpPersistedSize.value;
  // Card defaults ("auto" stored as no width/height), 1280×800 and 1920×1080
  // are treated as "not explicitly chosen": use the pane's CSS size.
  const legacyDefault = !conn.rdpWidth || !conn.rdpHeight
    || (conn.rdpWidth === 1280 && conn.rdpHeight === 800)
    || (conn.rdpWidth === 1920 && conn.rdpHeight === 1080);
  const auto = persisted ? null : legacyDefault ? autoRdpDesktopSize() : null;
  rdpAutoFit.value = Boolean(auto);
  clearAutoResizeTimer();
  return startRdpSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 3389,
    username: conn.username || "",
    // 尺寸优先级（P0-2）：workbenchState 里用户会话内选择的尺寸 > 连接表单
    // rdp_resolution > 窗口物理尺寸。
    width: persisted?.width ?? auto?.width ?? conn.rdpWidth ?? 1280,
    height: persisted?.height ?? auto?.height ?? conn.rdpHeight ?? 800,
    certificatePolicy: conn.rdpCertificatePolicy ?? "prompt",
    clipboard: conn.rdpClipboard ?? true,
    scaleMode: "fit",
    ...(conn.rdpDomain ? { domain: conn.rdpDomain } : {}),
  });
}

  return {
    rdpPasteOpen,
    rdpPasteText,
    rdpTypingBusy,
    rdpTypingSent,
    rdpTypingTotal,
    rdpFileHintOpen,
    rdpWriteRemoteClipboard,
    rdpTypeAsKeystrokes,
    RDP_RESOLUTION_PRESETS,
    rdpResolutionOpen,
    rdpCustomWidth,
    rdpCustomHeight,
    rdpResizing,
    rdpDesktopSize,
    rdpPersistedSize,
    rdpAutoFit,
    enableRdpAutoFit,
    onRdpSurfaceResize,
    rdpResolutionLabel,
    isRdpResolutionActive,
    rdpWindowProportionPreset,
    applyRdpResolution,
    applyRdpCustomResolution,
    routeRdpConnection,
    connectViaRdpDialog,
    startRdpFromConnection,
    rdpDialogSeed,
    rdpClosedTitle,
  };
}
