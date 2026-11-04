/**
 * 连接/会话域夹具：ssh/session/open|attach、ssh/sessions/list、
 * connection/protocol、ssh/terminal/replay，以及 ?err=disconnect 的单次断开
 * 与 ?auth=auto|autofail 的逐方式认证进度。
 */

import { UNHANDLED, type Unhandled, emitHostEvent } from "./shared";
import {
  authFixture,
  autoAuthActive,
  context,
  disconnectAfterMs,
  failSessionOpen,
  freshSessionOpen,
  protoFixture,
  slowSessionOpenMs,
  writable,
} from "./fixture";
import {
  currentSshSequence,
  emitTerminalTranscript,
  resetSshSequence,
} from "./terminal";

interface MockAutoAuthAttempt {
  method: string;
  status: "skipped" | "failed";
  detail: string;
}

// 剧本：password 失败 → private-key 跳过（本地无私钥）→ keyboard-interactive
// 失败 → agent 分歧点（auto 成功 / autofail 失败）。覆盖 skipped/failed 两种
// 状态，事件形状镜像 backend/src/sessions/ssh/auth/negotiation.rs authenticate_auto 的 emitter.event。
const AUTO_AUTH_BASE_ATTEMPTS: MockAutoAuthAttempt[] = [
  { method: "password", status: "failed", detail: "password rejected by server" },
  { method: "private-key", status: "skipped", detail: "no private key configured for this connection" },
  { method: "keyboard-interactive", status: "failed", detail: "keyboard-interactive authentication timed out" },
];
const AUTO_AUTH_AGENT_FAILURE: MockAutoAuthAttempt = { method: "agent", status: "failed", detail: "no agent socket available" };

// 聚合失败信息（auto_auth_failure_message 同构）：skipped 带前缀、failed 只带原因。
function autoAuthFailureMessage(attempts: MockAutoAuthAttempt[]): string {
  const summary = attempts
    .map((attempt) => (attempt.status === "skipped" ? `${attempt.method} (skipped: ${attempt.detail})` : `${attempt.method} (${attempt.detail})`))
    .join("; ");
  return `SSH authentication failed in Auto mode, tried in order — ${summary}`;
}

function emitAutoAuthProgress(includeAgentFailure: boolean): void {
  const attempts = includeAgentFailure ? [...AUTO_AUTH_BASE_ATTEMPTS, AUTO_AUTH_AGENT_FAILURE] : AUTO_AUTH_BASE_ATTEMPTS;
  for (const attempt of attempts) {
    emitHostEvent({
      method: "ssh/auth/auto",
      params: { operationId: "visual-auth-auto", connectionId: context.connectionId, method: attempt.method, status: attempt.status, detail: attempt.detail },
    });
  }
}

let disconnectEmitted = false;

// ?err=disconnect 注入入口：open 与 attach 完成后都调用一次；全局单发，
// 重连（再次 open/attach）后不再复发，与真实传输断开的单次语义一致。
function scheduleDisconnect() {
  if (!disconnectAfterMs || disconnectEmitted) return;
  disconnectEmitted = true;
  setTimeout(() => {
    emitHostEvent({ method: "ssh/session/state", params: { sessionId: "visual-session", state: "disconnected" } });
  }, disconnectAfterMs);
}

export async function handleSessionInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/session/open") {
    if (slowSessionOpenMs) await new Promise((resolve) => setTimeout(resolve, slowSessionOpenMs));
    if (failSessionOpen) throw new Error("SSH password authentication failed: password rejected by server");
    // ?auth=auto|autofail：先按 AUTO_AUTH_ORDER 发逐方式进度事件（真实
    // sidecar 在认证过程中发、响应前送达），autofail 再抛聚合错误终态。
    if (autoAuthActive) emitAutoAuthProgress(authFixture === "autofail");
    if (authFixture === "autofail") throw new Error(autoAuthFailureMessage([...AUTO_AUTH_BASE_ATTEMPTS, AUTO_AUTH_AGENT_FAILURE]));
    // A fresh session restarts sequence numbering at 1 (real sidecar
    // semantics): after an auto-reconnect the client resets its cursor to 0,
    // so continuing the global counter here would leave a permanent hole at
    // the old tail and spin the client's replay loop.
    resetSshSequence();
    emitTerminalTranscript();
    scheduleDisconnect();
    return { sessionId: "visual-session", connectionId: context.connectionId, workbenchId: context.workbenchId, connected: true, sequence: 0, chunkSize: 262144, directoryTrackingSupported: true };
  }
  if (method === "ssh/terminal/replay") return { frameCount: 0, firstAvailableSequence: 1, tailSequence: currentSshSequence(), complete: true };
  // 对齐真实 sidecar 的 connection/protocol：lifecycle 把非 SSH 连接的协议暂存，
  // 已登记的 SSH 连接也回 "ssh"（宿主先推 connection/connect 再建 webview），只有
  // 宿主尚未推送该连接时才是空串。?proto=pending 模拟最后那种迟到场景。
  if (method === "connection/protocol") {
    const staged = protoFixture === "rdp" || protoFixture === "telnet" || protoFixture === "vnc" || protoFixture === "serial" ? protoFixture : "";
    return { protocol: protoFixture === "pending" ? "" : staged || "ssh" };
  }
  // 工作台心跳：真实 sidecar 用它在宿主误发延迟拆除时续期；mock 只需应答成功。
  if (method === "ssh/session/alive") return { success: true };
  if (method === "ssh/sessions/list") {
    return { sessions: failSessionOpen || freshSessionOpen ? [] : [{ sessionId: "visual-session", connectionId: context.connectionId, workbenchId: context.workbenchId, readOnly: !writable, connected: true, sudoKeepalive: true, createdAt: Math.floor(Date.now() / 1000), authMethod: authFixture === "auto" ? "auto" : "private-key", host: "server.demo.internal", port: 22, username: "demo" }] };
  }
  if (method === "ssh/session/attach") {
    const input = params as Record<string, unknown>;
    // 默认启动走 sessions/list → reattach；?slow=N 同样延迟 attach，否则
    // 连接中卡片在默认路径下一闪而过、无法视觉验证。
    if (slowSessionOpenMs) await new Promise((resolve) => setTimeout(resolve, slowSessionOpenMs));
    if (failSessionOpen) throw new Error("Connection is not active");
    // Mirror the real sidecar: the session already owned by this workbench is
    // reported with a complete replay. The transcript (Welcome + OSC 633
    // cycle) is pushed as live frames right after attach so the first paint
    // under the default reattach startup already shows shell-integration
    // content (P2-2), not a bare prompt.
    emitTerminalTranscript();
    scheduleDisconnect();
    return {
      sessionId: String(input.sessionId || "") || "visual-session",
      connectionId: context.connectionId,
      workbenchId: context.workbenchId,
      connected: true,
      sequence: currentSshSequence(),
      chunkSize: 262144,
      directoryTrackingSupported: true,
      replay: { complete: true, frameCount: 0, firstAvailableSequence: currentSshSequence() + 1, tailSequence: currentSshSequence() },
    };
  }
  return UNHANDLED;
}
