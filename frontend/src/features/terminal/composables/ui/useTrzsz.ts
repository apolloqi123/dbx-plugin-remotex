import { computed, onBeforeUnmount, ref, type Ref } from "vue";
import { TrzszFilter } from "trzsz";
import {
  canStartTrzszTransfer,
  initialTrzszProgressState,
  installTrzszHandlers,
  isTrzszStopMessage,
  reduceTrzszProgress,
  trzszProgressPercent,
  type TrzszAnnounce,
  type TrzszDownloadFile,
  type TrzszProgressEvent,
  type TrzszProgressState,
} from "../../lib/transfer/terminalTrzsz";
import { sampleTransferSpeed, type TransferSpeedSample } from "../../../transfers/lib/transferSpeed";
import { standaloneArrayBuffer } from "../../../transfers/lib/standaloneBuffer";
import { rpc, rpcAvailable } from "../../../../shared/lib/rpc";

/** trzsz (trz / tsz)：官方 trzsz.js TrzszFilter 常驻下行流，announce 自动接管。
 * 传输的协议协商/收发全在 filter 内，插件只负责：选文件（浏览器 File API）、
 * 下载落盘（宿主 fileTransfer 优先、浏览器 <a download> 兜底）、进度 overlay。
 * 终端数据流集成点（dispatchTerminalOutput / 输入路由 / fit 列宽）仍在 App.vue。 */
