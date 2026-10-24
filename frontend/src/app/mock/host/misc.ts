/**
 * 杂项域夹具：ssh/alert/triage（告警排查）、sudo/profiles、ssh/knownHosts、
 * keys/discover。请求形状镜像 sidecar 契约，供走查弹窗无手填演示。
 */

import { UNHANDLED, type Unhandled } from "./shared";

export async function handleMiscInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/alert/triage") {
    // 镜像真实 sidecar 契约形状（normalized/category/suggestions + purposeKey），
    // 简化分类：关键词命中哪个组回哪组，兜底 generic；命令清单为只读诊断面。
    const payload = String((params as Record<string, unknown>)?.payload || "").trim();
    let parsed: Record<string, unknown> = {};
    try { parsed = JSON.parse(payload) as Record<string, unknown>; } catch { /* 纯文本回退 */ }
    const text = [parsed.title, parsed.message, payload].filter(Boolean).join(" ").toLowerCase();
    const category =
      text.includes("oom") || text.includes("out of memory") ? "oom" :
      text.includes("inode") ? "inode" :
      text.includes("cpu") || text.includes("负载") ? "cpu" :
      text.includes("memory") || text.includes("内存") ? "memory" :
      text.includes("disk") || text.includes("磁盘") || text.includes("空间") ? "disk" :
      text.includes("network") || text.includes("丢包") ? "network" :
      text.includes("service") || text.includes("systemd") ? "service" : "generic";
    const severityRaw = String(parsed.severity || "unknown").toLowerCase();
    // severity/source 均小写化，镜像后端 normalize（alert_triage::normalize）。
    return {
      normalized: {
        alertId: String(parsed.alertId || ""),
        title: String(parsed.title || ""),
        message: String(parsed.message || payload),
        severity: severityRaw,
        source: String(parsed.source || "").toLowerCase(),
        dataJson: parsed.data && typeof parsed.data === "object" ? JSON.stringify(parsed.data, null, 2) : "",
      },
      category,
      suggestions: category === "disk"
        ? [{ command: "df -h", purposeKey: "diskUsage" }, { command: "du -x -d 1 / | sort -rh | head -15", purposeKey: "diskDu" }]
        : [{ command: "uptime", purposeKey: "loadSnapshot" }, { command: "free -m", purposeKey: "memFree" }, { command: "df -h", purposeKey: "diskUsage" }],
    };
  }
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
