import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import { filesFromClipboard } from "../../../shared/lib/clipboardFiles";
import { displayPathToWire } from "../lib/sftpName";
import { normalizeDropTargetDir, resolveDropTargetDir } from "../../terminal/lib/behavior/terminalInteraction";
import { runTransfers, summarizeBatchUpload } from "../../transfers/lib/transferQueue";
import { createFileBatchCard, type FileBatchCardState } from "../../transfers/lib/fileBatch";
import { X } from "@lucide/vue";
import { rpcAvailable } from "../../../shared/lib/rpc";
import { useUploadDuplicates } from "./useUploadDuplicates";
import { useFolderUpload } from "./useFolderUpload";
import { useHostFileDrag } from "./useHostFileDrag";

/** 上传链（大功能聚合）：fileTransfer 选择器 → 原生回退 → 本地/文件夹/
 * 拖拽/剪贴板五路上传入口，统一汇入 uploadSource 传输队列；重复目标策略
 * （P1-5：预检 + 重命名/覆盖/询问「应用到全部」）与拖拽落点解析（终端
 * cwd → sftp home → 面板目录，wire 形式 M17）一并收口。
 * 拆分：重复裁决（useUploadDuplicates）、文件夹车道（useFolderUpload）、
 * 拖拽路由（useHostFileDrag）各自独立，本文件仍持有选项/落点状态/选择器/
 * 本地文件车道/拖入询问链与最终返回对象。 */
