import { ref, watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { TerminalBehaviorSettings } from "../../lib/appearance/terminalBehavior";
import { resolveRightClickBehavior } from "../../lib/appearance/terminalBehavior";
import type { TransferHistoryEntry } from "../../../transfers/lib/transferHistory";
import { filterSelectableSftpUris, resolveSftpContextMenuSelection } from "../../../sftp/lib/actions/sftpContextMenu";
import { pathFromUri } from "../../../sftp/lib/paths/sftpRemotePaths";

/** 右键菜单/工具条弹层的受控开关：外部状态由 App 或各业务 composable 持有。 */
export interface PopoverBinding {
  isOpen: () => boolean;
  setOpen: (open: boolean) => void;
}

type FileMenuEntry = { uri: string; kind: string };

/**
 * 终端/SFTP 右键菜单与工具条弹层互斥族的开关状态与打开动作。
 *
 * 只有「弹层打开态 + 打开瞬间的负载」属于本 composable；弹层内容与业务动作
 * （粘贴/复制/连接信息探测/指标关闭等）仍由 App 或各自 composable 承载，
 * 经回调注入。「关闭全部兄弟弹层」的族清单仍留在 App 的
 * closeToolbarPopovers（结构守卫按该函数体扫描收口覆盖），这里只调用它。
 */
export function useTerminalMenus<E extends FileMenuEntry>(options: {
  terminal: () => Terminal | undefined;
  terminalBehavior: () => TerminalBehaviorSettings;
  isRdp: () => boolean;
  pasteTerminal: () => void;
  copyTerminalSelection: () => void;
  transferHistoryMenu: Ref<{ taskId: string } | undefined>;
  closeToolbarPopovers: () => void;
  currentPath: Ref<string>;
  selectedPath: Ref<string>;
  getSelectedUris: () => readonly string[];
  setSelectedUris: (uris: string[]) => void;
  setLastClickedUri: (uri: string) => void;
  getSelectedEntryUris: () => string[];
  isLoadedDirectoryUri: (uri: string) => boolean;
  isProtectedDirectory: (entry: E) => boolean;
  copyTextToClipboard: (value: string, noticeKey: string, values?: Record<string, string | number>) => void;
  quickMenu: PopoverBinding;
  resetQuickMenu: () => void;
  sessionMenu: PopoverBinding;
  closeSessionShellViews: () => void;
  connectionInfo: PopoverBinding;
  refreshConnectionInfo: () => void;
  columnsOpen: Ref<boolean>;
  rdpResolution: PopoverBinding;
  rdpPaste: PopoverBinding;
  panelPalette: PopoverBinding;
  batchQuick: PopoverBinding;
  transferPanelOpen: Ref<boolean>;
  closeMetrics: () => void;
}) {
  const {
    terminal, terminalBehavior, isRdp, pasteTerminal, copyTerminalSelection,
    transferHistoryMenu, closeToolbarPopovers, currentPath, selectedPath,
    getSelectedUris, setSelectedUris, setLastClickedUri, getSelectedEntryUris,
    isLoadedDirectoryUri, isProtectedDirectory, copyTextToClipboard,
    quickMenu, resetQuickMenu, sessionMenu, closeSessionShellViews,
    connectionInfo, refreshConnectionInfo, columnsOpen,
    rdpResolution, rdpPaste, panelPalette, batchQuick, transferPanelOpen, closeMetrics,
  } = options;

  // 右键菜单由 reka ContextMenu 承载（定位/碰撞/Esc/外点关闭均交给 reka）；
  // 这里只保留受控 open 状态与负载数据，坐标由 trigger 从原生事件捕获。
  const terminalMenuOpen = ref(false);
  // 行右键菜单：selection 为打开菜单瞬间的多选快照（>1 时切换为批量区）。
  const fileMenu = ref<{ entry: E; selection: string[] }>();
  // 右键目标为根 "/" 或当前目录（文件夹）时置位：菜单保留浏览/复制/下载，
  // 但改名/删除/压缩/切剪等破坏性动作禁用，防止把正在浏览的目录移走/删掉。
  const fileMenuProtected = ref(false);
  // 右键菜单打开瞬间的选区快照（复制用它，而不是点击时刻的实时选区）。
  const menuSelectionSnapshot = ref("");

  function showTerminalMenu(event: MouseEvent) {
    // RDP 画布占住终端视图时，右键属于远程桌面（画布 mousedown 已把右键发往
    // rdp/input）；本地/SSH 终端菜单整类不适用，压住菜单与系统菜单即返回。
    if (isRdp()) {
      event.preventDefault();
      return;
    }
    // 右键四档（对标 Tabby「Mouse → Right click」）：off / menu / paste / clipboard。
    // clipboard 档按有无选区决定复制还是粘贴；Shift+右键恒出菜单，是 off 与 paste
    // 档下唯一回到菜单的逃生口（与既有行为一致）。
    // preventDefault 只给非菜单分支：reka 触发器据此跳过开菜单（同时也压住系统菜单）；
    // 菜单分支一旦 preventDefault，ContextMenuTrigger 自己就打不开了。
    const term = terminal();
    const target = resolveRightClickBehavior(terminalBehavior(), {
      hasSelection: term?.hasSelection() ?? false,
      shiftKey: event.shiftKey,
    });
    if (target === "menu") {
      // 打开瞬间快照选区文本：菜单接管焦点后 xterm 可能清空选区，复制时再
      // getSelection() 就会拿到空串（右键复制无效的静默失败路径）。
      menuSelectionSnapshot.value = term?.hasSelection() ? (term.getSelection() ?? "") : "";
      terminalMenuOpen.value = true;
      fileMenu.value = undefined;
      return;
    }
    event.preventDefault();
    terminalMenuOpen.value = false;
    fileMenu.value = undefined;
    if (target === "paste") pasteTerminal();
    else if (target === "copy") copyTerminalSelection();
  }

  function showFileMenu(event: MouseEvent, entry: E) {
    // 不再 preventDefault/stopPropagation：事件要冒泡到包裹文件树的
    // ContextMenuTrigger 完成定位与打开；空白区与行共用同一菜单根，由容器处理器
    // onFileAreaContextMenu 按事件目标区分（行内目标直接返回）。
    // 右键未选中的行（或当前无选区）时先把选区收敛到该行：否则批量动作会继续
    // 作用于右键前的旧选区，而视觉上被点中的行看起来是活动项。命中行已在
    // 选区内（含多选）时保留原选区。收敛结果统一过滤受保护目录：右键受保护
    // 目录只更新 selectedPath 作为视觉锚点，绝不进入批量选区。
    const selectedUris = getSelectedUris();
    const seed = resolveSftpContextMenuSelection(entry.uri, selectedUris);
    const selectable = filterSelectableSftpUris(
      seed ?? selectedUris,
      currentPath.value,
      pathFromUri,
      isLoadedDirectoryUri,
    );
    if (seed) {
      setSelectedUris(selectable);
      if (selectable.length) setLastClickedUri(entry.uri);
    } else if (selectable.length !== selectedUris.length) {
      // 旧状态里残留的受保护目录同样在右键时剔除。
      setSelectedUris(selectable);
    }
    selectedPath.value = entry.uri;
    fileMenuProtected.value = isProtectedDirectory(entry);
    fileMenu.value = {
      entry,
      // 与批量操作/计数同源：只快照仍挂在已加载树上的选中项。
      selection: getSelectedEntryUris(),
    };
    terminalMenuOpen.value = false;
    transferHistoryMenu.value = undefined;
  }

  // 右键菜单关闭（reka 外点/Esc/动作选中都会把 fileMenu 置空）：受保护标记
  // 跟随菜单生命周期，下一次普通行右键重新计算。
  watch(fileMenu, (menu) => {
    if (!menu) fileMenuProtected.value = false;
  });

  function showTransferHistoryMenu(event: MouseEvent, entry: TransferHistoryEntry) {
    // 浏览器下载、上传及旧记录都可能没有可验证的本机路径：拦截系统菜单与
    // reka 触发器（preventDefault 后 reka 跳过打开），但不展示无效操作。
    if (!entry.localPath) {
      event.preventDefault();
      transferHistoryMenu.value = undefined;
      return;
    }
    transferHistoryMenu.value = { taskId: entry.taskId };
    terminalMenuOpen.value = false;
    fileMenu.value = undefined;
  }

  function closeMenus() {
    terminalMenuOpen.value = false;
    fileMenu.value = undefined;
    transferHistoryMenu.value = undefined;
    closeToolbarPopovers();
  }

  /** document click 收口：reka 菜单/弹层内容 portal 到 body，内部点击会冒泡到
   *  document——旧实现面板上有 @click.stop，内部点击从不触发这里的清扫，保持语义一致。
   *  dialog-content 同样排除：模态弹窗（如 Docker 面板的日志抽屉/kill 确认）
   *  portal 在工具条 popover 之外，点弹窗内部不该把背后的工具条弹层连带收掉。 */
  function onDocumentClickCloseMenus(event: MouseEvent) {
    if ((event.target as HTMLElement | null)?.closest?.('[data-slot="context-menu-content"], [data-slot="popover-content"], [data-slot="dialog-content"]')) return;
    closeMenus();
  }

  /** 多选批量：复制所选路径（换行拼接写入剪贴板）。 */
  function copySelectedPaths() {
    const menu = fileMenu.value;
    fileMenu.value = undefined;
    if (!menu) return;
    const uris = menu.selection.length ? menu.selection : [menu.entry.uri];
    copyTextToClipboard(uris.map((uri) => pathFromUri(uri)).join("\n"), "sftpCopy.copiedPaths", { count: uris.length });
  }

  // ---- 模板内联互斥清单收敛为具名 toggle（round2），与五个函数 toggle 同族 ----

  function toggleQuickMenu() {
    const next = !quickMenu.isOpen();
    closeToolbarPopovers();
    quickMenu.setOpen(next);
    if (next) {
      // 每次打开回到列表态：清空搜索/展开（M32-A3 后弹层只剩列表执行）。
      resetQuickMenu();
    }
  }

  // 会话下拉：与其它 popover 同款互斥收口；打开时回到动作列表视图。
  function toggleSessionMenu() {
    const next = !sessionMenu.isOpen();
    closeToolbarPopovers();
    sessionMenu.setOpen(next);
    if (next) closeSessionShellViews();
  }

  function toggleConnectionInfo() {
    const next = !connectionInfo.isOpen();
    closeToolbarPopovers();
    connectionInfo.setOpen(next);
    if (next) refreshConnectionInfo();
  }

  function toggleColumnsMenu() {
    const next = !columnsOpen.value;
    closeToolbarPopovers();
    columnsOpen.value = next;
  }

  // 锚点按钮阻止了 pointerdown 的外点关闭，切换弹层时由这里显式互斥。
  function toggleRdpResolutionMenu() {
    const next = !rdpResolution.isOpen();
    closeToolbarPopovers();
    rdpResolution.setOpen(next);
  }

  function toggleRdpPasteMenu() {
    const next = !rdpPaste.isOpen();
    closeToolbarPopovers();
    rdpPaste.setOpen(next);
  }

  function togglePanelPaletteMenu() {
    const next = !panelPalette.isOpen();
    closeToolbarPopovers();
    panelPalette.setOpen(next);
  }

  function toggleBatchQuickPopover() {
    const next = !batchQuick.isOpen();
    closeToolbarPopovers();
    batchQuick.setOpen(next);
  }

  function toggleTransferPanel() {
    const next = !transferPanelOpen.value;
    closeToolbarPopovers();
    if (next) closeMetrics();
    transferPanelOpen.value = next;
  }

  return {
    terminalMenuOpen,
    fileMenu,
    fileMenuProtected,
    menuSelectionSnapshot,
    showTerminalMenu,
    showFileMenu,
    showTransferHistoryMenu,
    closeMenus,
    onDocumentClickCloseMenus,
    copySelectedPaths,
    toggleQuickMenu,
    toggleSessionMenu,
    toggleConnectionInfo,
    toggleColumnsMenu,
    toggleRdpResolutionMenu,
    toggleRdpPasteMenu,
    togglePanelPaletteMenu,
    toggleBatchQuickPopover,
    toggleTransferPanel,
  };
}
