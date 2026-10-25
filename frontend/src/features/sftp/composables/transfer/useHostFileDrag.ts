import { type Ref } from "vue";
import { collectDropRootEntries, scanDroppedEntries, type DropRootEntry } from "../../lib/tree/dropEntries";
import { planHostFileDrop } from "../../lib/transfer/hostFileDrop";
import { canAcceptTerminalDrop, canAcceptFileDrop, dragLeftPane } from "../../../terminal/lib/behavior/terminalInteraction";
import { rpcAvailable, RpcUnavailableError } from "../../../../shared/lib/rpc";

/** 文件夹管线条目（与 useFolderUpload 的入参结构同构）。 */
interface FolderUploadEntry {
  name: string;
  relativePath: string;
  size: number;
  readChunk: (offset: number, length: number) => Promise<Uint8Array>;
}

// 宿主桥 filedrop 条目（relativePath 为可选增量：宿主遍历目录后附带，
// 旧宿主不带——插件前向兼容，见 handleHostFileDrop）。
type HostFileDropFile = { handleId: string; name: string; size: number; contentType: string; relativePath?: string };

/** 拖拽路由（宿主桥 filedrop + SFTP 面板/终端两条 DOM 拖入链路）：
 * 落点解析、门禁与「拖入询问」占位由 useUploadChain 注入——拖入本身不
 * 持有上传管线。 */
