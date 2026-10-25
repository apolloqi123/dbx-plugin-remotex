// SFTP 文件树右键菜单的纯决策：选区收敛与受保护目录判定。抽成无 Vue
// 依赖的小模块，便于 App.vue 复用与单测锁定行为。
import { resolveRemotePath } from "../paths/remotePathInput";

/**
 * 右键命中的行不在当前选区（或当前无选区）时，把选区整体替换为该行，
 * 避免菜单里的批量动作继续作用于右键前的旧选区；命中行已在选区内
 * （含多选）时保持原选区，符合主流文件管理器语义。
 * 返回 null 表示无需改动，避免给 selectedUris 赋同值触发多余更新。
 */
export function resolveSftpContextMenuSelection(
  clickedUri: string,
  selectedUris: readonly string[],
): string[] | null {
  if (selectedUris.includes(clickedUri)) return null;
  return [clickedUri];
}

/**
 * 受保护目录：根目录、当前目录，以及当前目录的任一祖先目录。
 * 这些目录被执行改名/移动/删除/改权限/归档会让正在浏览的面板状态失效，
 * 或把用户当前所在分支整体移走，因此菜单与选中入口都要拦截。
 * 只按目录语义使用；调用方先判断 entry.kind。
 */
export function isProtectedSftpDirectoryPath(path: string, currentPath: string): boolean {
  const target = resolveRemotePath(path);
  if (target === "/") return true;
  const current = resolveRemotePath(currentPath);
  return current === target || current.startsWith(`${target}/`);
}

interface SftpSelectableEntry {
  uri: string;
  kind: string;
}

/**
 * 单条选中资格：文件/符号链接等非目录条目始终可选；目录只有不受保护时
 * 才可进入多选。受保护目录可以在右键菜单里预览/复制/下载，但不能成为
 * 批量删除/归档/下载的目标。
 */
export function isSelectableSftpEntry(
  entry: SftpSelectableEntry,
  currentPath: string,
  pathFromUri: (uri: string) => string,
): boolean {
  return entry.kind !== "directory" || !isProtectedSftpDirectoryPath(pathFromUri(entry.uri), currentPath);
}

/** 选中条目过滤器（selectedEntries 的兜底不变量）：剔除根/当前/祖先目录，
 *  保留文件、符号链接与普通目录，顺序不变。 */
export function filterSelectableSftpEntries<T extends SftpSelectableEntry>(
  entries: readonly T[],
  currentPath: string,
  pathFromUri: (uri: string) => string,
): T[] {
  return entries.filter((entry) => isSelectableSftpEntry(entry, currentPath, pathFromUri));
}

/** URI 选区过滤器（selectedUris 入口收敛）：调用方提供已加载目录 URI 集合，
 *  未知 URI 按非目录保留，避免把文件选择误删。 */
export function filterSelectableSftpUris(
  uris: readonly string[],
  currentPath: string,
  pathFromUri: (uri: string) => string,
  isDirectoryUri: (uri: string) => boolean,
): string[] {
  return uris.filter((uri) =>
    isSelectableSftpEntry(
      { uri, kind: isDirectoryUri(uri) ? "directory" : "file" },
      currentPath,
      pathFromUri,
    ),
  );
}