export function useTrzsz(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  canWrite: Ref<boolean>;
  zmodemBusy: Ref<boolean>;
  terminalMenuOpen: Ref<boolean>;
  terminal: () => { cols: number; focus: () => void } | undefined;
  writeTerminalOutput: (data: Uint8Array) => void;
  sendTerminalBytes: (data: Uint8Array) => void;
  probeLocalCapabilities: () => Promise<{ canSaveLocal: boolean; downloadsDir: string } | undefined>;
  saveHostFile: (chunks: Uint8Array[], fileName: string) => Promise<void>;
  loadDownloadUseDefaultDir: () => boolean;
  askDownloadTarget: (fileName: string) => Promise<{ dir: string; setDefault: boolean } | undefined>;
  loadDownloadDir: () => string;
  resolveDownloadConflictFor: (dir: string, fileName: string) => Promise<"rename" | "overwrite" | undefined>;
  applyChosenDirAsDefault: (dir: string) => void;
  revealTransferTarget: (path: string) => Promise<void>;
}) {
  const { t, showNotice, showError, session, canWrite, zmodemBusy, terminalMenuOpen, terminal: terminalGet, writeTerminalOutput, sendTerminalBytes, probeLocalCapabilities, saveHostFile, loadDownloadUseDefaultDir, askDownloadTarget, loadDownloadDir, resolveDownloadConflictFor, applyChosenDirAsDefault, revealTransferTarget } = options;

const trzszInput = ref<HTMLInputElement>();

// trzsz (trz/tsz)：进度 overlay 状态镜像（真实状态机在 lib/transfer/terminalTrzsz.ts）。
const trzszPhase = ref<TrzszProgressState["phase"]>("idle");
const trzszDirection = ref<TrzszProgressState["direction"]>("");
const trzszFileName = ref("");
const trzszFileIndex = ref(0);
const trzszFileCount = ref(0);
const trzszPercent = ref(0);
const trzszSpeed = ref(0);
const trzszMessage = ref("");

// trzsz：filter 常驻（与 zmodem sentry 同一条下行流），进度状态机与速度采样。
let trzszFilter: TrzszFilter | null = null;
let trzszProgress: TrzszProgressState = initialTrzszProgressState();
let trzszSpeedSample: TransferSpeedSample | undefined;
let trzszDetectionTimer = 0;
let trzszWatchdogTimer = 0;
let trzszOverlayTimer = 0;
let trzszPickResolver: ((files: File[] | undefined) => void) | undefined;
// 选文件兜底路径（WKWebView/旧内核不派发 cancel）挂的 focus 监听与其内层
// 800ms 定时器：结算/重开/卸载时必须成对摘除，否则残留监听会把之后某次
// 窗口聚焦误判成"本次未取消的上传被取消"。
let trzszPickFocusHandler: (() => void) | undefined;
let trzszPickFocusTimer = 0;

const trzszBusy = computed(() => trzszPhase.value === "waiting" || trzszPhase.value === "transferring");
const trzszOverlayVisible = computed(() => trzszPhase.value !== "idle");

const TRZSZ_DETECTION_TIMEOUT_MS = 5000;
const TRZSZ_WATCHDOG_TIMEOUT_MS = 15000;
const TRZSZ_SUCCESS_OVERLAY_MS = 2500;

/** Lazily wires the filter onto the terminal streams (keyboard input + output). */
function ensureTrzszFilter(): TrzszFilter {
  if (trzszFilter) return trzszFilter;
  const filter = new TrzszFilter({
    writeToTerminal: (output) => {
      if (typeof output === "string") writeTerminalOutput(new TextEncoder().encode(output));
      else if (output instanceof Uint8Array) writeTerminalOutput(output);
      else if (output instanceof ArrayBuffer) writeTerminalOutput(new Uint8Array(output));
    },
    // sendToServer 必须走现有 PTY 输入路径（8 字节 BE 序号前缀在 sendTerminalBytes 内封装）。
    sendToServer: (input) => sendTerminalBytes(typeof input === "string" ? new TextEncoder().encode(input) : Uint8Array.from(input)),
    terminalColumns: terminalGet()?.cols || 80,
  });
  installTrzszHandlers(filter, {
    pickUploadFiles: pickTrzszUploadFiles,
    saveDownloadedFiles: saveTrzszDownloadedFiles,
    emit: applyTrzszEvent,
  });
  trzszFilter = filter;
  return filter;
}

function handleTrzszAnnounce(announce: TrzszAnnounce) {
  // Announce 已到：无论等待态由谁进入（菜单触发或远端自行 trz/tsz），
  // 「等待远端响应」的检测定时器使命完成，必须先解除再判断占用。
  window.clearTimeout(trzszDetectionTimer);
  trzszDetectionTimer = 0;
  if (!canStartTrzszTransfer({ zmodemBusy: zmodemBusy.value, trzszBusy: trzszBusy.value })) return;
  // 看门狗：announce 后 filter 一直未发起传输（如去重拦截等边缘）时不让
  // waiting 态永久占用终端输入；filter 打开选文件框时即视为已接管并解除。
  window.clearTimeout(trzszWatchdogTimer);
  trzszWatchdogTimer = window.setTimeout(() => {
    trzszWatchdogTimer = 0;
    if (trzszPhase.value === "waiting") applyTrzszEvent({ type: "reset" });
  }, TRZSZ_WATCHDOG_TIMEOUT_MS);
  applyTrzszEvent({ type: "waiting", direction: announce.direction });
}

function applyTrzszEvent(event: TrzszProgressEvent) {
  trzszProgress = reduceTrzszProgress(trzszProgress, event);
  const state = trzszProgress;
  trzszPhase.value = state.phase;
  trzszDirection.value = state.direction;
  trzszFileName.value = state.fileName;
  trzszFileIndex.value = state.fileIndex;
  trzszFileCount.value = state.fileCount;
  trzszMessage.value = state.message;
  trzszPercent.value = trzszProgressPercent(state);
  if (event.type === "step") {
    trzszSpeedSample = sampleTransferSpeed(trzszSpeedSample, state.totalTransferred, performance.now());
    trzszSpeed.value = trzszSpeedSample.speed;
  } else {
    trzszSpeedSample = undefined;
    trzszSpeed.value = 0;
  }
  switch (event.type) {
    case "success":
      showNotice(t("trzszComplete", { count: Math.max(1, state.fileCount) }));
      window.clearTimeout(trzszOverlayTimer);
      // 成功态短暂可见后自动收起（失败态常驻，直到下一次传输或会话切换）。
      trzszOverlayTimer = window.setTimeout(resetTrzszOverlay, TRZSZ_SUCCESS_OVERLAY_MS);
      break;
    case "failure":
      window.clearTimeout(trzszOverlayTimer);
      // Ctrl+C 主动停止是用户意图，按提示呈现而非错误横幅。
      if (isTrzszStopMessage(event.message)) {
        resetTrzszOverlay();
        showNotice(t("trzszCancelled"));
      } else {
        showError(new Error(t("trzszFailed", { error: event.message })), "terminal");
      }
      break;
    case "cancelled":
      resetTrzszOverlay();
      break;
  }
}

function resetTrzszOverlay() {
  window.clearTimeout(trzszOverlayTimer);
  trzszOverlayTimer = 0;
  applyTrzszEvent({ type: "reset" });
}

/** Ctrl+C 等价：让 filter 停掉当前传输（协议侧走 stop/清理，随后报 cancelled）。 */
function cancelTrzszTransfer() {
  trzszFilter?.stopTransferringFiles();
}

/**
 * 会话切换 / 关闭时的静默收尾：停掉在途传输并复位 overlay，避免 busy 态
 * 卡住终端输入（与 cancelZmodemUpload 同语义）。
 */
function teardownTrzsz() {
  window.clearTimeout(trzszDetectionTimer);
  trzszDetectionTimer = 0;
  window.clearTimeout(trzszWatchdogTimer);
  trzszWatchdogTimer = 0;
  window.clearTimeout(trzszOverlayTimer);
  trzszOverlayTimer = 0;
  // 会话切换/关闭：在途选文件按取消结算，兜底监听与定时器一并摘除。
  finishTrzszPick(undefined);
  trzszFilter?.stopTransferringFiles();
  trzszProgress = initialTrzszProgressState();
  trzszSpeedSample = undefined;
  trzszPhase.value = "idle";
  trzszDirection.value = "";
  trzszFileName.value = "";
  trzszFileIndex.value = 0;
  trzszFileCount.value = 0;
  trzszPercent.value = 0;
  trzszSpeed.value = 0;
  trzszMessage.value = "";
}

const trzszStatusLabel = computed(() => {
  const name = trzszFileName.value;
  const percent = trzszPercent.value;
  switch (trzszPhase.value) {
    case "waiting":
      return t("trzszWaiting");
    case "transferring":
      return trzszDirection.value === "download" ? t("trzszDownloading", { name, percent }) : t("trzszUploading", { name, percent });
    case "success":
      return t("trzszComplete", { count: Math.max(1, trzszFileCount.value) });
    case "failed":
      return t("trzszFailed", { error: trzszMessage.value });
    default:
      return "";
  }
});

/** 右键菜单「Upload (trz)」：向 PTY 发送 trz 触发远端，announce 回来后接管。 */
function chooseTrzszUpload() {
  terminalMenuOpen.value = false;
  if (!session.value || !canWrite.value || !canStartTrzszTransfer({ zmodemBusy: zmodemBusy.value, trzszBusy: trzszBusy.value })) return;
  applyTrzszEvent({ type: "waiting", direction: "upload" });
  trzszDetectionTimer = window.setTimeout(() => {
    if (trzszPhase.value === "waiting") applyTrzszEvent({ type: "failure", message: t("trzszNotAvailable") });
  }, TRZSZ_DETECTION_TIMEOUT_MS);
  sendTerminalBytes(new TextEncoder().encode("trz\r"));
  terminalGet()?.focus();
}

/** 摘除选文件兜底路径的 focus 监听与内层定时器（结算/重开/卸载共用）。 */
function clearTrzszPickFocusGuard() {
  if (trzszPickFocusHandler) {
    window.removeEventListener("focus", trzszPickFocusHandler);
    trzszPickFocusHandler = undefined;
  }
  window.clearTimeout(trzszPickFocusTimer);
  trzszPickFocusTimer = 0;
}

/** filter 回调：浏览器 File API 多选（宿主沙箱内不可用 File System Access API）。 */
function pickTrzszUploadFiles(_directory: boolean): Promise<File[] | undefined> {
  // filter 已接管（走到选文件这一步），等待态看门狗使命完成。
  window.clearTimeout(trzszWatchdogTimer);
  trzszWatchdogTimer = 0;
  // 上一次未完成的选文件请求按取消处理，避免悬挂的 resolver。
  const previous = trzszPickResolver;
  trzszPickResolver = undefined;
  clearTrzszPickFocusGuard();
  previous?.(undefined);
  return new Promise((resolve) => {
    trzszPickResolver = resolve;
    // WKWebView/旧内核不派发 input 的 cancel 事件：窗口重新拿到焦点后一小段
    // 时间内 change 仍未触发（本次 resolver 还挂着）即视为用户取消。定时器
    // 只取消唤起它的那一次请求，不读全局 resolver——残留监听可能晚于新上传
    // 才触发，读全局会把新上传误判为取消。
    const onFocus = () => {
      trzszPickFocusTimer = window.setTimeout(() => {
        trzszPickFocusTimer = 0;
        if (trzszPickResolver === resolve) onTrzszPickCancel();
      }, 800);
    };
    trzszPickFocusHandler = onFocus;
    window.addEventListener("focus", onFocus, { once: true });
    trzszInput.value?.click();
  });
}

function onTrzszPickInput(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files || []);
  input.value = "";
  finishTrzszPick(files.length ? files : undefined);
}

