import { computed, ref, type Ref } from "vue";
import { isSelectableSftpEntry, filterSelectableSftpUris } from "../../lib/sftpContextMenu";
import { decideFileRowAction } from "../../lib/fileRowKeydown";

interface EntryLike {
  uri: string;
  name: string;
  kind: string;
  permissions?: string;
}

/**
 * SFTP 面板行选中态：单选锚点 + 多选 URI 集合（Set 做热点查询）、受保护
 * 目录拦截、文件行键盘语义。选中/剪贴板动作本体在 useSftpSelectionClipboard，
 * 这里只做树行入口的统一过滤与键盘决策。
 */
export function useSftpRowSelection<TEntry extends EntryLike>(options: {
  currentPath: Ref<string>;
  loadedTreeEntries: Ref<Array<{ uri: string; kind: string }>>;
  selectedPath: Ref<string>;
  deleteTarget: Ref<TEntry | undefined>;
  pathFromUri: (uri: string) => string;
  canWrite: () => boolean;
  selectFile: (entry: TEntry, event: MouseEvent) => void;
  openEditorTab: (entry: TEntry) => unknown;
  beginRename: (entry: TEntry) => void;
}) {
  const { currentPath, loadedTreeEntries, selectedPath, deleteTarget, pathFromUri, canWrite, selectFile, openEditorTab, beginRename } = options;

  const selectedUris = ref<string[]>([]);
  // 选中态热点查询走 Set：行 class 绑定与 selectedEntries 每次渲染对全部
  // 可见条目各查一次，数组 includes 是 O(条目×选中数)（万行目录 × 多选
  // 1000 项 ≈ 每帧千万次比较）；Set 写入仍走 selectedUris 数组。
  const selectedUriSet = computed(() => new Set(selectedUris.value));
  const lastClickedUri = ref("");

  function remoteBasename(path: string) {
    const index = path.lastIndexOf("/");
    return index < 0 ? path : path.slice(index + 1);
  }

  /** 根目录、当前目录及其任一祖先目录禁止进入多选：它们属于批量删除/归档/
   *  下载的高危目标，且把当前目录或它的祖先移走会让整个面板状态失效。
   *  复制/预览/下载等只读动作不受此限制（菜单按 fileMenuProtected 逐项禁用）。 */
  function isProtectedDirectory(entry: { uri: string; kind: string }): boolean {
    return !isSelectableSftpEntry(entry, currentPath.value, pathFromUri);
  }

  // 已加载树里的目录 URI 集合：URI 选区过滤器据此区分目录与文件；树尚未
  // 加载到的 URI 按非目录保留，避免误删文件选择。
  const loadedDirectoryUris = computed(() => {
    const uris = new Set<string>();
    for (const entry of loadedTreeEntries.value) {
      if (entry.kind === "directory") uris.add(entry.uri);
    }
    return uris;
  });

  function isLoadedDirectoryUri(uri: string): boolean {
    return loadedDirectoryUris.value.has(uri);
  }

  /** 树行选中入口（文件/文件夹统一）：先拦截根/当前目录/祖先目录，再交给
   *  多选逻辑；Ctrl 追加与 Shift 范围结果统一过过滤器，受保护目录不会进入
   *  selectedUris。 */
  function selectTreeEntry(entry: TEntry, event: MouseEvent | KeyboardEvent) {
    if (isProtectedDirectory(entry)) {
      // 根/当前目录/祖先目录不进入批量选区，但单击仍要给出视觉锚点；普通
      // 单击同时清掉旧批量选区，与「单击即替换选区」的文件行语义一致。
      selectedPath.value = entry.uri;
      if (!event.ctrlKey && !event.metaKey && !event.shiftKey) {
        selectedUris.value = [];
        lastClickedUri.value = "";
      }
      return;
    }
    selectFile(entry, event as MouseEvent);
    const selectable = filterSelectableSftpUris(
      selectedUris.value,
      currentPath.value,
      pathFromUri,
      isLoadedDirectoryUri,
    );
    if (selectable.length === selectedUris.value.length) return;
    selectedUris.value = selectable;
    // Shift 范围可能把 selectedPath（点击入口）之外的受保护目录带入，但点击
    // 入口本身已被拦；仍按兜底处理：锚点被剔除时清空高亮，避免选中样式残留。
    if (selectedPath.value && !selectable.includes(selectedPath.value)) selectedPath.value = "";
  }

  // R3-P2-5：文件行键盘语义——Enter 打开（目录进入/文件预览）、F2 重命名、
  // Delete 删除，对齐主流文件管理器；动作决策走 fileRowKeydown 纯模块（有
  // 单测），重命名输入框内的按键已自带 .stop。
  function onFileRowKeydown(event: KeyboardEvent, entry: TEntry) {
    const action = decideFileRowAction(event.key, canWrite());
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    // 根/当前目录：键盘改名/删除与右键菜单一样禁用（范围选择可包含文件夹，
    // 但不能把正在浏览的目录移走/删掉）。
    if (isProtectedDirectory(entry) && action !== "open") return;
    if (action === "open") void openEditorTab(entry);
    else if (action === "rename") beginRename(entry);
    else deleteTarget.value = entry;
  }

  return {
    selectedUris,
    selectedUriSet,
    lastClickedUri,
    remoteBasename,
    isProtectedDirectory,
    loadedDirectoryUris,
    isLoadedDirectoryUri,
    selectTreeEntry,
    onFileRowKeydown,
  };
}
