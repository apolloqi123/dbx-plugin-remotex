/**
 * 可视化夹具的宿主模拟器（dev/test-only）：mock.html / visual.html 引导与本
 * 文件的单测都会直接 import 本模块，副作用是把 window.dbxPlugin 装成真实
 * 宿主桥形状。
 *
 * 实现按域拆到 mock/host/*：
 * - fixture.ts   URL 开关 / context / appearance / theme / locale
 * - shared.ts    监听器、事件与二进制投递、帧编码
 * - session.ts   ssh/session/*、connection/protocol、ssh/terminal/replay
 * - terminal.ts  ssh/exec、batchInput、ssh/terminal/in 回显与回放脚本
 * - local.ts     local/terminal/*、local/session/*、偏好/目录/指标
 * - sftpFiles.ts sftp/* 与 sudo 文件别名、书签
 * - transfers.ts sftp/transfer/*、ssh|local/recording/*
 * - tunnels.ts   ssh/forward/*
 * - docker.ts    docker/*
 * - metrics.ts   ssh/metrics、ssh/processes/*、sftp/diskUsage
 * - settings.ts  ssh/settings、快捷命令、高亮规则、审计、mcp/settings
 * - otp.ts       otp/*
 * - ai.ts        host.ai.*
 * - rdp.ts       rdp/*
 * - misc.ts      alert triage、sudo profiles、knownHosts、keys
 *
 * 参数语义（?render / ?rw / ?err / ?slow / ?fresh / ?local / ?restored /
 * ?ai / ?aipanel / ?aifix / ?surface / ?auth / ?proto / ?tunnels / ?locale /
 * ?theme / ?noanim / ?docker）见 fixture.ts 顶部注释。
 */

import { CONNECTION_PROVIDER_ID, TUNNELS_QUICK_ID, TUNNELS_WORKBENCH_ID, WORKBENCH_ID } from "../shared/lib/pluginIds";

import {
  UNHANDLED,
  appearanceListeners,
  base64,
  binaryListeners,
  contextListeners,
  emitHostEvent,
  eventListeners,
  toBytes,
} from "./mock/host/shared";
import {
  aiCompletionOn,
  aiPanelOn,
  appearance,
  context,
  fixtureParams,
  getMockLocale,
  localOnlyContext,
  setMockLocale,
  theme,
  writable,
} from "./mock/host/fixture";
import { handleSessionInvoke } from "./mock/host/session";
import { emitTerminal, handleSshTerminalInput, handleTerminalInvoke } from "./mock/host/terminal";
import { exitLocalSession, handleLocalInvoke, handleLocalTerminalInput } from "./mock/host/local";
import { handleSftpInvoke } from "./mock/host/sftpFiles";
import { handleSftpUploadBinary, handleTransfersInvoke } from "./mock/host/transfers";
import { handleTunnelsInvoke } from "./mock/host/tunnels";
import { handleDockerInvoke } from "./mock/host/docker";
import { handleMetricsInvoke } from "./mock/host/metrics";
import { handleSettingsInvoke } from "./mock/host/settings";
import { handleMiscInvoke } from "./mock/host/misc";
import { handleRdpInvoke } from "./mock/host/rdp";
import { handleOtpInvoke } from "./mock/host/otp";
import { aiConversations, aiGenerations, getAiRecommendationStats, handleAiHostRequest } from "./mock/host/ai";

const request: DbxPluginApi["request"] = async <T = unknown>(method: string, params?: unknown) => {
  if (method === "host.getContext") return context as T;
  const aiResult = handleAiHostRequest(method, params);
  if (aiResult !== UNHANDLED) return aiResult as T;
  // host.listConnections（PR-A4 扩展点）：返回夹具连接（只读无密），供面板
  // connection-switching walkthrough; on legacy host semantics unknown methods still resolve to null (caller degrades).
  if (method === "host.listConnections") {
    return {
      connections: localOnlyContext
        ? []
        : [{ id: "visual-connection", name: "Production SSH", providerId: CONNECTION_PROVIDER_ID, connectionType: "ssh", readOnly: !writable }],
    } as T;
  }
  return null as T;
};

// 各域处理器按方法名互斥；顺序与拆分前 invoke 的 if/else 语义一致（首个
// 命中者返回结果，全部未命中才落到严格化的 unimplemented 抛错）。
const invokeHandlers: Array<(method: string, params?: unknown) => Promise<unknown>> = [
  handleSessionInvoke,
  handleTerminalInvoke,
  handleSftpInvoke,
  handleTransfersInvoke,
  handleTunnelsInvoke,
  handleDockerInvoke,
  handleLocalInvoke,
  handleMetricsInvoke,
  handleSettingsInvoke,
  handleMiscInvoke,
  handleRdpInvoke,
  handleOtpInvoke,
];

