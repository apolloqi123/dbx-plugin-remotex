/**
 * 远端路径纯函数（URI/路径规范化、拼接、父目录）：工作台 SFTP 各域共用。
 * 无状态、无依赖，单独成文件便于 composable 与 App 共同引用而不引入顺序问题。
 */
export function pathFromUri(uri: string) {
  return uri.replace(/^sftp:/, "") || "/";
}

export function normalizeRemotePath(path: string) {
  let value = path.trim() || "/";
  try { value = decodeURIComponent(value); } catch {}
  if (!value.startsWith("/")) value = `/${value}`;
  value = value.replace(/\/{2,}/g, "/");
  return value === "/" ? value : value.replace(/\/+$/, "");
}

export function joinRemote(parent: string, name: string) {
  return `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/${name.replace(/^\/+/, "")}`;
}

export function parentPath(path: string) {
  const normalized = path.replace(/\/+$/, "");
  const index = normalized.lastIndexOf("/");
  return index <= 0 ? "/" : normalized.slice(0, index);
}
