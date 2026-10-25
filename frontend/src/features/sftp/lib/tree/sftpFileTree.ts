// SFTP 单栏文件树模型（文件夹 + 文件同树）：懒加载节点的纯函数状态机，
// 不依赖 Vue，便于 App.vue 用 reactive 根节点驱动渲染。目录/文件排序、
// 隐藏项过滤、刷新时保留展开状态与过滤/扁平化都在这里完成。
import type { SftpEntryKind, SftpSanitizedEntry } from "../entries/sftpEntries";
import { isHiddenEntry } from "../entries/sftpFileFilters";

export interface SftpFileTreeNode {
  path: string;
  name: string;
  kind: SftpEntryKind;
  entry: SftpSanitizedEntry;
  expanded: boolean;
  loaded: boolean;
  loading: boolean;
  children: SftpFileTreeNode[];
}

export interface SftpTreeFilter {
  search?: string;
  type?: "all" | "directory" | "file";
}

export interface SftpTreeRow {
  node: SftpFileTreeNode;
  depth: number;
}

/** 条目 uri（sftp:/abs/path）→ 树节点使用的绝对路径。 */
function pathFromEntry(entry: SftpSanitizedEntry): string {
  const bare = entry.uri.startsWith("sftp:") ? entry.uri.slice("sftp:".length) : entry.uri;
  if (bare.startsWith("/")) return bare || "/";
  return `/${bare || entry.name}`;
}

function entryForPath(path: string): SftpSanitizedEntry {
  return { name: "/", uri: path === "/" ? "sftp:/" : `sftp:${path}`, kind: "directory" };
}

/**
 * 根节点（默认连接根目录 "/"）；name 固定 "/"，展示层直接显示为 "/"。
 * uri 保持 sftp:/ 前缀，与 sftp/list 的条目 uri 同构。
 */
export function createFileTreeRoot(path = "/"): SftpFileTreeNode {
  const normalized = path || "/";
  return {
    path: normalized,
    name: "/",
    kind: "directory",
    entry: entryForPath(normalized),
    expanded: false,
    loaded: false,
    loading: false,
    children: [],
  };
}

/** 按路径精确查找节点（树规模有限，DFS 足够）。 */
export function findFileTreeNode(root: SftpFileTreeNode, path: string): SftpFileTreeNode | null {
  if (root.path === path) return root;
  for (const child of root.children) {
    const hit = findFileTreeNode(child, path);
    if (hit) return hit;
  }
  return null;
}

function createNode(entry: SftpSanitizedEntry): SftpFileTreeNode {
  return {
    path: pathFromEntry(entry),
    name: entry.name,
    kind: entry.kind,
    entry,
    expanded: false,
    loaded: false,
    loading: false,
    children: [],
  };
}

function compareNodes(left: SftpFileTreeNode, right: SftpFileTreeNode): number {
  const leftDirectory = left.kind === "directory";
  const rightDirectory = right.kind === "directory";
  if (leftDirectory !== rightDirectory) return leftDirectory ? -1 : 1;
  return left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
}

/**
 * sftp/list 结果 → 子节点：文件夹和文件都进树；默认过滤隐藏项；目录优先，
 * 同类按名称做 numeric-aware locale 排序（file2 在 file10 前）。
 */
export function buildFileTreeChildren(
  entries: readonly SftpSanitizedEntry[],
  showHidden = false,
): SftpFileTreeNode[] {
  return entries
    .filter((entry) => showHidden || !isHiddenEntry(entry.name))
    .map(createNode)
    .sort(compareNodes);
}

/**
 * 挂载某个目录的子节点并展开它；父节点已被刷新掉时返回 null。
 * 同一路径刷新时保留仍存在的目录子节点的展开状态与已加载子树，避免
 * 刷新根目录后所有展开分支被收起。
 */
export function applyFileTreeChildren(
  root: SftpFileTreeNode,
  path: string,
  entries: readonly SftpSanitizedEntry[],
  showHidden = false,
): SftpFileTreeNode | null {
  const parent = findFileTreeNode(root, path);
  if (!parent) return null;

  const previous = new Map(parent.children.map((child) => [child.path, child]));
  parent.children = buildFileTreeChildren(entries, showHidden).map((child) => {
    const old = previous.get(child.path);
    // 只有真正加载完成的旧目录才保留展开态与子树；stale 后重拉时宁可
    // 收起分支，避免出现 expanded=true 但 children 为空的假展开节点。
    if (old && old.kind === "directory" && child.kind === "directory" && old.loaded) {
      child.expanded = old.expanded;
      child.loaded = true;
      child.children = old.children;
    }
    return child;
  });
  parent.loaded = true;
  parent.loading = false;
  parent.expanded = true;
  return parent;
}