export function useUploadChain(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  connected: Ref<boolean>;
  canWrite: Ref<boolean>;
  currentPath: Ref<string>;
  sftpHomePath: Ref<string>;
  terminalCwd: Ref<string>;
  sftpNameEncodingState: Ref<string>;
  dragActive: Ref<boolean>;
  terminalDragActive: Ref<boolean>;
  sftpPaneOpen: Ref<boolean>;
  terminalTransferBusy: Ref<boolean>;
  uploadSource: (name: string, size: number, readChunk: (offset: number, length: number) => Promise<Uint8Array>, resume?: { taskId: string; remotePath: string }, targetDir?: string, options?: { duplicatePreCheckedAbsent?: boolean; batchId?: string }) => Promise<boolean>;
  /** 传输任务账本（issue #165）：批次字节进度按 batchId 聚合。 */
  transferTasks: Record<string, import("../../transfers/lib/transferQueue").TransferTask>;
  loadTransferConcurrency: () => number;
  loadTransferMaxActive: () => number;
  loadTransferDuplicatePolicy: () => "rename" | "ask" | "overwrite";
  transferDuplicateState: Ref<"rename" | "ask" | "overwrite">;
  terminal: () => { focus: () => void } | undefined;
  loadDirectory: (path?: string) => Promise<void>;
  /** 上传成功后的刷新入口：参数是实际落点目录（undefined = 当前目录）。
   * 缺省回退 loadDirectory()（既有调用方行为）。App 注入树刷新后，拖到
   * 树行/自定义目录的上传也能立刻在列表与树里可见。 */
  refreshAfterUpload?: (targetDir?: string) => Promise<void>;
  openTransferPanel: () => void;
  joinRemote: (parent: string, name: string) => string;
  /** UploadSudo：与 useTransferQueue 同源判定（sudo 模式 + 可写连接）。重复
   * 预检（exists/rename-unique）与文件夹上传的逐目录建目录随之换 sudo 族——
   * 目标目录（如 /root）对登录用户不可读时 SFTP 探测/建目录必失败。 */
  sudoUpload: () => boolean;
}) {
  const { t, showNotice, showError, session, connected, canWrite, currentPath, sftpHomePath, terminalCwd, sftpNameEncodingState, dragActive, terminalDragActive, sftpPaneOpen, terminalTransferBusy, uploadSource, loadTransferConcurrency, loadTransferMaxActive, loadTransferDuplicatePolicy, transferDuplicateState, terminal: terminalGet, loadDirectory, refreshAfterUpload, openTransferPanel, joinRemote, sudoUpload, transferTasks } = options;

  /** 上传成功后的刷新（文件选择器/本地文件/文件夹/归档/宿主句柄统一入口）：
   * 优先注入的 refreshAfterUpload（按实际落点刷新列表与树），缺省回退
   * loadDirectory()（当前目录，保持旧调用方行为）。 */
  async function refreshUploadedTarget(targetDir?: string) {
    if (refreshAfterUpload) {
      await refreshAfterUpload(targetDir);
      return;
    }
    await loadDirectory();
  }

  // 批次并发与会话深度取小（评审 L-4）：批次上限（1..10）超过会话深度
  // （1..8，sidecar 权威）时，超出的任务会在 upload/start 被 "already has
  // N active transfers" 拒绝并中止整批——源头钳住而不是事后补救。
  const batchTransferLimit = () => Math.min(loadTransferConcurrency(), loadTransferMaxActive());

  // 重复目标策略（P1-5）：弹窗槽位与批次「应用到全部」决策槽为单实例，
  // 所有上传车道共用（见 useUploadDuplicates）。
  const {
    uploadDuplicatePrompt,
    uploadDuplicateApplyAll,
    resolveUploadDuplicate,
    resolveUploadDuplicateName,
    uploadWithDuplicatePolicy,
    resetUploadDuplicateBatchDecision,
  } = useUploadDuplicates({ uploadSource, currentPath, sudoUpload, joinRemote, session, transferDuplicateState });

const uploadInput = ref<HTMLInputElement>();
// 文件夹行右键入口的落点：文件选择器是异步的，先把目标目录记下来，等
// input change 回来再消费（工具栏/拖拽入口不传，沿用当前目录）。
const pendingUploadTargetDir = ref<string>();

// 普通多文件上传的批次聚合（issue #165）：一次选择/拖拽/粘贴的多文件在
// 传输面板聚成一张「文件 N/M · 字节」进度卡——逐文件卡在并发受限与文件
// 交接的间隙会整卡消失（完成卡即转入历史），用户感知为"传完就没然后
// 突然又来几个进度条"；批次卡在整批期间常驻填补空窗。单文件不显示。
// 生命周期（建卡/收尾清空）收口在 lib/fileBatch：end 在批次编排结束后
// 延迟清卡——不清卡会在任务账本 30s 终态清收后回跳成 0/N · 0%。
const fileBatchCard = ref<FileBatchCardState>();
const fileBatch = createFileBatchCard(fileBatchCard);
onBeforeUnmount(() => fileBatch.dispose());
/** 批次进度卡数据：文件数随任务账本结算推进，字节按 batchId 聚合。 */
const fileBatchProgress = computed(() => {
  const batch = fileBatchCard.value;
  if (!batch) return undefined;
  return summarizeBatchUpload(Object.values(transferTasks), batch.batchId, batch.filesTotal, batch.totalBytes);
});

const dropUploadPrompt = ref<{ files: Array<{ name: string }> }>();
const dropUploadTarget = ref<"cwd" | "custom">("cwd");
// 拖拽落点解析：终端 cwd（OSC 7/633）优先，其次远端主目录，最后兜底面板目
// 录——终端拖拽只在面板关闭时接收，面板目录此刻不可见，仅作旧 sidecar 兜底。
// 弹窗展示的就是这里的解析结果。
const dropCwdTarget = computed(() => resolveDropTargetDir({ terminalCwd: terminalCwd.value || undefined, sftpHome: sftpHomePath.value || undefined, fallback: currentPath.value }));
// M17 增量②：上传落点的 wire 形式（弹窗仍展示 dropCwdTarget 的显示形式）。
// shell cwd 回读与 sftp home 探测结果是显示文本，latin-1 下经 displayPathToWire
// 转成 wire 形式（% 自转义、U+0080..FF → %XX、>U+00FF 按 UTF-8 兜底），与本地
// 文件名 join 后整条符合 sidecar write_path_bytes 的「wire 目录前缀 + 用户新
// 输入的显示末段」分工；fallback（面板当前目录）本身来自列表链的 wire 形式，
// 原样透传。已知边界：shell cwd 回读中非 UTF-8 的服务器字节在终端解码层已
// 丢失（U+FFFD），无法还原为 latin-1 字节（登记，不做恢复）。
const dropCwdTargetWire = computed(() => resolveDropTargetDir({
  terminalCwd: wireDropDir(terminalCwd.value),
  sftpHome: wireDropDir(sftpHomePath.value),
  fallback: currentPath.value,
}));

/** latin-1 显示文本 → wire 形式（拖入上传的手输/shell cwd 目录）；auto 原样。 */
function wireDropDir(dir: string | undefined): string | undefined {
  if (!dir) return dir;
  return sftpNameEncodingState.value === "latin-1" ? displayPathToWire(dir) : dir;
}
const dropUploadPathInput = ref("");
const dropUploadPathInputEl = ref<HTMLInputElement>();

let dropUploadResolver: ((choice: "cancel" | "cwd" | { dir: string }) => void) | undefined;

async function chooseUpload(targetDir?: string) {
  if (!connected.value || !canWrite.value) return;
  openTransferPanel();
  if (!rpcAvailable() || !window.dbxPlugin.fileTransfer) {
    pendingUploadTargetDir.value = targetDir;
    uploadInput.value?.click();
    return;
  }
  try {
    const selection = await window.dbxPlugin.fileTransfer.pick({ multiple: true });
    const uploaded = await uploadHandleFiles(selection.files, targetDir);
    // 刷新已由 uploadHandleFiles 在成功后按实际落点完成（refreshAfterUpload/
    // loadDirectory），这里不再重复整目录拉取。
    if (uploaded) showNotice(t("uploaded", { count: uploaded }));
  } catch (cause) {
    // 宿主文件桥失败（pick 或读盘，如 unknown plugin file handle，issue #83/#79）
    // 时不再直接终止：回退到 webview 原生文件选择（File API），上传仍可继续。
    if (isHostBridgeReadFailure(cause)) {
      fallbackToNativeUploadPicker(targetDir);
      return;
    }
    showError(cause);
  }
}

/** 宿主桥句柄批量上传；返回真实上传数（重复弹窗放弃跳过的不计）。
 * forceAskDuplicates：拖拽/文件行入口让同名冲突无视用户默认策略（rename）
 * 也弹既有覆盖/重命名询问。 */
async function uploadHandleFiles(files: Array<{ handleId: string; name: string; size: number }>, targetDir?: string, options?: { forceAskDuplicates?: boolean }): Promise<number> {
  if (!rpcAvailable() || !window.dbxPlugin.fileTransfer || !files.length) return 0;
  resetUploadDuplicateBatchDecision();
  const batchId = fileBatch.begin(files);
  let uploaded = 0;
  try {
    await runTransfers(files, batchTransferLimit(), {
      id: (file) => file.handleId,
      run: async (file) => {
        try {
          const done = await uploadWithDuplicatePolicy(file.name, file.size, async (offset, length) => {
            const result = await window.dbxPlugin.fileTransfer!.read(file.handleId, offset, length);
            return window.dbxPlugin.decodeBase64(result.dataBase64);
          }, targetDir, { batchId, forceAskDuplicates: options?.forceAskDuplicates });
          if (done) uploaded += 1;
        } catch (cause) {
          // 桥接读盘错误转成可理解的提示；uploadSource 已补 upload-read-failed 代码，
          // 终端拖入路径（同函数）的 showError 也会显示这条友好文案。
          if (isHostBridgeReadFailure(cause)) {
            const code = (cause as Error & { code?: unknown }).code;
            throw Object.assign(new Error(t("uploadBridgeReadFailed", { name: file.name })), { code, cause });
          }
          throw cause;
        } finally {
          await window.dbxPlugin.fileTransfer!.cancel(file.handleId).catch(() => undefined);
        }
      },
    });
  } finally {
    // 批次收尾（issue #165）：成功、真实失败中止或重复弹窗放弃都走这里，
    // 卡片在保留窗口后清掉；批间重叠由 batchId 守卫（end 只收自己这批）。
    fileBatch.end(batchId);
  }
  if (uploaded) await refreshUploadedTarget(targetDir);
  return uploaded;
}

function isHostBridgeReadFailure(cause: unknown): boolean {
  if (!(cause instanceof Error)) return false;
  const code = (cause as Error & { code?: unknown }).code;
  return code === "upload-read-failed" || /file handle/i.test(cause.message);
}

// 桥接不可用时的兜底（对标 dbx-plugin-files PR #47）：提示后自动打开 webview
// 原生文件选择器（File API，不依赖宿主句柄），上传仍可完成。
function fallbackToNativeUploadPicker(targetDir?: string) {
  showNotice(t("uploadBridgeFallback"));
  pendingUploadTargetDir.value = targetDir;
  uploadInput.value?.click();
}

async function uploadLocalFiles(files: readonly File[], targetDir?: string, options?: { forceAskDuplicates?: boolean }) {
  openTransferPanel();
  resetUploadDuplicateBatchDecision();
  // File 对象没有稳定 id：包一层带序号的 key 再交给调度器。
  const entries = files.map((file, index) => ({ file, key: `local-${index}` }));
  const batchId = fileBatch.begin(files);
  let uploaded = 0;
  try {
    await runTransfers(entries, batchTransferLimit(), {
      id: (entry) => entry.key,
      run: async (entry) => {
        const done = await uploadWithDuplicatePolicy(
          entry.file.name,
          entry.file.size,
          async (offset, length) => new Uint8Array(await entry.file.slice(offset, offset + length).arrayBuffer()),
          targetDir,
          { batchId, forceAskDuplicates: options?.forceAskDuplicates },
        );
        if (done) uploaded += 1;
      },
    });
  } finally {
    // 批次收尾（issue #165）：编排结束即安排卡片清空，失败中止同帧收尾。
    fileBatch.end(batchId);
  }
  if (uploaded) await refreshUploadedTarget(targetDir);
  if (uploaded) showNotice(t("uploaded", { count: uploaded }));
}

// 文件夹上传（issue #78）：webkitdirectory 选择器/逐目录 ensure/归档车道
// 收口在 useFolderUpload，重复裁决复用上面的单一决策槽。
const {
  folderUploadInput,
  folderUploadSupported,
  folderUploadProgress,
  chooseFolderUpload,
  onFolderUploadInput,
  uploadFolderFiles,
} = useFolderUpload({
  t,
  showNotice,
  showError,
  connected,
  canWrite,
  session,
  currentPath,
  sftpNameEncodingState,
  openTransferPanel,
  uploadSource,
  joinRemote,
  sudoUpload,
  loadTransferDuplicatePolicy,
  batchTransferLimit,
  resolveUploadDuplicateName,
  resetUploadDuplicateBatchDecision,
  refreshUploadedTarget,
});

/**
 * Native file paste is the browser-compatible bridge for Finder/Explorer
 * clipboard files. Text paste is deliberately left untouched so path/search
 * inputs and the remote SFTP clipboard keep their existing behavior.
 */
function onSftpClipboardPaste(event: ClipboardEvent) {
  if (!connected.value || !canWrite.value) return;
  const files = filesFromClipboard(event.clipboardData);
  if (!files.length) return;
  event.preventDefault();
  event.stopPropagation();
  void uploadLocalFiles(files).catch(showError);
}

function onUploadInput(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files || []);
  input.value = "";
  const targetDir = pendingUploadTargetDir.value;
  pendingUploadTargetDir.value = undefined;
  if (files.length) void uploadLocalFiles(files, targetDir).catch(showError);
}