function onTrzszPickCancel() {
  finishTrzszPick(undefined);
}

/** 结算本次选文件：清 resolver 与兜底监听/定时器，再 resolve 发起时捕获的 Promise。 */
function finishTrzszPick(files: File[] | undefined) {
  const resolve = trzszPickResolver;
  trzszPickResolver = undefined;
  clearTrzszPickFocusGuard();
  resolve?.(files);
}

/**
 * 下载落盘：优先宿主 fileTransfer API（optional 1.1 特性，逐文件 beginSave/
 * write/finish）；沙箱 iframe（fileTransfer 缺失）走 sidecar 落盘——与 GIF
 * 导出同路，支持下载目录设置与「每次询问」；web/docker（sidecar 不在本机）
 * 回退浏览器 <a download>（与 SFTP 下载链路同一兜底写法）。
 */
async function saveTrzszDownloadedFiles(files: readonly TrzszDownloadFile[]) {
  const fileTransfer = rpcAvailable() ? window.dbxPlugin.fileTransfer : undefined;
  const saving = files.filter((file) => !file.isDirectory && file.byteLength > 0);
  if (!saving.length) return;
  if (!fileTransfer) {
    const local = await probeLocalCapabilities();
    if (!local?.canSaveLocal) {
      // issue #93：沙箱 iframe 内 <a download> 被浏览器静默丢弃，改为宿主
      // host.saveFile 单次落盘（取消/无桥/超限抛错走 showError）。
      for (const file of saving) await saveHostFile(file.chunks, file.fileName);
      return;
    }
    // 「使用默认地址」关闭时按批次只问一次，整批落同一目录；取消则整批不保存。
    let targetDir = "";
    let setDefaultAfter = false;
    if (!loadDownloadUseDefaultDir()) {
      const chosen = await askDownloadTarget(saving[0].fileName);
      if (chosen === undefined) return;
      targetDir = chosen.dir.trim();
      setDefaultAfter = chosen.setDefault;
    }
    const dir = targetDir || loadDownloadDir() || undefined;
    let lastSaved: { localPath: string; name: string } | undefined;
    for (const file of saving) {
      // 冲突策略逐文件生效：ask 撞名逐个询问，取消只跳过该文件。
      const conflict = await resolveDownloadConflictFor(dir || "", file.fileName);
      if (conflict === undefined) continue;
      const merged = new Uint8Array(file.byteLength);
      let offset = 0;
      for (const chunk of file.chunks) {
        merged.set(chunk, offset);
        offset += chunk.length;
      }
      lastSaved = await rpc<{ localPath: string; name: string }>("local/saveFile", {
        name: file.fileName,
        dataBase64: window.dbxPlugin.encodeBase64(merged),
        targetDir: dir,
        conflict: conflict === "overwrite" ? "overwrite" : undefined,
      });
    }
    if (lastSaved) {
      const savedPath = lastSaved.localPath;
      showNotice(saving.length === 1
        ? t("downloadedTo", { name: lastSaved.name, path: savedPath })
        : t("downloadedToDir", { count: saving.length, path: dir || savedPath.replace(/[\\/][^\\/]+$/, "") }), [
        { label: t("revealInFolder"), run: () => void revealTransferTarget(savedPath) },
      ]);
    }
    if (setDefaultAfter) applyChosenDirAsDefault(targetDir);
    return;
  }
  for (const file of saving) {
    // 契约：用户取消原生保存框返回 null——取消整批（与 SFTP 下载取消语义一致）。
    const target = await fileTransfer.beginSave({ name: file.fileName, size: file.byteLength });
    if (!target) throw new Error(t("transferStatus.cancelled"));
    try {
      let offset = 0;
      for (const chunk of file.chunks) {
        // issue #116：宿主桥 write 分支把 payload 直放 postMessage transfer
        // 列表，Uint8Array 视图会被 Chromium 拒绝（transferable type），必须
        // 交独立 ArrayBuffer。
        const write = await fileTransfer.write(target.handleId, offset, standaloneArrayBuffer(chunk));
        offset = write.nextOffset;
      }
      await fileTransfer.finish(target.handleId);
    } catch (cause) {
      await fileTransfer.cancel(target.handleId).catch(() => undefined);
      throw cause;
    }
  }
}