/** 整棵树标记为待重拉（保留结构与展开状态）。 */
export function markFileTreeStale(root: SftpFileTreeNode): void {
  root.loaded = false;
  for (const child of root.children) markFileTreeStale(child);
}

/**
 * 把「已加载但折叠」的目录标记为待重拉：变更后的 refreshExpandedSftpTree
 * 只重列展开分支，折叠分支不会收到新列表；若不标记，重新展开会直接复用
 * 旧 children（保存/改名后看到过期内容）。不在这里清空 children——折叠
 * 期间批量计数/选中仍以已加载节点为准；下次展开时 loaded=false 走重列，
 * applyFileTreeChildren 会用新列表整体替换，也就不会保留 stale 子树。
 * 比「重列全部已加载节点」少一轮网络请求，是本场景更便宜的等价修复。
 */
export function markCollapsedTreeNodesStale(root: SftpFileTreeNode): void {
  if (root.kind === "directory" && root.loaded && !root.expanded) root.loaded = false;
  for (const child of root.children) markCollapsedTreeNodesStale(child);
}

/**
 * 树异步请求的落地守卫：epoch 在树重建（显隐切换/会话重置）时递增，
 * 会话切换后旧响应一律作废；节点还必须仍是当前根上的同一个对象，
 * 防止父级刷新替换节点后把迟到子项挂到新节点/旧根上。
 */
export function isSftpTreeRequestCurrent(
  request: { epoch: number; sessionId: string | undefined; node: SftpFileTreeNode },
  current: { epoch: number; sessionId: string | undefined; root: SftpFileTreeNode },
): boolean {
  if (!request.sessionId || request.sessionId !== current.sessionId) return false;
  if (request.epoch !== current.epoch) return false;
  return findFileTreeNode(current.root, request.node.path) === request.node;
}

function typeMatches(kind: SftpEntryKind, type: SftpTreeFilter["type"]): boolean {
  if (!type || type === "all") return true;
  if (type === "directory") return kind === "directory";
  // 与现有列表过滤一致：Files 只含常规文件。
  return kind === "file";
}

function nameMatches(node: SftpFileTreeNode, query: string): boolean {
  return !query || node.name.toLowerCase().includes(query);
}

function hasMatchingDescendant(node: SftpFileTreeNode, query: string, type: SftpTreeFilter["type"]): boolean {
  return node.children.some(
    (child) =>
      (typeMatches(child.kind, type) && nameMatches(child, query)) ||
      hasMatchingDescendant(child, query, type),
  );
}

/**
 * 展开可见行：只下钻 expanded 目录。
 * - 文件：类型 + 搜索都命中才显示。
 * - 目录：自身命中、名称命中搜索、或任一已加载后代命中时显示，作为
 *   匹配项的祖先上下文；因此 type=file 时仍能看到通向文件的目录。
 * 搜索为空时目录自身视为命中（目录/全部类型），file 类型下则只保留
 * 确有文件后代的目录。
 */
export function flattenVisibleTreeRows(
  root: SftpFileTreeNode,
  filter: SftpTreeFilter = {},
): SftpTreeRow[] {
  const query = filter.search?.trim().toLowerCase() ?? "";
  const type = filter.type ?? "all";
  const rows: SftpTreeRow[] = [];

  function isRowVisible(node: SftpFileTreeNode): boolean {
    const selfMatches = typeMatches(node.kind, type) && nameMatches(node, query);
    if (node.kind !== "directory") return selfMatches;
    if (selfMatches) return true;
    if (query && nameMatches(node, query)) return true;
    return hasMatchingDescendant(node, query, type);
  }

  function walk(node: SftpFileTreeNode, depth: number): void {
    if (!isRowVisible(node)) return;
    rows.push({ node, depth });
    if (node.kind !== "directory" || !node.expanded) return;
    for (const child of node.children) walk(child, depth + 1);
  }

  walk(root, 0);
  return rows;
}
