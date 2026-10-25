import { nextTick, watch, type Ref } from "vue";
import { sanitizeSftpEntries, type SftpSanitizedEntry } from "../../lib/entries/sftpEntries";
import {
  applyFileTreeChildren,
  createFileTreeRoot,
  findFileTreeNode,
  isSftpTreeRequestCurrent,
  markCollapsedTreeNodesStale,
  markFileTreeStale,
  type SftpFileTreeNode,
} from "../../lib/tree/sftpFileTree";
import { mergeSftpDetail, type SftpDetailInfo } from "../../lib/actions/sftpDetails";
import { joinRemote, normalizeRemotePath, parentPath, pathFromUri } from "../../lib/paths/sftpRemotePaths";

/**
 * SFTP 单栏文件树：懒加载 / 祖先路径揭示 / 目录跟随 / 显示隐藏切换。
 * 当前目录数据仍以 entries/currentPath 为准（loadDirectory 是唯一出口），树节点
 * 在列表落地后由 entries watcher 同步；祖先路径用独立 list 调用，不改变当前
 * 目录，避免打开深层路径时闪烁。
 */
export function useSftpTreeWorkspace(options: {
  session: Ref<{ sessionId?: string } | undefined>;
  sudoMode: Ref<boolean>;
  entries: Ref<SftpSanitizedEntry[]>;
  currentPath: Ref<string>;
  sftpListingLoaded: Ref<boolean>;
  sftpShowHidden: Ref<boolean>;
  sftpTreeRoot: Ref<SftpFileTreeNode>;
  selectedUris: Ref<string[]>;
  selectedPath: Ref<string>;
  lastClickedUri: Ref<string>;
  sftpErrorKey: Ref<number>;
  loadedTreeEntries: Ref<Array<{ uri: string }>>;
  sftpTreePane: Ref<{ scrollPathToTop(path: string): Promise<boolean> } | null>;
  sftpFiltersActive: Ref<boolean>;
  sftpTypeFilter: Ref<string>;
  sftpPaneOpen: Ref<boolean>;
  isRdpConnection: Ref<boolean>;
  localUiMode: Ref<boolean>;
  isLocalMode: Ref<boolean>;
  connected: Ref<boolean>;
  followDirectory: Ref<boolean>;
  activePaneCwd: Ref<string>;
  sftpHomePath: Ref<string>;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  // 后段声明的依赖（导航/页签/重命名 composable）：惰性 getter，规避声明顺序。
  loadDirectory: (path?: string, fromTerminal?: boolean, quiet?: boolean) => Promise<void> | void;
  refreshSftpHomePath: () => Promise<void> | void;
  clearSftpSearch: () => void;
  openEditorTab: (entry: SftpSanitizedEntry) => void;
  setRenameDraft: (value: string) => void;
  commitRenameEntry: (entry: SftpSanitizedEntry) => void;
}) {
  const {
    session, sudoMode, entries, currentPath, sftpListingLoaded, sftpShowHidden,
    sftpTreeRoot, selectedUris, selectedPath, lastClickedUri, sftpErrorKey,
    loadedTreeEntries, sftpTreePane, sftpFiltersActive, sftpTypeFilter,
    sftpPaneOpen, isRdpConnection, localUiMode, isLocalMode, connected,
    followDirectory, activePaneCwd, sftpHomePath, showError,
    loadDirectory, refreshSftpHomePath, clearSftpSearch, openEditorTab,
    setRenameDraft, commitRenameEntry,
  } = options;

  let sftpTreeSessionId = "";
  let sftpTreeInitInFlight = false;
  // 树 epoch：重建（显隐切换）与会话重置都会递增；reveal/刷新在每次 await 后
  // 校验 epoch，丢弃落在旧根上的迟到结果，避免把子项挂到已被替换的树上。
  let sftpTreeRebuildSeq = 0;

  async function listSftpDirectory(path: string): Promise<SftpSanitizedEntry[]> {
    const sessionId = session.value?.sessionId;
    if (!sessionId) return [];
    const result = await window.dbxPlugin.invoke<{ entries: unknown }>(sudoMode.value ? "sudo/listDir" : "sftp/list", {
      sessionId,
      path: normalizeRemotePath(path),
      includeOwner: false,
    });
    return sanitizeSftpEntries(result.entries);
  }

  // 树行详情懒加载：树列表关闭了 includeOwner，悬浮详情图标时补一次 stat；
  // 结果按 uri+modifiedAt 缓存（最多 200 条），刷新后条目变化会自然重新取。
  const SFTP_DETAIL_CACHE_LIMIT = 200;
  const sftpDetailCache = new Map<string, SftpDetailInfo>();
  // 换会话/重连后同一路径的 stat 可能来自另一台主机，缓存不能跨会话复用。
  watch(() => session.value?.sessionId, () => sftpDetailCache.clear());
  async function loadSftpDetails(entry: SftpSanitizedEntry): Promise<SftpDetailInfo | null> {
    const sessionId = session.value?.sessionId;
    if (!sessionId) return null;
    const fallbackPath = pathFromUri(entry.uri) || "/";
    // chmod 通常不改 mtime：权限串也纳入缓存键。
    const cacheKey = `${entry.uri}|${entry.modifiedAt ?? ""}|${entry.permissions ?? ""}`;
    const cached = sftpDetailCache.get(cacheKey);
    if (cached) return cached;
    try {
      const stat = await window.dbxPlugin.invoke<Partial<SftpDetailInfo> & { mode?: string }>(sudoMode.value ? "sudo/stat" : "sftp/stat", {
        sessionId,
        path: fallbackPath,
      });
      const detail = mergeSftpDetail(entry, stat, fallbackPath);
      sftpDetailCache.set(cacheKey, detail);
      if (sftpDetailCache.size > SFTP_DETAIL_CACHE_LIMIT) {
        const oldest = sftpDetailCache.keys().next().value;
        if (oldest) sftpDetailCache.delete(oldest);
      }
      return detail;
    } catch {
      // 权限/旧 sidecar：详情卡回落到 entry 自带字段，不弹全局错误。
      return null;
    }
  }

  /** 列表是否确实属于 path：路径栏提交会先改 currentPath 再发请求，期间
   *  entries 仍是旧目录，不能把旧子项挂到新节点上。 */
  function listingBelongsToPath(listing: readonly { uri: string }[], path: string): boolean {
    if (!listing.length) return true;
    return listing.every((entry) => parentPath(pathFromUri(entry.uri)) === path);
  }

  /**
   * 展开并揭示绝对路径：根 → 逐段祖先 → 目标。根/祖先未加载时即时列目录；
   * 目标正好是当前目录且列表已落地时直接复用 entries，避免重复请求。
   * tree epoch 在每次 await 后复查（重建/会话重置会换掉根），过期的揭示直接
   * 放弃，不向旧根写子节点。quiet 用于目录跟随等后台路径：失败不弹全局横幅。
   */
  async function revealSftpPath(path: string, options: { quiet?: boolean } = {}): Promise<void> {
    if (!session.value) return;
    const epoch = sftpTreeRebuildSeq;
    const target = normalizeRemotePath(path);
    try {
      let root = sftpTreeRoot.value;
      if (!root.loaded) {
        const rootEntries = currentPath.value === "/" && sftpListingLoaded.value && listingBelongsToPath(entries.value, "/")
          ? entries.value
          : await listSftpDirectory("/");
        if (epoch !== sftpTreeRebuildSeq || !session.value) return;
        root = sftpTreeRoot.value;
        applyFileTreeChildren(root, "/", rootEntries, sftpShowHidden.value);
      }
      root = sftpTreeRoot.value;
      root.expanded = true;
      const segments = target === "/" ? [] : target.split("/").filter(Boolean);
      let cursor = "/";
      for (const segment of segments) {
        const nextPath = joinRemote(cursor, segment);
        let node = findFileTreeNode(root, nextPath);
        if (!node) return; // 隐藏项/已被删除：中止在最近的可用祖先。
        if (node.kind !== "directory") return;
        node.expanded = true;
        if (!node.loaded) {
          const listing = currentPath.value === nextPath && sftpListingLoaded.value && listingBelongsToPath(entries.value, nextPath)
            ? entries.value
            : await listSftpDirectory(nextPath);
          if (epoch !== sftpTreeRebuildSeq || !session.value) return;
          root = sftpTreeRoot.value;
          node = findFileTreeNode(root, nextPath);
          if (!node || node.kind !== "directory") return;
          node = applyFileTreeChildren(root, nextPath, listing, sftpShowHidden.value);
          if (!node) return;
          node.expanded = true;
          node.loaded = true;
        }
        cursor = nextPath;
      }
      if (epoch !== sftpTreeRebuildSeq) return;
      const targetNode = findFileTreeNode(sftpTreeRoot.value, target);
      if (targetNode?.kind === "directory") targetNode.expanded = true;
    } catch (cause) {
      if (epoch !== sftpTreeRebuildSeq) return;
      if (!options.quiet) showError(cause);
    }
  }

  /** 重列所有已加载且展开的树节点（不改变 currentPath/选中）；单个分支失败
   *  保留旧子项，当前目录的错误由 loadDirectory 统一反馈。
   *  已加载但折叠的分支不额外请求（代价高且用户看不到），先标记 loaded=false，
   *  下一次展开由 toggleSftpTreeNode 重新 list——否则在折叠目录里保存/改名后，
   *  重新展开会直接复用旧 children 显示过期内容。 */
  async function refreshExpandedSftpTree(): Promise<void> {
    if (!session.value) return;
    const epoch = sftpTreeRebuildSeq;
    markCollapsedTreeNodesStale(sftpTreeRoot.value);
    const paths: string[] = [];
    collectExpandedTreePaths(sftpTreeRoot.value, paths);
    for (const path of paths) {
      if (epoch !== sftpTreeRebuildSeq || !session.value) return;
      const node = findFileTreeNode(sftpTreeRoot.value, path);
      if (!node || node.kind !== "directory") continue;
      node.loading = true;
      try {
        const listing = await listSftpDirectory(path);
        if (epoch !== sftpTreeRebuildSeq) return;
        applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
      } catch {
        // 分支刷新失败保留旧子树，避免一次抖动把已展开目录清空。
      } finally {
        node.loading = false;
      }
    }
  }

  /** 变更后的统一刷新：当前目录列表（失败逐级回退父目录，覆盖当前目录被删/
   *  祖先被改名的场景）+ 所有已展开分支 + 把当前路径重新揭示。
   *  loadDirectory 会清空选中；这里先快照，刷新完成后只恢复「仍挂在当前树上」
   *  的 URI（编辑器保存保住选中文件，删除/改名的 URI 自然掉落）。会话已切换
   *  时绝不恢复，避免把旧会话选择带到新会话。 */
  async function refreshSftpAfterMutation(): Promise<void> {
    if (!session.value) return;
    const sessionId = session.value.sessionId;
    const pathAtStart = currentPath.value;
    const previousSelection = [...selectedUris.value];
    const previousSelectedPath = selectedPath.value;
    const previousLastClicked = lastClickedUri.value;
    let probe = currentPath.value;
    for (;;) {
      const errorKey = sftpErrorKey.value;
      await loadDirectory(probe);
      if (!session.value) return;
      if (sftpErrorKey.value === errorKey) break;
      const parent = parentPath(probe);
      if (parent === probe) break;
      probe = parent;
    }
    await refreshExpandedSftpTree();
    await revealSftpPath(currentPath.value, { quiet: true });
    // 会话切走、或刷新期间用户已导航到别的目录：不把旧目录的选中带回来。
    if (!session.value || session.value.sessionId !== sessionId || currentPath.value !== pathAtStart || !previousSelection.length) return;
    // 等 entries watcher 把新列表挂到树上后再取交集，避免拿到刷新前的节点。
    await nextTick();
    const loadedUris = new Set(loadedTreeEntries.value.map((entry) => entry.uri));
    const kept = previousSelection.filter((uri) => loadedUris.has(uri));
    if (!kept.length) return;
    selectedUris.value = kept;
    if (kept.includes(previousSelectedPath)) selectedPath.value = previousSelectedPath;
    lastClickedUri.value = kept.includes(previousLastClicked) ? previousLastClicked : kept[0];
  }

  /** 收集当前展开且已加载的目录路径（重建树时保持展开分支）。 */
  function collectExpandedTreePaths(node: SftpFileTreeNode, paths: string[]): void {
    if (node.kind !== "directory") return;
    if (node.expanded && node.loaded) paths.push(node.path);
    for (const child of node.children) collectExpandedTreePaths(child, paths);
  }

  /**
   * 显示/隐藏切换后重建树：旧树标记 stale，按展开路径从新根重放。
   * 重建而不是原地 apply，避免父级重拉时把已加载的深层子树（旧 loaded=false）
   * 误收起；当前目录末端仍用最新 entries，保证隐藏项立即出现/消失。
   */
  async function rebuildSftpTree(): Promise<void> {
    if (!session.value) return;
    const seq = ++sftpTreeRebuildSeq;
    const paths: string[] = [];
    collectExpandedTreePaths(sftpTreeRoot.value, paths);
    const expanded = new Set<string>([currentPath.value, ...paths]);
    markFileTreeStale(sftpTreeRoot.value);
    sftpTreeRoot.value = createFileTreeRoot("/");
    const ordered = [...expanded]
      .filter((path) => path !== "/")
      .sort((left, right) => left.split("/").filter(Boolean).length - right.split("/").filter(Boolean).length);
    for (const path of ordered) {
      if (seq !== sftpTreeRebuildSeq) return;
      await revealSftpPath(path);
    }
    if (seq === sftpTreeRebuildSeq) await revealSftpPath(currentPath.value);
  }

  function resetSftpTree() {
    sftpTreeRebuildSeq += 1;
    sftpTreeRoot.value = createFileTreeRoot("/");
    sftpTreeSessionId = "";
  }

  /** 初始树目标：跟随开启且有终端 cwd → cwd；否则当前用户 home；home 未知 → "/"。 */
  async function resolveSftpTreeTarget(): Promise<string> {
    if (followDirectory.value && activePaneCwd.value) return normalizeRemotePath(activePaneCwd.value);
    if (!sftpHomePath.value) await refreshSftpHomePath();
    return sftpHomePath.value ? normalizeRemotePath(sftpHomePath.value) : "/";
  }

  /** 每会话只初始化一次（/ 与当前目录一致时也确保首个列表落地）。 */
  async function initializeSftpTree(): Promise<void> {
    const sessionId = session.value?.sessionId;
    if (!sessionId || !sftpPaneOpen.value || isRdpConnection.value || localUiMode.value) return;
    if (sftpTreeInitInFlight) return;
    // 会话 id 变了但 closeSession 未走的恢复路径：旧树整体作废。
    if (sftpTreeSessionId && sftpTreeSessionId !== sessionId) resetSftpTree();
    if (sftpTreeSessionId === sessionId && sftpTreeRoot.value.loaded) {
      // 面板收起期间目录跟随只同步了 currentPath：重新打开时把树揭示到当前目录，
      // 跟随开启时仍要求目标贴顶（重新挂载的树从 scrollTop=0 开始）。
      await revealSftpPath(currentPath.value);
      if (followDirectory.value) {
        await nextTick();
        await scrollFollowedTarget(currentPath.value);
      }
      return;
    }
    sftpTreeInitInFlight = true;
    try {
      const before = currentPath.value;
      const target = await resolveSftpTreeTarget();
      if (!session.value || session.value.sessionId !== sessionId) return;
      if (currentPath.value !== before) {
        // home 探测期间用户已手动导航：以用户选择为准，不用初始目标回拉。
        await revealSftpPath(currentPath.value);
        sftpTreeSessionId = sessionId;
        return;
      }
      if (target !== currentPath.value || !sftpListingLoaded.value) await loadDirectory(target);
      await revealSftpPath(target);
      if (followDirectory.value) {
        await nextTick();
        await scrollFollowedTarget(target);
      }
      sftpTreeSessionId = sessionId;
    } finally {
      sftpTreeInitInFlight = false;
    }
  }

  /**
   * 把跟随目标滚到树顶。行数据可能比“揭示/列表落地”晚一个渲染周期（虚拟树
   * 的 rows 来自父级 computed），所以失败时先补一帧重试，再考虑过滤隐藏。
   * 返回是否最终成功（树未挂载时返回 false，且不动用户的过滤条件）。
   */
  async function scrollFollowedTarget(target: string): Promise<boolean> {
    const treePane = sftpTreePane.value;
    if (!treePane) return false;
    if (await treePane.scrollPathToTop(target)) return true;
    await nextTick();
    if (await treePane.scrollPathToTop(target)) return true;
    if (sftpFiltersActive.value && findFileTreeNode(sftpTreeRoot.value, target)) {
      // 目标已在树里、只是被搜索/类型过滤隐藏：清掉过滤再滚一次（同时清
      // 搜索草稿，避免输入框显示旧关键词并在下次键入时重新过滤）。目标本身
      // 不在树里（例如未显示隐藏目录）时保留用户过滤条件，不做无意义清空。
      clearSftpSearch();
      sftpTypeFilter.value = "all";
      await nextTick();
      return await treePane.scrollPathToTop(target);
    }
    return false;
  }

  /** 目录跟随：面板收起时保持旧语义（只同步当前目录数据），面板打开时再揭示树。
   *  在途 guard 合并 OSC 7 / 633 / SetUserVar 同 tick 触发的重复跟进。 */
  let followSftpBusy = false;
  let pendingFollowPath = "";
  async function followSftpPath(path: string): Promise<void> {
    if (!followDirectory.value || !session.value || isLocalMode.value) return;
    const target = normalizeRemotePath(path);
    if (!sftpPaneOpen.value) {
      if (currentPath.value !== target) await loadDirectory(target, true);
      return;
    }
    if (followSftpBusy) {
      if (target !== pendingFollowPath) pendingFollowPath = target;
      return;
    }
    followSftpBusy = true;
    try {
      if (currentPath.value !== target) await loadDirectory(target, true);
      // 跟随后台揭示失败已经有 #209 软提示（loadDirectory 静默档），不再叠全局横幅。
      await revealSftpPath(target, { quiet: true });
      // 跟随目标固定显示在树顶部：祖先全部展开后按行号直接滚到顶，避免目标
      // 落在可视区底部/视口外（虚拟列表不会自动 scrollIntoView）。
      await nextTick();
      await scrollFollowedTarget(target);
    } finally {
      followSftpBusy = false;
      const next = pendingFollowPath;
      pendingFollowPath = "";
      if (next && next !== target && followDirectory.value) void followSftpPath(next);
    }
  }

  /** 单击树行文件夹只做展开/收起：懒加载子项走 listSftpDirectory +
   *  applyFileTreeChildren，绝不修改 currentPath/entries/选中（进入目录只由
   *  双击/Enter 触发，见 openSftpTreeNode）。失败回退展开态。
   *  与 reveal 共用 epoch/session 守卫：列表返回时若树已重建、会话已切换或
   *  节点已被父级刷新替换，迟到结果直接丢弃，不向新树写旧子项。 */
  async function toggleSftpTreeNode(node: SftpFileTreeNode): Promise<void> {
    if (node.kind !== "directory" || node.loading) return;
    if (node.expanded) {
      node.expanded = false;
      return;
    }
    node.expanded = true;
    if (node.loaded) return;
    node.loading = true;
    const epoch = sftpTreeRebuildSeq;
    const sessionId = session.value?.sessionId;
    const requestIsCurrent = () => isSftpTreeRequestCurrent(
      { epoch, sessionId, node },
      { epoch: sftpTreeRebuildSeq, sessionId: session.value?.sessionId, root: sftpTreeRoot.value },
    );
    try {
      const listing = await listSftpDirectory(node.path);
      if (!requestIsCurrent()) {
        node.expanded = false;
        return;
      }
      const applied = applyFileTreeChildren(sftpTreeRoot.value, node.path, listing, sftpShowHidden.value);
      if (!applied) node.expanded = false;
    } catch (cause) {
      node.expanded = false;
      if (requestIsCurrent()) showError(cause);
    } finally {
      node.loading = false;
    }
  }

  /** 双击/Enter 文件夹：进入该目录（loadDirectory 同时更新 currentPath/entries
   *  并清空选择，子树由 entries watcher 落地）。 */
  async function openSftpTreeNode(node: SftpFileTreeNode): Promise<void> {
    if (node.kind === "directory") {
      node.expanded = true;
      await loadDirectory(node.path);
      return;
    }
    // 树内文件打开走终端上方的编辑器页签；右键「预览」仍走 openEntry。
    await openEditorTab(node.entry);
  }

  function commitTreeRename(node: SftpFileTreeNode, value: string): void {
    setRenameDraft(value);
    commitRenameEntry(node.entry);
  }

  // loadDirectory 落地后同步对应树节点；applyFileTreeChildren 只写 node，不改
  // currentPath/entries，因此不会触发本 watcher 自循环。只监听 entries：路径栏
  // 提交会先改 currentPath，若同时监听会在响应回来前拿旧列表挂到新节点。
  watch(entries, (listing) => {
    if (!session.value) return;
    const path = currentPath.value;
    if (!listingBelongsToPath(listing, path)) return;
    const node = findFileTreeNode(sftpTreeRoot.value, path);
    if (!node) {
      // 路径栏/书签/历史跳到尚未展开的深层目录：按当前列表补建祖先并揭示。
      // 树尚未初始化时交给 initializeSftpTree，避免首连重复列根目录。
      if (sftpTreeRoot.value.loaded) void revealSftpPath(path);
      return;
    }
    if (node.kind !== "directory") return;
    applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
    node.expanded = true;
    node.loaded = true;
    node.loading = false;
  });

  // 隐藏项显隐：重建展开分支（本地 rebuilt 后旧 loaded 全部失效，避免假展开）。
  watch(sftpShowHidden, () => {
    void rebuildSftpTree();
  });

  // 目录跟随只由真实 cwd 变化驱动：主终端（terminalCwd）与任意分屏页签
  // （onShellTabCwd，已按页签去重）。窗格聚焦切换只更新 activeTerminalPaneId，
  // 供重新打开面板时选初始目标，不再把 SFTP 反复带回各窗格目录。
  watch(followDirectory, (follow) => {
    if (!follow || !connected.value) return;
    if (activePaneCwd.value) void followSftpPath(activePaneCwd.value);
  });

  return {
    listSftpDirectory,
    loadSftpDetails,
    clearSftpDetailCache: () => sftpDetailCache.clear(),
    revealSftpPath,
    refreshExpandedSftpTree,
    refreshSftpAfterMutation,
    initializeSftpTree,
    followSftpPath,
    toggleSftpTreeNode,
    openSftpTreeNode,
    commitTreeRename,
    resetSftpTree,
  };
}
