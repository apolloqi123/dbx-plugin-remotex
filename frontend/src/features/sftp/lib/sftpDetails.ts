// 文件详情卡的数据模型与纯函数：tree/list 条目只带 size/modifiedAt/
// permissions 等基础字段，owner/group 在树列表里是关闭的（includeOwner:
// false），因此详情由父级通过 loadDetails 懒加载。这里的合并规则保持
// 「stat 值优先 → ownerName/groupName 兜底 → entry 自带值 → fallbackPath」，
// 渲染行只负责本地化与格式化，方便组件与父级共用同一套语义。
import { formatBytes } from "../../../shared/lib/format";
import type { SftpEntryKind, SftpSanitizedEntry } from "./sftpEntries";

export interface SftpDetailInfo {
  path: string;
  kind: SftpEntryKind;
  size?: number;
  modifiedAt?: number;
  permissions?: string;
  owner?: string;
  group?: string;
  ownerName?: string | null;
  groupName?: string | null;
}

export interface SftpDetailRow {
  key: string;
  label: string;
  value: string;
}

const PLACEHOLDER = "—";
const DATE_OPTIONS: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

function textOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

/** entry.uri（sftp:/abs/path）→ 绝对路径；不可解析时返回空串交给 fallback。 */
function pathFromUri(entry: SftpSanitizedEntry): string {
  if (typeof entry.uri !== "string") return "";
  const bare = entry.uri.startsWith("sftp:") ? entry.uri.slice("sftp:".length) : entry.uri;
  return bare.startsWith("/") ? bare : "";
}

/** 仅使用 entry 自带字段的详情；树节点路径作为权威的 fallbackPath。 */
export function sftpDetailFromEntry(entry: SftpSanitizedEntry, fallbackPath: string): SftpDetailInfo {
  const detail: SftpDetailInfo = {
    path: fallbackPath || pathFromUri(entry),
    kind: entry.kind,
  };
  const size = numberOrUndefined(entry.size);
  if (size !== undefined) detail.size = size;
  const modifiedAt = numberOrUndefined(entry.modifiedAt);
  if (modifiedAt !== undefined) detail.modifiedAt = modifiedAt;
  const permissions = textOrUndefined(entry.permissions);
  if (permissions !== undefined) detail.permissions = permissions;
  const owner = textOrUndefined(entry.owner);
  if (owner !== undefined) detail.owner = owner;
  const group = textOrUndefined(entry.group);
  if (group !== undefined) detail.group = group;
  return detail;
}

function nameValue(value: string | null | undefined): string | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  return textOrUndefined(value);
}

/**
 * stat（sftp/stat）结果合并进 entry 详情：stat 的每个字段存在即覆盖；
 * owner/group 缺失时依次回退到 stat.ownerName/groupName 与 entry 自身；
 * path 始终以 stat.path 或 fallbackPath 为准。
 */
export function mergeSftpDetail(
  entry: SftpSanitizedEntry,
  /** sftp/stat 以 `mode` 返回权限串；详情模型统一用 `permissions`。 */
  stat: (Partial<SftpDetailInfo> & { mode?: string }) | null | undefined,
  fallbackPath: string,
): SftpDetailInfo {
  const detail = sftpDetailFromEntry(entry, fallbackPath);
  if (!stat) return detail;

  const path = textOrUndefined(stat.path);
  if (path !== undefined) detail.path = path;
  if (stat.kind) detail.kind = stat.kind;

  const size = numberOrUndefined(stat.size);
  if (size !== undefined) detail.size = size;
  const modifiedAt = numberOrUndefined(stat.modifiedAt);
  if (modifiedAt !== undefined) detail.modifiedAt = modifiedAt;
  const permissions = textOrUndefined(stat.permissions) ?? textOrUndefined(stat.mode);
  if (permissions !== undefined) detail.permissions = permissions;

  const statOwner = textOrUndefined(stat.owner);
  const statOwnerName = textOrUndefined(stat.ownerName);
  const owner = statOwner ?? statOwnerName;
  if (owner !== undefined) detail.owner = owner;
  const statGroup = textOrUndefined(stat.group);
  const statGroupName = textOrUndefined(stat.groupName);
  const group = statGroup ?? statGroupName;
  if (group !== undefined) detail.group = group;

  const ownerName = nameValue(stat.ownerName);
  if (ownerName !== undefined) detail.ownerName = ownerName;
  const groupName = nameValue(stat.groupName);
  if (groupName !== undefined) detail.groupName = groupName;

  return detail;
}

function formatModified(modifiedAt: number | undefined, locale: string): string {
  if (modifiedAt === undefined || !Number.isFinite(modifiedAt)) return PLACEHOLDER;
  // sidecar 的 sftp/list、sftp/stat 都以秒返回 mtime；宿主桥/测试数据可能
  // 已是毫秒。以 1e12 为界兼容两种单位（1970-2001 的秒值不会误判）。
  const milliseconds = Math.abs(modifiedAt) < 1_000_000_000_000 ? modifiedAt * 1000 : modifiedAt;
  try {
    return new Intl.DateTimeFormat(locale || undefined, DATE_OPTIONS).format(milliseconds);
  } catch {
    // 宿主传入结构非法的 locale 时退回系统默认，详情卡不应因此整块报错。
    return new Intl.DateTimeFormat(undefined, DATE_OPTIONS).format(milliseconds);
  }
}

function kindLabel(kind: SftpEntryKind, t: (key: string) => string): string {
  return t(`sftpDetails.${kind}`);
}

function ownerLabel(detail: SftpDetailInfo): string {
  return detail.owner ?? detail.ownerName ?? PLACEHOLDER;
}

function groupLabel(detail: SftpDetailInfo): string {
  return detail.group ?? detail.groupName ?? PLACEHOLDER;
}

/** 详情卡行（顺序固定），纯格式化，便于快照/边界测试。 */
export function sftpDetailRows(
  detail: SftpDetailInfo,
  t: (key: string) => string,
  locale: string,
): SftpDetailRow[] {
  return [
    {
      key: "path",
      label: t("sftpDetails.path"),
      value: detail.path || PLACEHOLDER,
    },
    {
      key: "type",
      label: t("sftpDetails.type"),
      value: kindLabel(detail.kind, t),
    },
    {
      key: "size",
      label: t("sftpDetails.size"),
      value: numberOrUndefined(detail.size) === undefined ? PLACEHOLDER : formatBytes(detail.size),
    },
    {
      key: "modified",
      label: t("sftpDetails.modified"),
      value: formatModified(detail.modifiedAt, locale),
    },
    {
      key: "permissions",
      label: t("sftpDetails.permissions"),
      value: textOrUndefined(detail.permissions) ?? PLACEHOLDER,
    },
    {
      key: "owner",
      label: t("sftpDetails.owner"),
      value: ownerLabel(detail),
    },
    {
      key: "group",
      label: t("sftpDetails.group"),
      value: groupLabel(detail),
    },
  ];
}
