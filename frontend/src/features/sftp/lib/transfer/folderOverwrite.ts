// 整文件夹覆盖判定的纯函数层：一次上传（拖拽/选择/宿主桥）可能带一个或
// 多个顶层文件夹（多根拖拽），覆盖确认按「根目录」粒度判定——用户对每个
// 已存在的根只确认一次，接受则整目录递归删除后再走归档/逐文件车道。
//
// 根名必须是一个安全段：拒绝 ""/"."/".."/"/"/含分隔符；相对路径先做与
// folderUpload 同款的段归一（反斜杠、空段、`.`、`..` 回弹），`..` 只回弹
// 栈顶、栈空即忽略，因此归一结果永远落在 base 内。零 UI / 零 RPC 依赖。

/** 相对路径 → 段序列（与 buildFolderUploadPlan 的归一语义一致）。 */
function relativeSegments(relativePath: string): string[] {
  const raw = String(relativePath ?? "").trim();
  if (!raw) return [];
  const segments: string[] = [];
  for (const part of raw.replace(/\\/g, "/").split("/")) {
    const segment = part.trim();
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments;
}

/** 根段是否可安全拼到 base 之下（单段、非相对特殊名、非绝对）。 */
function isSafeRoot(root: string): boolean {
  return root.length > 0
    && root !== "."
    && root !== ".."
    && root !== "/"
    && !root.includes("/")
    && !root.includes("\\")
    && !root.includes("\0");
}

/**
 * 单条相对路径的顶层根目录名：没有目录段（裸文件）或根不合法时返回
 * undefined。与 folderUploadRootPaths 共用同一套归一规则。
 */
export function folderUploadRootPath(relativePath: string): string | undefined {
  const segments = relativeSegments(relativePath);
  if (segments.length < 2) return undefined;
  const root = segments[0];
  return isSafeRoot(root) ? root : undefined;
}

/**
 * 从文件夹上传条目提取顶层根目录名：去重、归一、过滤非法值、字典序排序。
 * 裸文件（没有任何目录段，如 `a.txt`）不产生根——它们本就落在 base 下，
 * 不能把文件名当成「待覆盖目录」。
 */
export function folderUploadRootPaths(entries: readonly { relativePath: string }[]): string[] {
  const roots = new Set<string>();
  for (const entry of entries) {
    const root = folderUploadRootPath(entry.relativePath);
    if (root) roots.add(root);
  }
  return [...roots].sort((left, right) => (left < right ? -1 : left > right ? 1 : 0));
}

/**
 * 根目录的远端目标路径：`joinRemote(baseDir, root)`。根不合法（空/dot/
 * dotdot/绝对/多段）时返回空串，调用方据此跳过该根（绝不允许越出 base）。
 */
export function folderOverwriteTarget(baseDir: string, root: string, joinRemote: (parent: string, name: string) => string): string {
  const safeRoot = String(root ?? "").trim();
  if (!isSafeRoot(safeRoot)) return "";
  const base = String(baseDir ?? "").trim() || "/";
  return joinRemote(base, safeRoot);
}
