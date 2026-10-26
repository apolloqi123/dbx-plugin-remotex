// 容器内文件路径与列表归一（纯逻辑，无 Vue 依赖）。
//
// 规则与 sidecar `features/docker/files` 同口径：绝对路径、折叠 `.`/`..`、
// 去掉重复分隔符与尾部斜杠；根固定为 `/`。列表响应做防御性归一：非数组、
// 非对象、缺 name 的条目一律丢弃；`kind` 只认 `dir`，其余按文件展示（与
// 后端「尾部 `/` 判定目录、其余一律按文件」一致）。RPC 调用留在
// `composables/useContainerFiles`。

export type ContainerFileKind = "dir" | "file";

export interface ContainerFileEntry {
  name: string;
  path: string;
  kind: ContainerFileKind;
}

export interface ContainerFileList {
  path: string;
  entries: ContainerFileEntry[];
}

export interface ContainerFileCrumb {
  name: string;
  path: string;
}

/** 绝对化 + 折叠 `.`/`..`；空输入与根都归一为 `/`。 */
export function normalizeContainerPath(input: string): string {
  const segments: string[] = [];
  for (const segment of String(input ?? "").split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return `/${segments.join("/")}`;
}

/** 父目录；根本身返回 `/`。 */
export function parentPath(input: string): string {
  const normalized = normalizeContainerPath(input);
  if (normalized === "/") return "/";
  const index = normalized.lastIndexOf("/");
  return index <= 0 ? "/" : normalized.slice(0, index);
}

/** 目录 + 名称 → 归一后的绝对路径。 */
export function joinContainerPath(dir: string, name: string): string {
  return normalizeContainerPath(`${dir}/${name}`);
}

/** 目录优先、名称升序（大小写敏感，与后端排序一致）。 */
export function sortEntries(entries: readonly ContainerFileEntry[]): ContainerFileEntry[] {
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
  });
}

/** 面包屑：根 + 每级祖先（含当前目录），逐级可点击。 */
export function breadcrumbs(path: string): ContainerFileCrumb[] {
  const normalized = normalizeContainerPath(path);
  const crumbs: ContainerFileCrumb[] = [{ name: "/", path: "/" }];
  if (normalized === "/") return crumbs;
  let current = "";
  for (const segment of normalized.split("/").filter(Boolean)) {
    current += `/${segment}`;
    crumbs.push({ name: segment, path: current });
  }
  return crumbs;
}

/** `docker/files/list` 响应归一：路径回落请求值，条目逐条校验。 */
export function containerFileListPayload(raw: unknown, requestedPath: string): ContainerFileList {
  const record = (raw ?? {}) as Record<string, unknown>;
  const path = normalizeContainerPath(
    typeof record.path === "string" && record.path ? record.path : requestedPath,
  );
  const rows = Array.isArray(record.entries) ? record.entries : [];
  const entries: ContainerFileEntry[] = [];
  for (const row of rows) {
    if (!row || typeof row !== "object") continue;
    const entry = row as Record<string, unknown>;
    if (typeof entry.name !== "string" || !entry.name) continue;
    const kind: ContainerFileKind = entry.kind === "dir" ? "dir" : "file";
    const entryPath =
      typeof entry.path === "string" && entry.path.startsWith("/")
        ? normalizeContainerPath(entry.path)
        : joinContainerPath(path, entry.name);
    entries.push({ name: entry.name, path: entryPath, kind });
  }
  return { path, entries: sortEntries(entries) };
}

/** 容器缺 `ls`（distroless）时后端把 stderr 原样带回：UI 换成可操作文案。 */
export function isMissingLsError(message: string): boolean {
  const lower = String(message ?? "").toLowerCase();
  return (
    lower.includes("executable file not found") ||
    lower.includes("ls: not found") ||
    lower.includes("ls: command not found") ||
    lower.includes('exec: "ls"')
  );
}
