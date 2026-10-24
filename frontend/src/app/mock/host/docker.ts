/**
 * Docker 面板夹具：docker/list|logs|action 镜像 sidecar 家族 payload 形状。
 * ?docker=off 模拟「CLI 未安装」（available:false），?docker=sudo 模拟
 * found-but-denied（needsSudo:true）；默认返回容器表。
 */

import { UNHANDLED, type Unhandled } from "./shared";
import { fixtureParams } from "./fixture";

export async function handleDockerInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  // cli 参数仅做白名单校验回显（真实端点归一在 sidecar），供引擎设置联动走查。
  if (method === "docker/list") {
    const mode = fixtureParams.get("docker");
    if (mode === "off") return { available: false, needsSudo: false, containers: [] };
    if (mode === "sudo") return { available: false, needsSudo: true, containers: [] };
    return {
      available: true,
      needsSudo: false,
      containers: [
        { id: "d4a7c9f1e2b3a1c0d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7", name: "web-nginx", image: "nginx:1.27", state: "running", status: "Up 3 days", ports: "0.0.0.0:8080->80/tcp, :::8080->80/tcp", createdAt: "2026-09-25 08:15:04 +0000 UTC" },
        { id: "9f8e7d6c5b4a", name: "cache-redis", image: "redis:7-alpine", state: "running", status: "Up 26 hours", ports: "127.0.0.1:6379->6379/tcp", createdAt: "2026-09-27 09:30:00 +0000 UTC" },
        { id: "1a2b3c4d5e6f", name: "db-postgres", image: "postgres:16", state: "exited", status: "Exited (0) 2 hours ago", ports: "", createdAt: "2026-09-18 21:30:00 +0000 UTC" },
        { id: "0f1e2d3c4b5a", name: "worker-queue", image: "python:3.12-slim", state: "paused", status: "Up 5 days (Paused)", ports: "", createdAt: "2026-09-23 12:00:00 +0000 UTC" },
      ],
    };
  }
  if (method === "docker/logs") {
    const input = (params || {}) as Record<string, unknown>;
    const containerId = String(input.containerId || "");
    if (!/^[0-9a-f]{12,64}$/.test(containerId)) throw new Error(`Invalid containerId '${containerId}': expected 12-64 lowercase hex characters`);
    return {
      logs: [
        "10:00:01 INFO  nginx ready — accept connections on :80",
        "10:00:04 WARN  upstream response slower than 500ms (612ms)",
        "10:00:09 INFO  GET /health 200 in 2ms",
        "10:00:12 ERROR upstream connect timeout after 3 tries",
      ].join("\n") + "\n",
      container: {
        id: `sha256:${containerId}`,
        name: "web-nginx",
        image: "nginx:1.27",
        status: "running",
        running: true,
        startedAt: "2026-09-25T08:15:04.2Z",
        health: "healthy",
        restartPolicy: "unless-stopped",
      },
    };
  }
  if (method === "docker/action") {
    const input = (params || {}) as Record<string, unknown>;
    const containerId = String(input.containerId || "");
    if (!/^[0-9a-f]{12,64}$/.test(containerId)) throw new Error(`Invalid containerId '${containerId}': expected 12-64 lowercase hex characters`);
    const action = String(input.action || "");
    if (!["start", "stop", "restart", "kill", "rm"].includes(action)) throw new Error(`Unsupported docker action '${action}'. Supported: start, stop, restart, kill, rm`);
    if (fixtureParams.get("docker") === "off") throw new Error(`${action} failed: docker CLI not found`);
    return { success: true, output: `${containerId}\n` };
  }
  return UNHANDLED;
}