// 与 askUploadDuplicate 同款互斥：两个拖入批次先后落点询问时，后问覆盖
// dropUploadResolver 会把先问的 promise 顶成永久挂起（拖入静默卡死）。
let dropUploadAskChain: Promise<unknown> = Promise.resolve();

function askDropUploadTarget(files: Array<{ name: string }>): Promise<"cancel" | "cwd" | { dir: string }> {
  const asked = dropUploadAskChain.then(
    () =>
      new Promise<"cancel" | "cwd" | { dir: string }>((resolve) => {
        dropUploadTarget.value = "cwd";
        dropUploadPathInput.value = "";
        dropUploadResolver = resolve;
        dropUploadPrompt.value = { files };
      }),
  );
  dropUploadAskChain = asked.catch(() => undefined);
  return asked;
}

// 拖拽路由（宿主桥 filedrop + SFTP 面板/终端两条 DOM 拖入链路）收口在
// useHostFileDrag：落点状态、上传管线与「拖入询问」仍由本文件注入。
const {
  handleHostFileDrop,
  onDrop,
  onSftpDragEnter,
  onSftpDragLeave,
  onTerminalDragEnter,
  onTerminalDragLeave,
  onTerminalDrop,
} = useHostFileDrag({
  t,
  showNotice,
  showError,
  connected,
  canWrite,
  sftpPaneOpen,
  terminalTransferBusy,
  dragActive,
  terminalDragActive,
  openTransferPanel,
  terminal: terminalGet,
  uploadHandleFiles,
  uploadFolderFiles,
  uploadLocalFiles,
  askDropUploadTarget,
  dropCwdTargetWire,
  wireDropDir,
  isHostBridgeReadFailure,
  fallbackToNativeUploadPicker,
});

