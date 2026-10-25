// 连接元数据的显示文本消毒：宿主给的字段可能是真的 null，也可能是序列化后的
// `"null"`/`"undefined"`，这里只保留有意义的字符串。纯函数，无 feature 依赖。

/**
 * Normalize host-provided display text. DBX connection metadata can contain
 * actual nulls as well as serialized values such as `"null"`/`"undefined"`.
 * Treat those and whitespace-only values as missing so they never leak into
 * the toolbar or connection-info popover.
 */
export function normalizeConnectionText(value: unknown): string {
  if (typeof value !== "string") return "";
  const normalized = value.trim();
  return normalized && normalized.toLowerCase() !== "null" && normalized.toLowerCase() !== "undefined" ? normalized : "";
}
