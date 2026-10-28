/**
 * 杂项域夹具：sudo/profiles、ssh/knownHosts、keys/discover。请求形状镜像
 * sidecar 契约，供走查弹窗无手填演示。
 */

import { UNHANDLED, type Unhandled } from "./shared";

export async function handleMiscInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "sudo/profiles/list") return { profiles: [] };
  if (method === "sudo/profiles/reveal") return { profile: {} };
  if (method === "sudo/profiles/save" || method === "sudo/profiles/delete") return { success: true };
  if (method === "ssh/knownHosts/list") return { entries: [] };
  if (method === "ssh/knownHosts/remove") {
    // 夹具 known_hosts 恒为空（list 返回空表），移除必然 0 条；仍按 sidecar
    // 校验 host 并回 {success, removed: <条数>}，参数缺失直接抛错。
    if (!String((params as Record<string, unknown>)?.host || "").trim()) throw new Error("host is required");
    return { success: true, removed: 0 };
  }
  if (method === "keys/discover") return { keys: [] };
  return UNHANDLED;
}
