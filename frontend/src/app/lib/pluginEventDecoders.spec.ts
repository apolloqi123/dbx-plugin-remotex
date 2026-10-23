// pluginEventDecoders 单测：合法 payload 原样收敛、畸形 payload 返回 null
// （调用方静默丢事件），以及 MCP 审批缺 sessionId 的向后兼容分支。

import { describe, expect, it } from "vitest";
import {
  decodeAgentFinishPayload,
  decodeAgentNoticePayload,
  decodeAgentPromptPayload,
  decodeRdpPointerEvent,
  decodeSerialUploadProgress,
  decodeTriggerEvent,
} from "./pluginEventDecoders";

describe("decodeRdpPointerEvent", () => {
  it("接受四种合法形态", () => {
    expect(decodeRdpPointerEvent({ type: "default" })).toEqual({ type: "default" });
    expect(decodeRdpPointerEvent({ type: "hidden" })).toEqual({ type: "hidden" });
    expect(decodeRdpPointerEvent({ type: "position", x: 1, y: 2 })).toEqual({ type: "position", x: 1, y: 2 });
    expect(
      decodeRdpPointerEvent({ type: "bitmap", width: 2, height: 3, hotspotX: 0, hotspotY: 1, rgbaBase64: "AA==" }),
    ).toEqual({ type: "bitmap", width: 2, height: 3, hotspotX: 0, hotspotY: 1, rgbaBase64: "AA==" });
  });

  it("拒绝畸形形态（缺字段/NaN/未知 type/非对象）", () => {
    expect(decodeRdpPointerEvent(null)).toBeNull();
    expect(decodeRdpPointerEvent({ type: "position", x: 1 })).toBeNull();
    expect(decodeRdpPointerEvent({ type: "position", x: Number.NaN, y: 2 })).toBeNull();
    expect(decodeRdpPointerEvent({ type: "bitmap", width: 2 })).toBeNull();
    expect(decodeRdpPointerEvent({ type: "bogus" })).toBeNull();
  });
});

describe("decodeSerialUploadProgress", () => {
  const valid = {
    sessionId: "s1",
    protocol: "zmodem",
    fileName: "a.bin",
    fileIndex: 0,
    sent: 10,
    total: 100,
    state: "running",
  };

  it("合法 payload（含可选 reason）", () => {
    expect(decodeSerialUploadProgress(valid)).toEqual(valid);
    expect(decodeSerialUploadProgress({ ...valid, state: "failed", reason: "timeout" })).toMatchObject({ reason: "timeout" });
  });

  it("拒绝未知协议与缺字段；未知状态字符串按终态失败收敛（不悬挂 UI）", () => {
    expect(decodeSerialUploadProgress({ ...valid, protocol: "kermit" })).toBeNull();
    expect(decodeSerialUploadProgress({ ...valid, state: 42 })).toBeNull();
    expect(decodeSerialUploadProgress({ ...valid, state: null })).toBeNull();
    expect(decodeSerialUploadProgress({ ...valid, sent: "10" })).toBeNull();
    expect(decodeSerialUploadProgress(undefined)).toBeNull();
    // 未来新增状态：接受事件并按失败（终态）收敛，overlay 可收起。
    expect(decodeSerialUploadProgress({ ...valid, state: "paused", reason: "future" })).toMatchObject({
      state: "failed",
      reason: "future",
    });
  });
});

describe("decodeAgentPromptPayload", () => {
  const valid = {
    challengeId: "c1",
    sessionId: "s1",
    tool: "ssh",
    command: "rm -rf /tmp/x",
    risk: "elevated",
    requestedAt: 1700000000,
    timeoutSecs: 60,
  };

  it("合法 payload", () => {
    expect(decodeAgentPromptPayload(valid)).toEqual(valid);
  });

  it("MCP confirm 档允许缺 sessionId（进程级挑战）", () => {
    const { sessionId: _omit, ...withoutSession } = valid;
    expect(decodeAgentPromptPayload({ ...withoutSession, source: "mcp" })).toMatchObject({ sessionId: "", source: "mcp" });
    expect(decodeAgentPromptPayload(withoutSession)).toBeNull();
  });

  it("拒绝非法 risk/缺 challengeId/非对象", () => {
    expect(decodeAgentPromptPayload({ ...valid, risk: "high" })).toBeNull();
    expect(decodeAgentPromptPayload({ ...valid, challengeId: 7 })).toBeNull();
    expect(decodeAgentPromptPayload([])).toBeNull();
  });
});

describe("decodeAgentNoticePayload / decodeAgentFinishPayload", () => {
  it("合法与畸形", () => {
    expect(decodeAgentNoticePayload({ sessionId: "s", tool: "t", command: "c", risk: "low" })).toEqual({
      sessionId: "s",
      tool: "t",
      command: "c",
      risk: "low",
    });
    expect(decodeAgentNoticePayload({ sessionId: "s", tool: "t", command: "c" })).toBeNull();
    expect(decodeAgentFinishPayload({ sessionId: "s", status: "denied" })).toEqual({ sessionId: "s", status: "denied" });
    expect(decodeAgentFinishPayload({ sessionId: "s", status: "ok" })).toEqual({ sessionId: "s", status: "ok" });
  });

  it("finish 接受未知但合法的字符串状态（按终态清空 agentRunning）", () => {
    // sidecar 版本漂移新增状态：事件仍须收敛，否则 agentRunning 永久悬挂。
    for (const status of ["ok", "cancelled", "future-status"]) {
      expect(decodeAgentFinishPayload({ sessionId: "s", status })).toEqual({ sessionId: "s", status });
    }
    // 畸形 payload 继续拒绝：非对象 / 缺 sessionId / 非字符串 status。
    expect(decodeAgentFinishPayload(null)).toBeNull();
    expect(decodeAgentFinishPayload({ status: "done" })).toBeNull();
    expect(decodeAgentFinishPayload({ sessionId: "s" })).toBeNull();
    expect(decodeAgentFinishPayload({ sessionId: "s", status: 1 })).toBeNull();
    expect(decodeAgentFinishPayload({ sessionId: "s", status: null })).toBeNull();
  });
});

describe("decodeTriggerEvent", () => {
  it("stage 归一为 ≥1 的整数，kind 缺省为空串", () => {
    expect(decodeTriggerEvent({ stage: 2, kind: "timeout" })).toEqual({ stage: 2, kind: "timeout" });
    expect(decodeTriggerEvent({ stage: 0 })).toEqual({ stage: 1, kind: "" });
    expect(decodeTriggerEvent({ stage: "3" })).toEqual({ stage: 3, kind: "" });
    expect(decodeTriggerEvent({ stage: Number.NaN })).toEqual({ stage: 1, kind: "" });
    expect(decodeTriggerEvent(null)).toBeNull();
  });
});