// 选中“指定目录”即聚焦路径输入框（禁用态拿不到焦点，所以不在打开时聚焦）：
// 键盘流为拖入 → Tab/方向键切到自定义 → 直接输入 → Enter 提交。
watch(dropUploadTarget, async (target) => {
  if (target !== "custom") return;
  await nextTick();
  dropUploadPathInputEl.value?.focus();
});

function confirmDropUpload() {
  if (!dropUploadPrompt.value) return;
  if (dropUploadTarget.value === "custom") {
    const dir = normalizeDropTargetDir(dropUploadPathInput.value);
    if (!dir) return;
    resolveDropUpload({ dir });
    return;
  }
  resolveDropUpload("cwd");
}

function resolveDropUpload(choice: "cancel" | "cwd" | { dir: string }) {
  dropUploadPrompt.value = undefined;
  const resolve = dropUploadResolver;
  dropUploadResolver = undefined;
  resolve?.(choice);
}


  return {
    uploadInput,
    folderUploadInput,
    folderUploadSupported,
    folderUploadProgress,
    fileBatchProgress,
    dropUploadPrompt,
    dropUploadTarget,
    dropCwdTarget,
    dropCwdTargetWire,
    dropUploadPathInput,
    dropUploadPathInputEl,
    uploadDuplicatePrompt,
    uploadDuplicateApplyAll,
    wireDropDir,
    resolveUploadDuplicateName,
    resolveUploadDuplicate,
    onUploadInput,
    chooseFolderUpload,
    onFolderUploadInput,
    onSftpClipboardPaste,
    handleHostFileDrop,
    uploadLocalFiles,
    uploadFolderFiles,
    onDrop,
    onSftpDragEnter,
    onSftpDragLeave,
    onTerminalDragEnter,
    onTerminalDragLeave,
    onTerminalDrop,
    chooseUpload,
    confirmDropUpload,
    resolveDropUpload,
  };
}