export function useHostFileDrag(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  connected: Ref<boolean>;
  canWrite: Ref<boolean>;
  sftpPaneOpen: Ref<boolean>;
  terminalTransferBusy: Ref<boolean>;
  dragActive: Ref<boolean>;
  terminalDragActive: Ref<boolean>;
  openTransferPanel: () => void;
  terminal: () => { focus: () => void } | undefined;
  uploadHandleFiles: (files: Array<{ handleId: string; name: string; size: number }>, targetDir?: string, options?: { forceAskDuplicates?: boolean }) => Promise<number>;
  uploadFolderFiles: (entries: readonly FolderUploadEntry[], baseDir?: string, options?: { forceAskDuplicates?: boolean }) => Promise<void>;
  uploadLocalFiles: (files: readonly File[], targetDir?: string, options?: { forceAskDuplicates?: boolean }) => Promise<void>;
  askDropUploadTarget: (files: Array<{ name: string }>) => Promise<"cancel" | "cwd" | { dir: string }>;
  dropCwdTargetWire: Ref<string>;
  wireDropDir: (dir: string | undefined) => string | undefined;
  isHostBridgeReadFailure: (cause: unknown) => boolean;
  fallbackToNativeUploadPicker: (targetDir?: string) => void;
}) {
  const { t, showNotice, showError, connected, canWrite, sftpPaneOpen, terminalTransferBusy, dragActive, terminalDragActive, openTransferPanel, terminal: terminalGet, uploadHandleFiles, uploadFolderFiles, uploadLocalFiles, askDropUploadTarget, dropCwdTargetWire, wireDropDir, isHostBridgeReadFailure, fallbackToNativeUploadPicker } = options;

// 宿主 fileTransfer 桥的拖入链路（桌面端）：OS 级拖放由宿主 webview 捕获并路由
// 到本工作台。与其他两条链路共用同一道门禁：只读连接/断连时拒绝并提示，不能
// 成为绕过 readOnly 的旁路。落点按面板状态分流（planHostFileDrop）：SFTP 面板
// 打开 → 当前目录；终端独占 → 走落点询问；否则忽略。桥故障时与工具栏上传一致
// 回退原生选择器重挑，而不是只报错走死。
async function handleHostFileDrop(files: HostFileDropFile[], targetDir?: string) {
  dragActive.value = false;
  if (!canAcceptFileDrop({ connected: connected.value, canWrite: canWrite.value })) {
    showNotice(t("dropRefused"));
    return;
  }
  const plan = planHostFileDrop({
    files: files.length,
    connected: connected.value,
    canWrite: canWrite.value,
    sftpPaneOpen: sftpPaneOpen.value,
    terminalTransferBusy: terminalTransferBusy.value,
  });
  if (plan.kind === "ignore") return;
  openTransferPanel();
  // 带相对路径的条目（宿主遍历目录后的增量契约）走文件夹管线，其余照旧。
  const folderFiles = files.filter((file) => typeof file.relativePath === "string" && file.relativePath);
  const plainFiles = files.filter((file) => !file.relativePath);
  let plainUploaded = 0;
  try {
    if (plan.kind === "terminal") {
      const choice = await askDropUploadTarget(files);
      terminalGet()?.focus();
      if (choice === "cancel") return;
      // 落点转 wire 形式（M17 增量②，与终端拖拽同款分工）。
      const targetDir = choice === "cwd" ? dropCwdTargetWire.value : wireDropDir(choice.dir);
      // 拖拽入口强制询问同名冲突（覆盖/重命名），不受用户默认 rename 策略影响。
      if (folderFiles.length) await uploadHostFolderEntries(folderFiles, targetDir, { forceAskDuplicates: true });
      if (plainFiles.length) plainUploaded = await uploadHandleFiles(plainFiles, targetDir, { forceAskDuplicates: true });
    } else {
      // 宿主整包接管 drop 时，父级把树行悬停的目录作为落点传进来；缺省仍是
      // 当前 SFTP 目录。
      if (folderFiles.length) await uploadHostFolderEntries(folderFiles, targetDir, { forceAskDuplicates: true });
      if (plainFiles.length) plainUploaded = await uploadHandleFiles(plainFiles, targetDir, { forceAskDuplicates: true });
    }
    if (plainUploaded) showNotice(t("uploaded", { count: plainUploaded }));
  } catch (cause) {
    if (isHostBridgeReadFailure(cause)) fallbackToNativeUploadPicker();
    else showError(cause);
  }
}

/** 宿主桥句柄的文件夹批量上传：readChunk 走 fileTransfer.read，整批结束后
 * 统一回收句柄（readChunk 逐块调用，不能在块级 finally 里提前关闭）。
 * 桥缺失（含「有 fileTransfer 无 invoke」的半桥）不能静默返回：上传无法
 * 进行，但句柄仍必须在 finally 里回收，错误交给 handleHostFileDrop 的
 * catch 统一 showError。 */
async function uploadHostFolderEntries(files: HostFileDropFile[], targetDir?: string, options?: { forceAskDuplicates?: boolean }) {
  // 回收不依赖 invoke：半桥下 fileTransfer 仍在位，不能跟着上传门禁一起跳过。
  const fileTransfer = (window as { dbxPlugin?: DbxPluginApi }).dbxPlugin?.fileTransfer;
  try {
    if (!rpcAvailable()) throw new RpcUnavailableError("sftp/upload/start");
    await uploadFolderFiles(files.map((file) => ({
      name: file.name,
      relativePath: file.relativePath as string,
      size: file.size,
      readChunk: async (offset: number, length: number) => {
        const result = await window.dbxPlugin.fileTransfer!.read(file.handleId, offset, length);
        return window.dbxPlugin.decodeBase64(result.dataBase64);
      },
    })), targetDir, options);
  } finally {
    if (fileTransfer) {
      for (const file of files) {
        await fileTransfer.cancel(file.handleId).catch(() => undefined);
      }
    }
  }
}

function onDrop(event: DragEvent, targetDir?: string) {
  dragActive.value = false;
  // 与终端侧共用同一道门禁；拒绝时给提示而不是无声吞掉拖入。
  if (!canAcceptFileDrop({ connected: connected.value, canWrite: canWrite.value })) {
    showNotice(t("dropRefused"));
    return;
  }
  // 根条目必须在本处理器内同步收（事件让出后 dataTransfer.items 失效）。
  const roots = event.dataTransfer ? collectDropRootEntries(event.dataTransfer.items) : null;
  void (async () => {
    // 拖到树行/自定义目录时用显式落点；缺省仍是当前目录。拖拽入口强制询问
    // 同名冲突（覆盖/重命名），不受用户默认 rename 策略影响。
    if (roots) {
      const scan = await scanDroppedEntries(roots);
      if (scan.directories > 0) {
        // 含目录（空目录也一样）走文件夹管线：零文件由管线给 folderUpload.empty 提示。
        await uploadFolderFiles(scan.entries.map(folderEntryFromFile), targetDir, { forceAskDuplicates: true });
        return;
      }
      if (scan.entries.length) {
        await uploadLocalFiles(scan.entries.map((entry) => entry.file), targetDir, { forceAskDuplicates: true });
        return;
      }
    }
    // webkitGetAsEntry 不可用（旧内核），或 API 在位但条目全部取不到（部分
    // 合成/桥接拖拽仍会填 dataTransfer.files）时回退 files 车道。
    const files = Array.from(event.dataTransfer?.files || []);
    if (files.length) await uploadLocalFiles(files, targetDir, { forceAskDuplicates: true });
  })().catch(showError);
}

/** 拖拽遍历结果 → 文件夹管线条目（readChunk 从 File 分片读）。 */
function folderEntryFromFile(entry: { file: File; relativePath: string }): FolderUploadEntry {
  const file = entry.file;
  return {
    name: file.name,
    relativePath: entry.relativePath,
    size: file.size,
    readChunk: (offset: number, length: number) => file.slice(offset, offset + length).arrayBuffer().then((buffer) => new Uint8Array(buffer)),
  };
}

function onSftpDragEnter(event: DragEvent) {
  // 只对文件拖拽点亮高亮：拖文本/元素路过不应给出可放置暗示（终端侧同款预检）。
  if (!event.dataTransfer?.types.includes("Files")) return;
  dragActive.value = true;
}

function onSftpDragLeave(event: DragEvent) {
  // 拖拽真正离开面板子树才熄灭「上传」高亮（issue #197）：dragleave 在面板内
  // 子元素间移动时也冒泡触发，旧 `.self` 绑定把「经子元素拖出面板/拖出窗口/
  // Esc 取消」的那次 leave 一并吞掉，面板就永久停在上传悬浮层。
  const pane = event.currentTarget;
  if (pane instanceof Node && !dragLeftPane(event, pane)) return;
  dragActive.value = false;
}

function onTerminalDragEnter(event: DragEvent) {
  if (!event.dataTransfer?.types.includes("Files")) return;
  if (!canAcceptTerminalDrop({ connected: connected.value, canWrite: canWrite.value, transferBusy: terminalTransferBusy.value, sftpPaneOpen: sftpPaneOpen.value })) return;
  terminalDragActive.value = true;
}

function onTerminalDragLeave(event: DragEvent) {
  // 同面板侧（issue #197）：按 relatedTarget 判定去向，子元素间路过不闪烁，
  // 拖走/取消即恢复拖入前状态。
  const pane = event.currentTarget;
  if (pane instanceof Node && !dragLeftPane(event, pane)) return;
  terminalDragActive.value = false;
}

function onTerminalDrop(event: DragEvent) {
  terminalDragActive.value = false;
  if (!canAcceptTerminalDrop({ connected: connected.value, canWrite: canWrite.value, transferBusy: terminalTransferBusy.value, sftpPaneOpen: sftpPaneOpen.value })) {
    // 拒绝不再静默：面板打开时指引拖到面板（那里目录可见），其余（断连/
    // 只读/传输占用）给同一条提示。
    showNotice(t(sftpPaneOpen.value ? "terminalDropToPanel" : "dropRefused"));
    return;
  }
  // Files dropped on the terminal ask for a landing directory first: the
  // shell's cwd (SFTP directory tracking) or any absolute directory typed in
  // the prompt — silence would make a wrong-guess overwrite too easy.
  // 根条目同样在本处理器内同步收（与 SFTP 面板拖拽同款约束）。
  const roots = event.dataTransfer ? collectDropRootEntries(event.dataTransfer.items) : null;
  const fallbackFiles = Array.from(event.dataTransfer?.files || []);
  // Firefox 等浏览器对文件夹拖拽不给 dataTransfer.files：弹窗清单回退显示
  // 根目录名（同步可得，不受异步遍历影响）。
  const promptFiles = fallbackFiles.length
    ? fallbackFiles
    : (roots ?? []).filter((root) => root.isDirectory).map((root) => ({ name: root.name }));
  if (!promptFiles.length) return;
  void runTerminalDropUpload(fallbackFiles, roots, promptFiles);
}

async function runTerminalDropUpload(files: File[], roots: DropRootEntry[] | null, promptFiles: Array<{ name: string }>) {
  const choice = await askDropUploadTarget(promptFiles);
  terminalGet()?.focus();
  if (choice === "cancel") return;
  try {
    // 落点转 wire 形式（M17 增量②）：cwd 选项取 wire 化的解析结果，自定义
    // 目录是手输显示文本，latin-1 下经 wireDropDir 转换（auto 原样）。
    const targetDir = choice === "cwd" ? dropCwdTargetWire.value : wireDropDir(choice.dir);
    if (roots) {
      const scan = await scanDroppedEntries(roots);
      if (scan.directories > 0) {
        await uploadFolderFiles(scan.entries.map(folderEntryFromFile), targetDir, { forceAskDuplicates: true });
        return;
      }
    }
    if (files.length) await uploadLocalFiles(files, targetDir, { forceAskDuplicates: true });
  } catch (cause) {
    showError(cause);
  }
}

  return {
    handleHostFileDrop,
    onDrop,
    onSftpDragEnter,
    onSftpDragLeave,
    onTerminalDragEnter,
    onTerminalDragLeave,
    onTerminalDrop,
  };
}
