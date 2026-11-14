import { computed, ref, watch, type Ref } from "vue";
import {
  createFileTreeRoot,
  flattenVisibleTreeRows,
  type SftpFileTreeNode,
} from "../../lib/tree/sftpFileTree";
import { filterSelectableSftpEntries } from "../../lib/actions/sftpContextMenu";
import { normalizeRemotePath, pathFromUri } from "../../lib/paths/sftpRemotePaths";
import { type SftpColumn, type SftpSanitizedEntry } from "../../lib/entries/sftpEntries";
import { type SftpTypeFilter } from "../../lib/entries/sftpFileFilters";
import { useSftpRowSelection } from "../actions/useSftpRowSelection";
import { useSftpRename } from "../actions/useSftpRename";
import { useSftpNavigation } from "./useSftpNavigation";
import { useSftpTreeWorkspace } from "./useSftpTreeWorkspace";

/**
 * 后段声明的 App 权威态以 getter（必要时配 setter）注入，本模块包装成既有
 * composable 要求的 Ref 契约：读走 getter，写回 App 原 ref（唯一权威态）。
 * 只读依赖用 computed；被下游写入的依赖（followDirectory/sftpTypeFilter）必须
 * 给 setter，否则写回会静默落在适配器上而不进 App。
 */
function bindRef<T>(get: () => T, set?: (value: T) => void): Ref<T> {
  return (set ? computed({ get, set }) : computed(get)) as Ref<T>;
}

/** SFTP 工作区：当前目录列表（entries/currentPath 是列表唯一出口）、单栏树、
 * 行选中与行内重命名的组合根。导航（loadDirectory 枢纽）/树懒加载/删除/新建
 * 目录等子域仍由各自 composable 持有，本文件只做接线，不复制实现。 */