const invoke: DbxPluginApi["invoke"] = async <T = unknown>(method: string, params?: unknown) => {
  for (const handler of invokeHandlers) {
    const result = await handler(method, params);
    if (result !== UNHANDLED) return result as T;
  }
  // 严格化（docs/plans/MOCK_HOST_STRICTNESS_PLAN.zh-CN.md）：未实现的方法显式
  // 报错而非静默成功——静默成功会把调用侧的方法名/参数回归吞成「成功」，
  // 让 GUI 走查与真实宿主行为系统性背离。处理三选一见
  // scripts/verify/mock_host_registry_check.mjs（实现桩 / ALLOWLIST / SKIP）。
  throw new Error(`[mockDbxHost] unimplemented method: ${method}`);
};

// Sequence number of mock host instances injected by openWorkbench (host-authority simulation, spec §11).
let mockOpenWorkbenchSeq = 0;

window.dbxPlugin = {
  ready: Promise.resolve(context),
  context,
  appearance,
  theme,
  get locale() {
    return getMockLocale();
  },
  request,
  invoke,
  notify: async (method, params) => {
    // ssh/batchBar/state：sidecar 只做跨 webview 广播（不落存储）。mock 同样
    // 回投事件，各端按 source 过滤自己的回声（单工作台夹具下即无可见变化）。
    if (method === "ssh/batchBar/state") {
      emitHostEvent({ method, params: (params ?? {}) as Record<string, unknown> });
    }
  },
  // 新建会话按钮的桥调用：mock 只记录参数（控制台可见），不真的开 tab。
  // Mirrors the §11 host authority: the plugin payload stays under context.plugin while plugin-supplied
  // reserved fields (workbenchId/restored/surface) are always dropped and the identity is injected by the mock (host)
  // generated identity is injected — an early test surface for host A1 behavior.
  openWorkbench: async (contributionId, childContext, options) => {
    console.info("[mock] openWorkbench", contributionId, childContext, options);
    const payload = (childContext && typeof childContext === "object" && !Array.isArray(childContext) ? childContext : {}) as Record<string, unknown>;
    delete payload.workbenchId;
    delete payload.restored;
    delete payload.surface;
    const hostContext = {
      ...payload,
      workbenchId: `mock-workbench-${++mockOpenWorkbenchSeq}`,
      restored: false,
      surface: "tab",
    };
    (window as unknown as { __dbxMockOpenWorkbench?: unknown[] }).__dbxMockOpenWorkbench ??= [];
    (window as unknown as { __dbxMockOpenWorkbench: unknown[] }).__dbxMockOpenWorkbench.push({ contributionId, context: hostContext });
  },
  sendBinary: async (channel, data) => {
    if (channel.startsWith("sftp/upload/")) {
      handleSftpUploadBinary(channel, toBytes(data));
      return;
    }
    if (channel.startsWith("local/terminal/in/")) {
      handleLocalTerminalInput(channel, toBytes(data));
      return;
    }
    if (!channel.startsWith("ssh/terminal/in/")) return;
    handleSshTerminalInput(toBytes(data));
  },
  onEvent: (listener) => { eventListeners.add(listener); return () => eventListeners.delete(listener); },
  onBinary: (listener) => { binaryListeners.add(listener); return () => binaryListeners.delete(listener); },
  onAppearanceChange: (listener) => { appearanceListeners.add(listener); listener(appearance); return () => appearanceListeners.delete(listener); },
  // 镜像真桥 API 面（X-P5 裁剪幽灵 API）：真桥只有 onContext（注册不立即回调，
  // 初始 context 经 api.ready / host.getContext）；locale 经 onEvent 的 env 推送。
  onContext: (listener) => { contextListeners.add(listener); return () => contextListeners.delete(listener); },
  decodeBase64: (value) => Uint8Array.from(atob(value), (character) => character.charCodeAt(0)),
  encodeBase64: base64,
  workbenchState: { set: async () => undefined },
  clipboard: { readText: async () => "", writeText: async () => undefined },
  // 宿主 host.saveFile mock（Host API 1.1，单次整包落盘桥）：与真实宿主同形——
  // fileName 去路径分隔符、整包 Uint8Array 接受、成功返回 { path }。
  saveFile: async (options, data) => {
    const bytes = typeof data === "string" ? Uint8Array.from(atob(data), (c) => c.charCodeAt(0)) : new Uint8Array(data instanceof ArrayBuffer ? data : (data as Uint8Array).buffer);
    void bytes;
    return { path: (options.fileName || "download.bin").split(/[\\/]/).pop() || "download.bin" };
  },
  fileTransfer: {
    pick: async () => ({ files: [] }),
    read: async () => ({ dataBase64: "", length: 0, eof: true }),
    beginSave: async () => ({ handleId: "visual-save-handle-0001", chunkBytes: 262144 }),
    write: async (_handleId, offset, data) => {
      // 字符串载荷为 base64；确认的是解码后的字节数，与二进制载荷同义。
      const written = typeof data === "string" ? atob(data).length : data.byteLength;
      return { written, nextOffset: offset + written };
    },
    finish: async () => undefined,
    cancel: async () => undefined,
    onDragState: () => () => undefined,
    onDrop: () => () => undefined,
  },
  // 宿主 host.storage mock（Host API 1.2，pluginHostBridge storage 命名空间同形）：
  // 与真实 web 宿主同形由 localStorage 兜底（键名不变；字符串值原样、对象 JSON
  // 编码），刷新/重开不丢——?render=dom / mock 走查依赖该语义；opaque origin
  // 等不可用场景退化为内存 Map。get 未命中返回 null，set(undefined) 归一化为 null。
  capabilities: { storage: true, ...(aiPanelOn ? { ai: true } : {}), ...(aiCompletionOn ? { aiCompletion: true, aiRecommendations: true } : {}) },
  storage: (() => {
    let ls: Storage | null = null;
    try {
      window.localStorage.setItem("__dbx_mock_storage_probe__", "1");
      window.localStorage.removeItem("__dbx_mock_storage_probe__");
      ls = window.localStorage;
    } catch {
      ls = null;
    }
    const mem = new Map<string, string>();
    const write = (key: string, value: unknown) => {
      const raw = typeof value === "string" ? value : JSON.stringify(value);
      if (ls) ls.setItem(key, raw);
      else mem.set(key, raw);
    };
    const read = (key: string): unknown => {
      const raw = ls ? ls.getItem(key) : (mem.get(key) ?? null);
      if (raw === null) return null;
      try {
        const parsed = JSON.parse(raw);
        return parsed !== null && typeof parsed === "object" ? parsed : raw;
      } catch {
        return raw;
      }
    };
    return {
      get: async (key: string) => read(key),
      set: async (key: string, value: unknown) => {
        write(key, value === undefined ? null : value);
        return null;
      },
      delete: async (key: string) => {
        if (ls) ls.removeItem(key);
        else mem.delete(key);
        return null;
      },
    };
  })(),
};
queueMicrotask(() => document.dispatchEvent(new CustomEvent("dbx-plugin-init", {
  detail: { contributionId: fixtureParams.get("tunnels") === "quick" ? TUNNELS_QUICK_ID : fixtureParams.has("tunnels") ? TUNNELS_WORKBENCH_ID : WORKBENCH_ID },
})));

