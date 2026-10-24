/**
 * AI 宿主桥夹具（host.ai.*，Warp AI 对齐批）：记录会话/生成/推荐位请求供
 * smoke 走查断言，并按 prompt 任务分流罐头（面板生成 / 失败修复）。
 */

import { UNHANDLED, type Unhandled } from "./shared";

export const aiConversations: Array<Record<string, unknown>> = [];
export const aiGenerations: Array<Record<string, unknown>> = [];
const aiRecommendations: Array<Record<string, unknown>> = [];
let aiRecommendationClears = 0;

export function getAiRecommendationStats(): { pushes: number; clears: number } {
  return { pushes: aiRecommendations.length, clears: aiRecommendationClears };
}

/** host.ai.* 请求分发（其余 request 方法由父入口处理）。 */
export function handleAiHostRequest(method: string, params?: unknown): unknown | Unhandled {
  // host.ai.openConversation（Warp AI 对齐批）：记录请求供 smoke 断言
  // （title/prompt/context 与真实宿主 createPluginAiConversation 同形入参）。
  if (method === "host.ai.openConversation") {
    aiConversations.push((params ?? {}) as Record<string, unknown>);
    return null;
  }
  // 直连生成 + 推荐位（Warp AI 对齐批跟进）：记录请求供 smoke 断言；生成
  // 回罐头「命令 + Why」两行，prompt 原样记录（断言 triage 提示位与脱敏）。
  if (method === "host.ai.listModels") {
    return [
      { configId: "mock-config", name: "Mock provider", model: "mock-fast-model", isDefault: true },
    ];
  }
  if (method === "host.ai.generateText") {
    const input = (params ?? {}) as { prompt?: string };
    aiGenerations.push(input);
    // 按任务分流罐头（AI 体验改造 v2）：# 搜索 prompt 带「User request:」标记
    // → 回 ls -S；失败修复 prompt 带「Diagnose the failure」标记 → 回 df -h
    // （既有修复走查断言依赖）。两罐头同守「首行命令 + Why 行」输出契约。
    const prompt = typeof input.prompt === "string" ? input.prompt : "";
    const can = prompt.includes("User request:")
      ? "ls -S\nWhy: sorts by size, largest first."
      : "df -h /srv\nWhy: the deploy failed because the disk filled up; check free space first.";
    // 400ms 延迟：等待态提示（loading 条 + 输入冻结）在走查里可观测。
    return new Promise((resolve) => setTimeout(() => resolve(can), 400));
  }
  if (method === "host.ai.setRecommendations") {
    aiRecommendations.push((params ?? {}) as Record<string, unknown>);
    return null;
  }
  if (method === "host.ai.clearRecommendations") {
    aiRecommendationClears += 1;
    return null;
  }
  return UNHANDLED;
}
