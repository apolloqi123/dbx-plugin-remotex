import { computed, nextTick, ref, watch, type Ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import type { Terminal } from "@xterm/xterm";
import type { RdpConnectOptions } from "../components/RdpConnectDialog.vue";
import { initialRdpSessionState, rdpCertRemainingSecs, type RdpInputEvent, type RdpSessionStateView } from "../lib/rdpFrame";
import { useDesktopClipboard } from "./useDesktopClipboard";
import { useRdpFileClipboard } from "./useRdpFileClipboard";

/** RDP 证书确认弹窗数据（原 App.vue 局部接口 RdpCertPrompt，随状态收口）。 */
export interface RdpCertPromptView {
  challengeId: string;
  sessionId: string;
  host: string;
  port: number;
  fingerprint: string;
  knownHostStatus: string;
  receivedAt: number;
}

/** RDP 会话生命周期 + 证书确认域：start/close/reconnect/输入与剪贴板回写
 * 四个 RPC 出口、rdp-certificate 挑战的 120s 倒计时弹窗（fail-closed，超时
 * 即拒绝，与 sidecar 两侧语义一致）。rdpSession/rdpState/rdpSurface 等
 * 模板共享态留守 App.vue，经 options 注入；rdpClipboardChunks 因卸载清理
 * 仍在 App.vue，同样透传。terminal/disposed 为每会话可变量，以 getter 取。 */
export function useRdpSession(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  rdpSession: Ref<{ sessionId: string; host: string; port: number } | null>;
  rdpState: Ref<RdpSessionStateView>;
  rdpDialogOpen: Ref<boolean>;
  rdpSurface: Ref<{ reset(): void; sendPasteShortcut?(): void; $el?: any } | null>;
  rdpScaleMode: Ref<RdpConnectOptions["scaleMode"]>;
  rdpClipboardChunks: Map<string, { parts: string[]; received: number }>;
  workbenchId: Ref<string>;
  /** 当前连接 id（可选）：rdp/start 带上它，sidecar 才能把连接表单暂存的
   * 密码/用户名合并进 NLA 认证（webview 拿不到凭据，只能给身份）。 */
  connectionId: Ref<string>;
  isDisposed: () => boolean;
  getTerminal: () => Terminal | undefined;
}) {
  const {
    t,
    showError,
    showNotice,
    rdpSession,
    rdpState,
    rdpDialogOpen,
    rdpSurface,
    rdpScaleMode,
    rdpClipboardChunks,
    workbenchId,
    connectionId,
    isDisposed,
    getTerminal,
  } = options;

  // RDP 证书确认（connection/challenge kind=rdp-certificate，RDP-3 前端）：
  // 复用 host-key 挑战的 kind 分支入口，展示 SHA256 指纹 + 120s 倒计时 +
  // remember 勾选；应答走 rdp/certificate/resolve（fail-closed，超时即拒绝）。
  const rdpCertPrompt = ref<RdpCertPromptView | null>(null);
  const rdpCertRemember = ref(false);
  const rdpCertRemaining = ref(0);
  let rdpCertTimer = 0;
  /** 连接表单的 rdp_clipboard：关闭时既不同步也不接受手动粘贴。 */
  const rdpClipboardEnabled = ref(true);
  /** CLIPRDR 通道状态：unavailable = 服务器策略拒绝剪贴板虚拟通道。 */
  const rdpClipboardStatus = ref<"unknown" | "ready" | "unavailable">("unknown");
  const rdpClipboard = useDesktopClipboard({
    t,
    showError,
    showNotice,
    sessionId: computed(() => rdpSession.value?.sessionId ?? null),
    active: () => rdpState.value.state === "running" && rdpSession.value !== null,
    setRemoteText: async (sessionId, text) => {
      await rpc("rdp/set-clipboard", { sessionId, text });
    },
    sendPasteShortcut: () => rdpSurface.value?.sendPasteShortcut?.(),
    receivedNoticeKey: "rdp.clipboardReceived",
    accessDeniedNoticeKey: "rdp.clipboardAccessDenied",
  });
  const rdpFileClipboard = useRdpFileClipboard({
    t,
    showError,
    showNotice,
    sessionId: computed(() => rdpSession.value?.sessionId ?? null),
    active: () => rdpState.value.state === "running" && rdpSession.value !== null,
    enabled: () => rdpClipboardEnabled.value,
  });

  watch(
    () => [rdpSession.value?.sessionId ?? null, rdpState.value.state] as const,
    ([sessionId, state], previous) => {
      if (!sessionId || state !== "running" || !rdpClipboardEnabled.value) {
        rdpClipboard.stop();
        rdpFileClipboard.stopDropListener();
        return;
      }
      if (previous?.[0] !== sessionId) {
        rdpClipboard.reset();
        rdpFileClipboard.reset();
        rdpFileClipboard.stopDropListener();
      }
      rdpClipboard.start();
      void rdpClipboard.syncNow();
      rdpFileClipboard.startDropListener();
    },
    { immediate: true },
  );

  async function startRdpSession(options: RdpConnectOptions): Promise<boolean> {
    // 重入保护（reconnect 双 start 竞态修复的第二道闸）：该连接已在连接中
    // （connecting）时直接视为成功——reconnect 梯子/宿主 replay 并发触发的
    // 第二次启动不再拆掉在途会话重建（配合 sidecar 同 workbenchId 幂等查重）。
    if (rdpSession.value && rdpState.value.state === "connecting") return true;
    // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
    if (rdpSession.value && rdpState.value.state !== "closed") await closeRdpSession();
    try {
      const info = await rpc<{ sessionId: string; host: string; port: number }>("rdp/start", {
        workbenchId: workbenchId.value,
        connectionId: connectionId.value || undefined,
        host: options.host,
        port: options.port,
        username: options.username,
        width: options.width,
        height: options.height,
        certificatePolicy: options.certificatePolicy,
        clipboard: options.clipboard,
        ...(options.password ? { password: options.password } : {}),
        ...(options.domain ? { domain: options.domain } : {}),
      });
      if (isDisposed()) {
        void rpc("rdp/close", { sessionId: info.sessionId }).catch(() => undefined);
        return false;
      }
      rdpSession.value = { sessionId: info.sessionId, host: info.host, port: info.port };
      rdpScaleMode.value = options.scaleMode;
      rdpClipboardEnabled.value = options.clipboard !== false;
      rdpClipboardStatus.value = "unknown";
      rdpClipboard.syncEnabled.value = rdpClipboardEnabled.value;
      rdpClipboard.reset();
      rdpClipboard.stop();
      rdpFileClipboard.reset();
      rdpFileClipboard.stopDropListener();
      rdpState.value = { state: "connecting", error: "", errorKind: "", attempt: 0, maxAttempts: 0 };
      rdpSurface.value?.reset();
      await nextTick();
      rdpSurface.value?.$el?.querySelector("canvas")?.focus();
      return true;
    } catch (cause) {
      // A saved connection must not vanish behind a blank terminal: surface the
      // failure in the RDP overlay instead of silently leaving the tab idle.
      rdpState.value = {
        state: "closed",
        error: cause instanceof Error ? cause.message : String(cause),
        errorKind: "",
        attempt: 0,
        maxAttempts: 0,
      };
      showError(cause, "terminal");
      return false;
    }
  }

  async function closeRdpSession() {
    const sessionId = rdpSession.value?.sessionId;
    rdpSession.value = null;
    rdpState.value = initialRdpSessionState();
    rdpClipboardStatus.value = "unknown";
    dismissRdpCertPrompt();
    rdpClipboard.reset();
    rdpClipboard.stop();
    rdpFileClipboard.reset();
    rdpFileClipboard.stopDropListener();
    rdpClipboardChunks.delete(sessionId ?? "");
    if (!sessionId) return;
    await rpc("rdp/close", { sessionId }).catch(() => undefined);
    getTerminal()?.focus();
  }

  function sendRdpInput(event: RdpInputEvent) {
    const sessionId = rdpSession.value?.sessionId;
    if (!sessionId) return;
    void rpc("rdp/input", { sessionId, ...event }).catch(() => undefined);
  }

  /** 只写远端剪贴板（工具栏“粘贴到剪贴板”），由用户在远端自行 Ctrl+V。 */
  function sendRdpClipboard(text: string): Promise<boolean> {
    if (!rdpClipboardEnabled.value) return Promise.resolve(false);
    return rdpClipboard.setRemoteClipboard(text);
  }

  /** Ctrl/Cmd+V（或原生 paste 事件携带的文本）：同步后向远端补发一次 Ctrl+V。 */
  function pasteRdpClipboard(nativeText?: string): Promise<boolean> {
    if (!rdpClipboardEnabled.value) return Promise.resolve(false);
    return rdpClipboard.pasteToRemote(nativeText);
  }

  /** 远端 CF_UNICODETEXT → 宿主剪贴板（由 usePluginEvents 分片拼好后调用）。 */
  function receiveRdpClipboard(text: string) {
    void rdpClipboard.receiveRemoteText(text);
  }

  /** CLIPRDR 通道状态（后端 on_ready / 就绪超时）：通道不可用通常是服务器策略
   *  禁用剪贴板重定向，停止后台读取并给出一次可见提示。 */
  function receiveRdpClipboardStatus(params: Record<string, unknown>) {
    const state = typeof params.state === "string" ? params.state : "";
    if (state === "ready") {
      rdpClipboardStatus.value = "ready";
      rdpClipboard.setChannelAvailable(true);
      // GNOME Remote Desktop and other non-Windows servers negotiate text
      // only; the toolbar then points at SSH/SFTP instead of Ctrl+C/V files.
      rdpFileClipboard.setServerCapability(params.fileCopy === true);
      return;
    }
    if (state === "unavailable") {
      rdpFileClipboard.setServerCapability(false);
      if (rdpClipboardStatus.value !== "unavailable") {
        rdpClipboardStatus.value = "unavailable";
        rdpClipboard.setChannelAvailable(false);
        showNotice(t("rdp.clipboardUnavailable"));
      }
    }
  }

  /** 文件剪贴板事件（available/progress/ready/error）。 */
  function receiveRdpClipboardFiles(method: string, params: Record<string, unknown>) {
    rdpFileClipboard.handleEvent(method, params);
  }

  function startRdpRemoteDownload(): Promise<boolean> {
    return rdpFileClipboard.startRemoteDownload();
  }

  function saveRdpRemoteArchive(): Promise<boolean> {
    return rdpFileClipboard.saveRemoteArchive();
  }

  function dismissRdpRemoteFiles(): Promise<void> {
    return rdpFileClipboard.dismissRemoteFiles();
  }

  // 手动重连（graceful disconnect / 终态错误后的出口）：generation 递增由
  // sidecar 负责，前端只触发并让 rdp/session/state 事件驱动状态条。
  async function reconnectRdpSession() {
    const sessionId = rdpSession.value?.sessionId;
    if (!sessionId) {
      // Start failed before a session existed: retry through the prefilled form.
      rdpDialogOpen.value = true;
      return;
    }
    try {
      await rpc("rdp/reconnect", { sessionId });
    } catch (cause) {
      showError(cause, "terminal");
    }
  }

  // —— RDP 证书确认（rdp-certificate challenge）：120s 倒计时 + fail-closed ——
  // 倒计时基于队首 receivedAt + 120s 绝对期限（与 AI 审批弹窗同一 tick 模式），
  // 到 0 仅关弹窗——sidecar 侧超时同样拒绝该挑战，两侧语义一致。
  watch(rdpCertPrompt, (prompt) => {
    if (rdpCertTimer) {
      window.clearInterval(rdpCertTimer);
      rdpCertTimer = 0;
    }
    if (!prompt) {
      rdpCertRemaining.value = 0;
      return;
    }
    const tick = () => {
      const current = rdpCertPrompt.value;
      if (!current) return;
      rdpCertRemaining.value = rdpCertRemainingSecs(current.receivedAt, Date.now());
      if (rdpCertRemaining.value <= 0) dismissRdpCertPrompt();
    };
    tick();
    rdpCertTimer = window.setInterval(tick, 250);
  });

  // 挑战一次性：先出弹窗再 resolve（超时/取消/未知 id 一律按拒绝处理）。
  function dismissRdpCertPrompt() {
    if (rdpCertTimer) {
      window.clearInterval(rdpCertTimer);
      rdpCertTimer = 0;
    }
    rdpCertPrompt.value = null;
    rdpCertRemaining.value = 0;
  }

  async function resolveRdpCertificate(accept: boolean) {
    const prompt = rdpCertPrompt.value;
    if (!prompt) return;
    const remember = accept && rdpCertRemember.value;
    dismissRdpCertPrompt();
    try {
      await rpc("rdp/certificate/resolve", {
        challengeId: prompt.challengeId,
        accept,
        remember,
      });
    } catch (cause) {
      showError(cause, "terminal");
    }
  }

  return {
    rdpCertPrompt,
    rdpCertRemember,
    rdpCertRemaining,
    startRdpSession,
    closeRdpSession,
    sendRdpInput,
    sendRdpClipboard,
    pasteRdpClipboard,
    receiveRdpClipboard,
    receiveRdpClipboardFiles,
    receiveRdpClipboardStatus,
    startRdpRemoteDownload,
    saveRdpRemoteArchive,
    dismissRdpRemoteFiles,
    rdpClipboardEnabled,
    rdpClipboardStatus,
    rdpFileBusy: rdpFileClipboard.busy,
    rdpFileProgress: rdpFileClipboard.progress,
    rdpRemoteFiles: rdpFileClipboard.remoteFiles,
    rdpArchiveReady: rdpFileClipboard.archiveReady,
    rdpRemoteReady: rdpFileClipboard.remoteReady,
    rdpNativeFileClipboard: rdpFileClipboard.nativeSupported,
    rdpServerFileClipboard: rdpFileClipboard.serverFiles,
    rdpServerFileClipboardKnown: rdpFileClipboard.serverFilesKnown,
    rdpFileError: rdpFileClipboard.error,
    reconnectRdpSession,
    resolveRdpCertificate,
  };
}