/** 终端输入路由（route === "trzsz"）：传输中键入进 filter（Ctrl+C 停传输），
 * 等待协商期直接吞掉（防止杂散键入干扰 trz 握手）。 */
function feedTrzszTerminalInput(data: string) {
  if (trzszPhase.value === "transferring") trzszFilter?.processTerminalInput(data);
}

/** 终端列宽同步（fit 后调用）：进度条按终端列宽渲染。 */
function setTrzszTerminalColumns(cols: number) {
  trzszFilter?.setTerminalColumns(cols);
}

onBeforeUnmount(() => {
  window.clearTimeout(trzszDetectionTimer);
  window.clearTimeout(trzszWatchdogTimer);
  window.clearTimeout(trzszOverlayTimer);
  // 卸载兜底：在途选文件结算 + 摘除 focus 监听与内层定时器（否则回调会
  // post-teardown 打到已销毁的 composable）。
  finishTrzszPick(undefined);
  trzszFilter?.stopTransferringFiles();
  trzszFilter = null;
});

  return {
    trzszInput,
    trzszPhase,
    trzszDirection,
    trzszFileName,
    trzszFileIndex,
    trzszFileCount,
    trzszPercent,
    trzszSpeed,
    trzszMessage,
    trzszBusy,
    trzszOverlayVisible,
    trzszStatusLabel,
    ensureTrzszFilter,
    handleTrzszAnnounce,
    cancelTrzszTransfer,
    teardownTrzsz,
    chooseTrzszUpload,
    pickTrzszUploadFiles,
    onTrzszPickInput,
    onTrzszPickCancel,
    feedTrzszTerminalInput,
    setTrzszTerminalColumns,
  };
}