export function useSftpWorkspace(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  sftpError: Ref<string>;
  sftpErrorOpen: Ref<boolean>;
  sftpErrorKey: Ref<number>;
  /** 早于本模块声明的权威态（直接传 ref）。 */
  terminalCwd: Ref<string>;
  sftpPaneOpen: Ref<boolean>;
  visibleColumns: Ref<SftpColumn[]>;
  directoryTrackingSupported: Ref<boolean | undefined>;
  directoryParser: { reset(): void };
  sftpTreePane: Ref<{ scrollPathToTop(path: string): Promise<boolean> } | null>;
  deleteTarget: Ref<SftpSanitizedEntry | undefined>;
  joinRemote: (parent: string, name: string) => string;
  /** 晚于本模块声明的权威态（惰性 getter/setter，规避声明顺序）。 */
  getSudoMode: () => boolean;
  setSudoMode: (value: boolean) => void;
  getConnected: () => boolean;
  getCanWrite: () => boolean;
  getIsRdpConnection: () => boolean;
  getLocalUiMode: () => boolean;
  getIsLocalMode: () => boolean;
  getActivePaneCwd: () => string;
  getFollowDirectory: () => boolean;
  setFollowDirectory: (value: boolean) => void;
  getSftpShowHidden: () => boolean;
  getSftpSearch: () => string;
  getSftpFiltersActive: () => boolean;
  getSftpTypeFilter: () => SftpTypeFilter;
  setSftpTypeFilter: (value: SftpTypeFilter) => void;
  /** 晚于本模块声明的接线（惰性回调）。 */
  getPendingTerminalInput: () => string;
  persistState: () => void;
  refreshDiskUsage: () => Promise<void> | void;
  persistDirectoryFollowPref: (value: boolean) => void;
  resetLinkTargets: () => void;
  hydrateLinkTargets: (list: SftpSanitizedEntry[], isCurrent: () => boolean) => void;
  closeBookmarksMenu: () => void;
  clearSftpSearch: () => void;
  clearRowSelection: () => void;
  selectFile: (entry: SftpSanitizedEntry, event: MouseEvent) => void;
  openEditorTab: (entry: SftpSanitizedEntry) => unknown;
  handleEditorRename: (oldPath: string, newPath: string) => void;
}) {
  const currentPath = ref("/");
  const entries = ref<SftpSanitizedEntry[]>([]);
  const selectedPath = ref("");
  const loadingFiles = ref(false);
  // 本会话是否已成功加载过 SFTP 目录：自动连接关闭时，手动打开面板的瞬间
  // 据此惰性补载（每会话一次，断开重置）。
  const sftpListingLoaded = ref(false);
  // sftp/home 探测结果：quick tab 置顶展示（获取失败时该项隐藏）。
  const sftpHomePath = ref("");
  // 单栏文件树：文件夹与文件同树，目录懒加载。根固定连接根 "/"，可见行由
  // 搜索/类型过滤在 flattenVisibleTreeRows 内完成；entries 仍是"当前目录"的
  // 数据源，树节点在 loadDirectory 落地后同步子树。
  const sftpTreeRoot = ref(createFileTreeRoot("/"));
  const sftpTreeRows = computed(() =>
    flattenVisibleTreeRows(sftpTreeRoot.value, {
      search: options.getSftpSearch(),
      type: options.getSftpTypeFilter() === "directory" ? "directory" : options.getSftpTypeFilter() === "file" ? "file" : "all",
    }),
  );
  const sftpTreeEntries = computed(() => sftpTreeRows.value.map((row) => row.node.entry));
  const sftpTreeTotalRows = computed(() => flattenVisibleTreeRows(sftpTreeRoot.value, {}));
  // 已加载树节点全集（含被搜索/类型过滤隐藏或折叠在后代里的条目）：批量计数、
  // 删除/下载目标都以它为准，避免过滤/折叠把已选中的远端条目静默丢掉。
  function collectLoadedTreeEntries(node: SftpFileTreeNode, out: SftpSanitizedEntry[]): void {
    out.push(node.entry);
    for (const child of node.children) collectLoadedTreeEntries(child, out);
  }
  const loadedTreeEntries = computed(() => {
    const out: SftpSanitizedEntry[] = [];
    collectLoadedTreeEntries(sftpTreeRoot.value, out);
    return out;
  });
  // 当前行锚点条目（详情/右键菜单按它取整条元数据）。
  const selectedEntry = computed(() => entries.value.find((entry) => entry.uri === selectedPath.value));
  // 多选可能跨不同已展开目录（也可能被过滤/折叠隐藏）：按整棵已加载树解析，
  // 顺序与树内 DFS 一致；只有已从树上消失的路径才会被剔除。
  // 兜底不变量：即使旧状态/边界路径把受保护目录塞进了 selectedUris，批量
  // 删除/归档/下载也只消费过滤后的条目。
  const selectedEntries = computed(() =>
    filterSelectableSftpEntries(
      loadedTreeEntries.value.filter((entry) => selectedUriSet.value.has(entry.uri)),
      currentPath.value,
      pathFromUri,
    ),
  );

  const sudoMode = bindRef(options.getSudoMode, options.setSudoMode);
  const connected = bindRef(options.getConnected);
  const canWrite = bindRef(options.getCanWrite);
  const followDirectory = bindRef(options.getFollowDirectory, options.setFollowDirectory);
  const sftpShowHidden = bindRef(options.getSftpShowHidden);
  const sftpTypeFilter = bindRef(options.getSftpTypeFilter, options.setSftpTypeFilter);
  const sftpFiltersActive = bindRef(options.getSftpFiltersActive);
  const isRdpConnection = bindRef(options.getIsRdpConnection);
  const localUiMode = bindRef(options.getLocalUiMode);
  const isLocalMode = bindRef(options.getIsLocalMode);
  const activePaneCwd = bindRef(options.getActivePaneCwd);

  // SFTP 目录导航：loadDirectory 枢纽/sudo/路径跳转收口在 composables/useSftpNavigation。
  const {
    loadHome,
    refreshSftpHomePath,
    loadDirectory,
    sftpSoftError,
    toggleSudoMode,
    enableSudoModeAndReload,
    setDirectoryTracking,
    goToPath,
    pathBarEditing, pathBarInputEl, pathBarDraft, pathCrumbs,
    beginPathBarEdit, cancelPathBarEdit, submitPathInput,
  } = useSftpNavigation({
    t: options.t, showNotice: options.showNotice, showError: options.showError,
    session: options.session, sudoMode, connected, canWrite,
    currentPath, entries, loadingFiles,
    sftpError: options.sftpError, sftpErrorOpen: options.sftpErrorOpen, sftpErrorKey: options.sftpErrorKey,
    sftpHomePath, visibleColumns: options.visibleColumns, followDirectory,
    directoryParser: options.directoryParser, selectedPath,
    directoryTrackingSupported: options.directoryTrackingSupported,
    clearRowSelection: () => options.clearRowSelection(), normalizeRemotePath,
    getPendingTerminalInput: () => options.getPendingTerminalInput(),
    persistState: () => options.persistState(), refreshDiskUsage: () => options.refreshDiskUsage(),
    persistDirectoryFollowPref: (value) => options.persistDirectoryFollowPref(value),
    resetLinkTargets: () => options.resetLinkTargets(),
    hydrateLinkTargets: (list, isCurrent) => options.hydrateLinkTargets(list, isCurrent),
    closeBookmarksMenu: () => options.closeBookmarksMenu(),
    onListingLoaded: () => { sftpListingLoaded.value = true; },
  });

  // 行内重命名：提交/短路语义收口在 composables/useSftpRename（树行改名经
  // commitRenameEntry 进入；成功后由 onMutated 刷新列表与已展开树）。
  const {
    renamingPath,
    renameDraft,
    renameSubmitting,
    beginRename,
    commitRename,
  } = useSftpRename({
    t: options.t, showError: options.showError, session: options.session, canWrite, sudoMode,
    selectedPath, pathFromUri, joinRemote: options.joinRemote, loadDirectory,
    onRenamed: (oldPath, newPath) => options.handleEditorRename(oldPath, newPath),
    onMutated: () => refreshSftpAfterMutation(),
  });

  /** 行内重命名提交入口（树行改名经 setRenameDraft + 本函数）。 */
  function commitRenameEntry(entry: SftpSanitizedEntry) {
    return commitRename(entry);
  }

  // 行选中态（多选集合/保护目录拦截/键盘语义）收口在 composables/useSftpRowSelection；
  // 选中/剪贴板动作本体由 App 的 useSftpSelectionClipboard 惰性注入。
  const {
    selectedUris,
    selectedUriSet,
    lastClickedUri,
    remoteBasename,
    isProtectedDirectory,
    loadedDirectoryUris,
    isLoadedDirectoryUri,
    selectTreeEntry,
    onFileRowKeydown,
  } = useSftpRowSelection<SftpSanitizedEntry>({
    currentPath, loadedTreeEntries, selectedPath, deleteTarget: options.deleteTarget, pathFromUri,
    canWrite: () => canWrite.value,
    selectFile: (entry, event) => options.selectFile(entry, event),
    openEditorTab: (entry) => options.openEditorTab(entry),
    beginRename: (entry) => beginRename(entry),
  });

  // SFTP 单栏文件树：懒加载/祖先路径揭示/目录跟随/显示隐藏切换收口在
  // composables/useSftpTreeWorkspace。
  const {
    listSftpDirectory,
    loadSftpDetails,
    clearSftpDetailCache,
    revealSftpPath,
    refreshExpandedSftpTree,
    refreshSftpAfterMutation,
    initializeSftpTree,
    followSftpPath,
    toggleSftpTreeNode,
    openSftpTreeNode,
    commitTreeRename,
    resetSftpTree,
  } = useSftpTreeWorkspace({
    session: options.session, sudoMode, entries, currentPath, sftpListingLoaded, sftpShowHidden,
    sftpTreeRoot, selectedUris, selectedPath, lastClickedUri, sftpErrorKey: options.sftpErrorKey,
    loadedTreeEntries, sftpTreePane: options.sftpTreePane, sftpFiltersActive, sftpTypeFilter,
    sftpPaneOpen: options.sftpPaneOpen, isRdpConnection, localUiMode, isLocalMode, connected,
    followDirectory, activePaneCwd, sftpHomePath, showError: options.showError,
    loadDirectory: (path, fromTerminal, quiet) => loadDirectory(path, fromTerminal, quiet),
    refreshSftpHomePath: () => refreshSftpHomePath(),
    refreshDiskUsage: () => options.refreshDiskUsage(),
    clearSftpSearch: () => options.clearSftpSearch(),
    openEditorTab: (entry) => options.openEditorTab(entry),
    setRenameDraft: (value) => { renameDraft.value = value; },
    commitRenameEntry: (entry) => commitRename(entry),
  });

  // 目录跟随只由真实 cwd 变化驱动（分屏页签的 cwd 由 App 的 onShellTabCwd 接入，
  // 窗格聚焦切换只更新 activeTerminalPaneId，不再回拉各窗格目录）。
  const terminalCwd = options.terminalCwd;
  watch(terminalCwd, (cwd) => {
    if (!followDirectory.value || !cwd || !connected.value) return;
    void followSftpPath(cwd);
  });

  return {
    entries,
    currentPath,
    selectedPath,
    loadingFiles,
    sftpListingLoaded,
    sftpHomePath,
    sftpTreeRoot,
    sftpTreeRows,
    sftpTreeEntries,
    sftpTreeTotalRows,
    loadedTreeEntries,
    selectedEntry,
    selectedEntries,
    selectedUris,
    selectedUriSet,
    lastClickedUri,
    remoteBasename,
    isProtectedDirectory,
    loadedDirectoryUris,
    isLoadedDirectoryUri,
    onSftpSelection: selectTreeEntry,
    onFileRowKeydown,
    loadHome,
    refreshSftpHomePath,
    loadDirectory,
    sftpSoftError,
    toggleSudoMode,
    enableSudoModeAndReload,
    setDirectoryTracking,
    goToPath,
    pathBarEditing,
    pathBarInputEl,
    pathBarDraft,
    pathCrumbs,
    beginPathBarEdit,
    cancelPathBarEdit,
    submitPathInput,
    listSftpDirectory,
    loadSftpDetails,
    clearSftpDetailCache,
    revealSftpPath,
    refreshExpandedSftpTree,
    refreshSftpAfterMutation,
    initializeSftpTree,
    followSftpPath,
    toggleSftpTreeNode,
    openSftpTreeNode,
    commitTreeRename,
    resetSftpTree,
    renamingPath,
    renameDraft,
    renameSubmitting,
    beginRename,
    commitRename,
    commitRenameEntry,
  };
}
