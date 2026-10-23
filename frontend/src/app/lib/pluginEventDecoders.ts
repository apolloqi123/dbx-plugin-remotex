// 后端事件 payload 解码：跨进程边界的 `unknown` 一律先收敛再交给域逻辑。
//
// 背景（review）：usePluginEvents 在 RDP/串口/AI 审批分支直接
// `event.params as unknown as X` —— 双重断言把运行时校验完全跳过，sidecar
// 版本漂移或畸形事件会让域状态机吃到半截对象（画布 cursor 错位、上传进度
// 变成 NaN、审批挑战缺字段）。此处以纯函数做**字段级窄化**：形态不符返回
// null，调用方静默丢弃该事件（事件流不是用户操作，不应在 UI 上报错）。

import type { AgentFinishPayload, AgentNoticePayload, AgentPromptPayload } from "../../features/agent/lib/agentTerminal";
import type { RdpPointerEvent } from "../../features/remote-desktop/lib/rdpFrame";
import type { SerialUploadProgress, SerialUploadProtocol } from "../../features/serial/lib/serialUpload";

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isFiniteNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

const isString = (value: unknown): value is string => typeof value === "string";

const AGENT_RISKS = ["low", "elevated"] as const;
const SERIAL_PROTOCOLS: readonly SerialUploadProtocol[] = ["xmodem", "ymodem", "zmodem"];
const SERIAL_UPLOAD_STATES = ["running", "file_complete", "complete", "failed"] as const;

/** `rdp/pointer`：服务端光标形状（default/hidden/position/bitmap）。 */
export function decodeRdpPointerEvent(value: unknown): RdpPointerEvent | null {
  if (!isRecord(value)) return null;
  switch (value.type) {
    case "default":
      return { type: "default" };
    case "hidden":
      return { type: "hidden" };
    case "position":
      return isFiniteNumber(value.x) && isFiniteNumber(value.y)
        ? { type: "position", x: value.x, y: value.y }
        : null;
    case "bitmap":
      if (
        isFiniteNumber(value.width) &&
        isFiniteNumber(value.height) &&
        isFiniteNumber(value.hotspotX) &&
        isFiniteNumber(value.hotspotY) &&
        isString(value.rgbaBase64)
      ) {
        return {
          type: "bitmap",
          width: value.width,
          height: value.height,
          hotspotX: value.hotspotX,
          hotspotY: value.hotspotY,
          rgbaBase64: value.rgbaBase64,
        };
      }
      return null;
    default:
      return null;
  }
}

/** `serial/upload/progress`：串口文件上传进度。 */
export function decodeSerialUploadProgress(value: unknown): SerialUploadProgress | null {
  if (!isRecord(value)) return null;
  const state = value.state;
  if (
    !isString(value.sessionId) ||
    !isString(value.fileName) ||
    !SERIAL_PROTOCOLS.includes(value.protocol as SerialUploadProtocol) ||
    !isFiniteNumber(value.fileIndex) ||
    !isFiniteNumber(value.sent) ||
    !isFiniteNumber(value.total) ||
    !isString(state)
  ) {
    return null;
  }
  // 未知的未来状态字符串按终态失败处理：丢弃事件会让 overlay 停在 running
  // 且取消键随 busy 消失（UI 悬挂）。字段畸形仍由上面的窄化拒绝。
  const normalizedState = SERIAL_UPLOAD_STATES.includes(state as (typeof SERIAL_UPLOAD_STATES)[number])
    ? (state as SerialUploadProgress["state"])
    : "failed";
  return {
    sessionId: value.sessionId,
    protocol: value.protocol as SerialUploadProtocol,
    fileName: value.fileName,
    fileIndex: value.fileIndex,
    sent: value.sent,
    total: value.total,
    state: normalizedState,
    ...(isString(value.reason) ? { reason: value.reason } : {}),
  };
}

/** `ssh/agent/prompt`：审批挑战。MCP confirm 档允许缺 sessionId（进程级）。 */
export function decodeAgentPromptPayload(value: unknown): AgentPromptPayload | null {
  if (!isRecord(value)) return null;
  const source = value.source === "mcp" ? "mcp" : undefined;
  const sessionId = isString(value.sessionId) ? value.sessionId : source === "mcp" ? "" : null;
  if (
    !isString(value.challengeId) ||
    sessionId === null ||
    !isString(value.tool) ||
    !isString(value.command) ||
    !AGENT_RISKS.includes(value.risk as (typeof AGENT_RISKS)[number]) ||
    !isFiniteNumber(value.requestedAt) ||
    !isFiniteNumber(value.timeoutSecs)
  ) {
    return null;
  }
  return {
    challengeId: value.challengeId,
    sessionId,
    tool: value.tool,
    command: value.command,
    risk: value.risk as AgentPromptPayload["risk"],
    requestedAt: value.requestedAt,
    timeoutSecs: value.timeoutSecs,
    ...(source ? { source } : {}),
  };
}

/** `ssh/agent/notice`：低危命令注入告知。 */
export function decodeAgentNoticePayload(value: unknown): AgentNoticePayload | null {
  if (!isRecord(value)) return null;
  if (
    !isString(value.sessionId) ||
    !isString(value.tool) ||
    !isString(value.command) ||
    !AGENT_RISKS.includes(value.risk as (typeof AGENT_RISKS)[number])
  ) {
    return null;
  }
  return {
    sessionId: value.sessionId,
    tool: value.tool,
    command: value.command,
    risk: value.risk as AgentNoticePayload["risk"],
  };
}

/** `ssh/agent/finish`：审批执行收尾。 */
export function decodeAgentFinishPayload(value: unknown): AgentFinishPayload | null {
  if (!isRecord(value)) return null;
  // status 接受任意字符串：未知的未来状态仍代表“本次执行已结束”，事件派发
  // 据此清空 agentRunning；非字符串/缺字段仍按畸形 payload 丢弃。
  if (!isString(value.sessionId) || !isString(value.status)) return null;
  return { sessionId: value.sessionId, status: value.status };
}

/** `telnet/trigger` / `ssh/trigger` 的展示字段：stage 归一为 ≥1 的整数。 */
export function decodeTriggerEvent(value: unknown): { stage: number; kind: string } | null {
  if (!isRecord(value)) return null;
  const stage = Math.max(1, Math.trunc(isFiniteNumber(value.stage) ? value.stage : Number(value.stage) || 1));
  return { stage, kind: isString(value.kind) ? value.kind : "" };
}
