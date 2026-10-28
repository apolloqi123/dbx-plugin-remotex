/**
 * 端口映射/隧道域夹具（ssh/forward/*）：内存列表 + 递增 id，start/stop 与
 * 真实 sidecar 语义一致（0 端口由 mock 随机挑一个、start 后广播 active 状态）。
 */

import { UNHANDLED, type Unhandled, emitHostEvent } from "./shared";
import { context } from "./fixture";

const mockForwards: Array<Record<string, unknown>> = [];
let mockForwardSeq = 1;

/** 可视化夹具：给 ?forwards=1 / ?tunnels=1 播种运行中隧道与保存预设。 */
export function seedTunnelFixture() {
  if (mockForwards.length) return;
  mockForwards.push(
    {
      id: "fixture-local", sessionId: "", connectionId: context.connectionId, kind: "local",
      listenHost: "127.0.0.1", listenPort: 8080, boundPort: 8080,
      targetHost: "db.internal", targetPort: 5432, state: "active", error: null,
      connectionsTotal: 7, connectionsActive: 2, bytesUp: 481_233, bytesDown: 3_912_004,
    },
    {
      id: "fixture-remote", sessionId: "", connectionId: context.connectionId, kind: "remote",
      listenHost: "127.0.0.1", listenPort: 9000, boundPort: 9000,
      targetHost: "localhost", targetPort: 3000, state: "active", error: null,
      connectionsTotal: 1, connectionsActive: 1, bytesUp: 12_800, bytesDown: 204_800,
    },
    {
      id: "fixture-socks", sessionId: "", connectionId: context.connectionId, kind: "dynamic",
      listenHost: "127.0.0.1", listenPort: 1080, boundPort: 1080,
      targetHost: "", targetPort: 0, state: "starting", error: null,
      connectionsTotal: 0, connectionsActive: 0, bytesUp: 0, bytesDown: 0,
    },
  );
}

export async function handleTunnelsInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/forward/list") return { forwards: mockForwards };
  if (method === "ssh/forward/interfaces") {
    return {
      interfaces: [
        { name: "lo0", addr: "127.0.0.1", isLoopback: true },
        { name: "lo0", addr: "::1", isLoopback: true },
        { name: "en0", addr: "192.168.1.24", isLoopback: false },
        { name: "utun4", addr: "fd7a:115c:a1e0::3", isLoopback: false },
      ],
    };
  }
  if (method === "ssh/forward/start") {
    const input = params as Record<string, unknown>;
    const listenPort = Number(input.listenPort || 0);
    const boundPort = listenPort > 0 ? listenPort : 30000 + Math.floor(Math.random() * 20000);
    const forward = {
      id: `mock-forward-${mockForwardSeq++}`,
      sessionId: String(input.sessionId || ""),
      connectionId: String(input.connectionId || context.connectionId),
      kind: input.kind === "remote" || input.kind === "dynamic" ? input.kind : "local",
      listenHost: String(input.listenHost || "127.0.0.1"),
      listenPort,
      boundPort,
      targetHost: String(input.targetHost || ""),
      targetPort: Number(input.targetPort || 0),
      state: "active",
      error: null,
      connectionsTotal: 0,
      connectionsActive: 0,
      bytesUp: 0,
      bytesDown: 0,
    };
    mockForwards.push(forward);
    setTimeout(() => {
      emitHostEvent({ method: "ssh/forward/state", params: { id: forward.id, sessionId: forward.sessionId, connectionId: forward.connectionId, state: "active", error: null } });
    }, 20);
    return { forward };
  }
  if (method === "ssh/forward/stop") {
    const id = String((params as Record<string, unknown>)?.id || "");
    const index = mockForwards.findIndex((forward) => forward.id === id);
    if (index < 0) throw new Error("Port mapping was not found or already stopped");
    mockForwards.splice(index, 1);
    return { success: true };
  }
  return UNHANDLED;
}