// AI 面板会话记录（Warp AI 对齐批）：smoke 走查断言 host.ai.openConversation
// 的入参（title/prompt/context 快照与脱敏效果）。
(window as unknown as { __dbxMockAiConversations?: Array<Record<string, unknown>> }).__dbxMockAiConversations = aiConversations;
(window as unknown as { __dbxMockAiGenerations?: Array<Record<string, unknown>> }).__dbxMockAiGenerations = aiGenerations;
(window as unknown as { __dbxMockAiRecommendations?: () => { pushes: number; clears: number } }).__dbxMockAiRecommendations = getAiRecommendationStats;
// mock 专有调试入口（真实桥无此字段）：切换 locale 并经 onEvent 投递
// {type:"env", locale}（镜像真桥 updateLocale），供 mock.html 控制台 / 单测
// 走查 i18n 切换链（瞬态 notice 不随切语重译的 R5-P2-2 维持豁免，不在本夹具
// 模拟范围）。
(window as unknown as { __dbxMockSetLocale?: (next: string) => void }).__dbxMockSetLocale = setMockLocale;

// mock 专有调试入口：把本地终端打入退出态（验证退出覆盖层/退出码显示）。
(window as unknown as { __dbxMockExitLocal?: () => void }).__dbxMockExitLocal = exitLocalSession;

// mock 专有调试入口：向 SSH 可视会话注入一段终端输出（issue #150 交互提示
// 守卫走查用——在页面里摆出 MFA/验证码/选择菜单等远端提示行，验证 ↑/↓ 与
// 建议/ghost 浮层让位远端）。语义与真实 PTY 输出一致（走既有 binary 帧）。
(window as unknown as { __dbxMockEmitTerminal?: (text: string) => void }).__dbxMockEmitTerminal = (text: string) => {
  emitTerminal(text);
};

// 供单元测试（mockDbxHost.spec.ts）以模块形式动态导入并重置状态。
export {};
