/**
 * Docker 面板夹具：docker/list|logs|action 与容器管理分屏的
 * docker/logs/start|stop、docker/files/* 家族，镜像 sidecar 的载荷形状
 * （backend/src/app/rpc/docker.rs、docker_files.rs、features/docker/stream.rs）。
 *
 * ?docker= 开关：
 * - 缺省：容器表 + 基础目录树；日志流每 700ms 推一行，10s 自然结束。
 * - `off`：模拟「CLI 未安装」——list 不可用，文件/日志调用直接报未找到 CLI。
 * - `sudo`：模拟 daemon socket 被拒——list needsSudo；文件页报权限错误，
 *   日志流先推一行再以 `needsSudo:true` 结束（走查「以 sudo 重试」）。
 * - `files`：在基础容器树外补 /opt/app 子树（含零字节文件），供文件页多级
 *   进出/空文件走查。
 *
 * 暂存路径是合成的固定前缀（不落真实磁盘）：SSH 为 /tmp/dbx-sshx.<token>/…，
 * 本机为 /tmp/dbx-sshx-mock/<seq>/…，与 sidecar 的前缀校验语义一致。
 */

import { UNHANDLED, type Unhandled, emitHostEvent } from "./shared";
import { fixtureParams } from "./fixture";

const CONTAINER_ID = /^[0-9a-f]{12,64}$/;
const CLI = "docker";
const STREAM_ID = "mock-stream";
const LOG_TICK_MS = 700;
const LOG_STREAM_MS = 10_000;
const TAIL_DEFAULT = 200;
const TAIL_MIN = 10;
const TAIL_MAX = 2000;
const UPLOAD_MAX_BYTES = 512 * 1024 * 1024;
const UPLOAD_CHUNK_BYTES = 256 * 1024;
const FIXTURE_MTIME = 1786262400;

interface MockContainerEntry {
  kind: "dir" | "file";
  size: number;
  mtime: number;
  mode: string;
}

/** 容器内固定树；`?docker=files` 在模块求值时并入 EXTRA_TREE。 */
const BASE_TREE: Record<string, MockContainerEntry> = {
  "/": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/bin": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/bin/busybox": { kind: "file", size: 1183448, mtime: FIXTURE_MTIME, mode: "0755" },
  "/etc": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/etc/hostname": { kind: "file", size: 13, mtime: FIXTURE_MTIME, mode: "0644" },
  "/etc/hosts": { kind: "file", size: 221, mtime: FIXTURE_MTIME, mode: "0644" },
  "/etc/nginx": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/etc/nginx/nginx.conf": { kind: "file", size: 1264, mtime: FIXTURE_MTIME, mode: "0644" },
  "/home": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/home/demo": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/home/demo/notes.txt": { kind: "file", size: 512, mtime: FIXTURE_MTIME, mode: "0644" },
  "/tmp": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "1777" },
  "/tmp/.keep": { kind: "file", size: 0, mtime: FIXTURE_MTIME, mode: "0644" },
  "/var": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/var/lib": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/var/log": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/var/log/nginx": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/var/log/nginx/access.log": { kind: "file", size: 741248, mtime: FIXTURE_MTIME, mode: "0644" },
  "/var/log/nginx/error.log": { kind: "file", size: 8192, mtime: FIXTURE_MTIME, mode: "0644" },
  "/var/log/syslog": { kind: "file", size: 15432, mtime: FIXTURE_MTIME, mode: "0644" },
};

/** `?docker=files` 追加的多级目录 + 零字节文件。 */
const EXTRA_TREE: Record<string, MockContainerEntry> = {
  "/opt": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/opt/app": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/opt/app/bin": { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" },
  "/opt/app/bin/start.sh": { kind: "file", size: 128, mtime: FIXTURE_MTIME, mode: "0755" },
  "/opt/app/config.yml": { kind: "file", size: 2048, mtime: FIXTURE_MTIME, mode: "0644" },
  "/opt/app/empty.txt": { kind: "file", size: 0, mtime: FIXTURE_MTIME, mode: "0644" },
};

/** 当前夹具树：`?docker=files` 在模块求值时并入 EXTRA_TREE，import 会继续追加。 */
const containerTree: Record<string, MockContainerEntry> = {
  ...BASE_TREE,
  ...(fixtureParams.get("docker") === "files" ? EXTRA_TREE : {}),
};

const LOG_LINES = [
  "10:00:01 INFO  nginx ready — accept connections on :80",
  "10:00:03 INFO  worker process 27 started",
  "10:00:04 WARN  upstream response slower than 500ms (612ms)",
  "10:00:06 INFO  GET /health 200 in 2ms",
  "10:00:08 INFO  GET /assets/app.4f2c.js 200 in 4ms",
  "10:00:09 INFO  GET /api/status 200 in 11ms",
  "10:00:12 ERROR upstream connect timeout after 3 tries",
  "10:00:13 INFO  retrying upstream in 250ms",
  "10:00:14 INFO  GET /api/status 200 in 9ms",
  "10:00:17 INFO  GET /index.html 200 in 1ms",
  "10:00:19 WARN  client closed connection before response (499)",
  "10:00:21 INFO  GET /health 200 in 2ms",
  "10:00:24 INFO  cache hit ratio 0.94",
  "10:00:27 INFO  GET /api/items?page=2 200 in 18ms",
  "10:00:30 INFO  graceful shutdown signal received",
];

let logTimer: ReturnType<typeof setInterval> | null = null;
let logEndTimer: ReturnType<typeof setTimeout> | null = null;
let logCursor = 0;
let stagingSeq = 0;
let uploadSeq = 0;

const uploads = new Map<string, { name: string; received: number }>();
/** 本轮派发的暂存路径 → 字节数，供 import 回填固定树里的 size。 */
const stagedSizes = new Map<string, number>();

function dockerMode(): string {
  return fixtureParams.get("docker") || "default";
}

function requireContainerId(params: Record<string, unknown>): string {
  const containerId = String(params.containerId || "");
  if (!CONTAINER_ID.test(containerId)) {
    throw new Error(`Invalid containerId '${containerId}': expected 12-64 lowercase hex characters`);
  }
  return containerId;
}

/** 镜像 sidecar 的路径归一：绝对路径、折叠 `.`/`..`、拒绝控制字符。 */
function normalizePath(raw: unknown): string {
  if (typeof raw !== "string" || raw === "") throw new Error("Container path must not be empty");
  if (!raw.startsWith("/")) throw new Error(`Container path '${raw}' must be absolute (start with '/')`);
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(raw)) throw new Error("Container path must not contain control characters");
  const segments: string[] = [];
  for (const segment of raw.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") segments.pop();
    else segments.push(segment);
  }
  return segments.length ? `/${segments.join("/")}` : "/";
}

function parentPath(path: string): string {
  const index = path.lastIndexOf("/");
  return index <= 0 ? "/" : path.slice(0, index);
}

function baseName(path: string): string {
  return path.split("/").pop() || "download";
}

function cliName(params: Record<string, unknown>): string {
  const cli = String(params.cli || "").trim();
  return cli || CLI;
}

function guardMode(cli: string, method = "files/list"): void {
  const mode = dockerMode();
  if (mode === "off") throw new Error(`${cli} CLI not found on this host`);
  if (mode === "sudo") {
    throw new Error(
      `${cli} ${method} failed (exit 1): permission denied while trying to connect to the Docker daemon socket`,
    );
  }
}

/** tail 钳制与 sidecar 一致（10–2000，缺省 200）。 */
function clampTail(raw: unknown): number {
  const value = Number(raw);
  if (raw === undefined || raw === null || !Number.isFinite(value)) return TAIL_DEFAULT;
  return Math.min(TAIL_MAX, Math.max(TAIL_MIN, Math.round(value)));
}

function listContainer(path: string) {
  const entries = Object.entries(containerTree)
    .filter(([candidate]) => candidate !== "/" && parentPath(candidate) === path)
    .map(([candidate, entry]) => ({ name: baseName(candidate), path: candidate, kind: entry.kind }))
    .sort((left, right) =>
      // 与 sidecar `sort_entries` 同口径：目录优先，再按名称区分大小写升序。
      left.kind === right.kind
        ? left.name < right.name
          ? -1
          : left.name > right.name
            ? 1
            : 0
        : left.kind === "dir"
          ? -1
          : 1,
    );
  return { path, parent: parentPath(path), entries };
}

function statPayload(path: string) {
  const entry = containerTree[path];
  if (!entry) {
    throw new Error(`${CLI} files/stat failed (exit 1): stat: cannot statx '${path}': No such file or directory`);
  }
  return { kind: entry.kind, size: entry.size, mtime: entry.mtime, mode: entry.mode };
}

/** 暂存路径：远端 `/tmp/dbx-sshx.<token>/<name>`，本机 `/tmp/dbx-sshx-mock/<seq>/<name>`。 */
function stagingPath(name: string, local: boolean): string {
  stagingSeq += 1;
  const dir = local
    ? `/tmp/dbx-sshx-mock/${String(stagingSeq).padStart(4, "0")}`
    : `/tmp/dbx-sshx.mock-${String(stagingSeq).padStart(4, "0")}`;
  return `${dir}/${name}`;
}

function isLocalTarget(params: Record<string, unknown>): boolean {
  return params.target === "local";
}

function stopLogStream(): boolean {
  const active = logTimer !== null || logEndTimer !== null;
  if (logTimer !== null) clearInterval(logTimer);
  if (logEndTimer !== null) clearTimeout(logEndTimer);
  logTimer = null;
  logEndTimer = null;
  return active;
}

function emitLogChunk(streamId: string): void {
  const line = LOG_LINES[logCursor % LOG_LINES.length];
  logCursor += 1;
  emitHostEvent({ method: "docker/logs/chunk", params: { streamId, text: `${line}\n` } });
}

function endLogStream(streamId: string, payload: Record<string, unknown>): void {
  stopLogStream();
  emitHostEvent({ method: "docker/logs/end", params: { streamId, ...payload } });
}

function startLogStream(streamId: string, tail: number): void {
  // 单工作台夹具只维护一条流：重复 start 覆盖旧流（sidecar 允许 4 条，
  // 走查只需要一条可见的跟随流）。tail 决定起始回溯位置（夹具日志共 15 行）。
  stopLogStream();
  logCursor = Math.max(0, LOG_LINES.length - Math.min(tail, LOG_LINES.length));
  const mode = dockerMode();
  if (mode === "sudo") {
    // daemon socket 被拒：推一行后以 needsSudo 收口（UI 显示「以 sudo 重试」）。
    emitLogChunk(streamId);
    logEndTimer = setTimeout(() => {
      endLogStream(streamId, {
        exitCode: 1,
        error: "permission denied while trying to connect to the Docker daemon socket",
        needsSudo: true,
      });
    }, LOG_TICK_MS);
    return;
  }
  logTimer = setInterval(() => emitLogChunk(streamId), LOG_TICK_MS);
  logEndTimer = setTimeout(() => endLogStream(streamId, { exitCode: 0 }), LOG_STREAM_MS);
}

export async function handleDockerInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  const input = (params || {}) as Record<string, unknown>;
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
    const containerId = String(input.containerId || "");
    if (!CONTAINER_ID.test(containerId)) throw new Error(`Invalid containerId '${containerId}': expected 12-64 lowercase hex characters`);
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
    const containerId = String(input.containerId || "");
    if (!CONTAINER_ID.test(containerId)) throw new Error(`Invalid containerId '${containerId}': expected 12-64 lowercase hex characters`);
    const action = String(input.action || "");
    if (!["start", "stop", "restart", "kill", "rm"].includes(action)) throw new Error(`Unsupported docker action '${action}'. Supported: start, stop, restart, kill, rm`);
    if (fixtureParams.get("docker") === "off") throw new Error(`${action} failed: docker CLI not found`);
    return { success: true, output: `${containerId}\n` };
  }

  // 容器管理分屏：真流式日志（事件 docker/logs/chunk|end）。
  if (method === "docker/logs/start") {
    const cli = cliName(input);
    requireContainerId(input);
    if (dockerMode() === "off") throw new Error(`${cli} CLI not found on this host`);
    const streamId = STREAM_ID;
    startLogStream(streamId, clampTail(input.tail));
    return { streamId };
  }
  if (method === "docker/logs/stop") {
    const streamId = String(input.streamId || "");
    // stop 幂等：未知/已结束的 id 返回 stopped:false，不抛错。
    if (streamId !== STREAM_ID) return { stopped: false };
    return { stopped: stopLogStream() };
  }

  if (method === "docker/files/list") {
    const cli = cliName(input);
    requireContainerId(input);
    const path = normalizePath(input.path ?? "/");
    guardMode(cli);
    if (containerTree[path]?.kind !== "dir") {
      throw new Error(
        `${cli} files/list failed (exit 2): ls: cannot access '${path}': No such file or directory`,
      );
    }
    return listContainer(path);
  }
  if (method === "docker/files/stat") {
    const cli = cliName(input);
    requireContainerId(input);
    const path = normalizePath(input.path ?? "/");
    guardMode(cli, "files/stat");
    return statPayload(path);
  }
  if (method === "docker/files/export") {
    const cli = cliName(input);
    requireContainerId(input);
    const path = normalizePath(input.path ?? "/");
    guardMode(cli, "files/stat");
    const entry = containerTree[path];
    if (!entry) {
      throw new Error(`${cli} files/stat failed (exit 1): stat: cannot statx '${path}': No such file or directory`);
    }
    if (entry.kind === "dir") throw new Error("目录暂不支持下载");
    const name = baseName(path);
    const local = isLocalTarget(input);
    const staging = stagingPath(name, local);
    stagedSizes.set(staging, entry.size);
    return { stagingPath: staging, size: entry.size, name, location: local ? "local" : "remote" };
  }
  if (method === "docker/files/prepareUpload") {
    if (isLocalTarget(input)) {
      throw new Error("docker/files/prepareUpload is only available for SSH targets");
    }
    const name = baseName(String(input.name || "upload.bin"));
    return { stagingPath: stagingPath(name, false), name };
  }
  if (method === "docker/files/import") {
    const cli = cliName(input);
    requireContainerId(input);
    guardMode(cli, "files/import");
    const staging = String(input.stagingPath || "");
    const destInput = String(input.destPath || "");
    if (!staging) throw new Error("Missing stagingPath");
    const dest = normalizePath(destInput);
    const name = baseName(staging);
    // destPath 以 `/` 结尾视为目录并保留暂存文件名，否则视为完整目标路径。
    const target = destInput.endsWith("/") ? `${dest === "/" ? "" : dest}/${name}` : dest;
    // 导入成功后固定树里可见（文件页刷新即可看到刚上传的文件）。
    const size = stagedSizes.get(staging) ?? 1024;
    stagedSizes.delete(staging);
    containerTree[target] = { kind: "file", size, mtime: FIXTURE_MTIME, mode: "0644" };
    const parent = parentPath(target);
    if (!containerTree[parent]) containerTree[parent] = { kind: "dir", size: 4096, mtime: FIXTURE_MTIME, mode: "0755" };
    return { path: target };
  }
  if (method === "docker/files/cleanup") {
    // 幂等：已清理/未知暂存路径同样返回 removed:true，走查不会出现假告警。
    return { removed: true };
  }
  if (method === "docker/files/saveLocal") {
    if (!isLocalTarget(input)) {
      throw new Error("docker/files/saveLocal is only available for local targets");
    }
    const name = baseName(String(input.name || "download.bin"));
    return { localPath: `/Users/demo/Downloads/${name}`, name };
  }
  if (method === "docker/files/upload/begin") {
    if (!isLocalTarget(input)) {
      throw new Error("docker/files/upload/begin is only available for local targets");
    }
    const name = baseName(String(input.name || "upload.bin"));
    const uploadId = `mock-upload-${++uploadSeq}`;
    uploads.set(uploadId, { name, received: 0 });
    return { uploadId, maxBytes: UPLOAD_MAX_BYTES };
  }
  if (method === "docker/files/upload/chunk") {
    const uploadId = String(input.uploadId || "");
    const upload = uploads.get(uploadId);
    if (!upload) throw new Error(`Unknown upload '${uploadId}'`);
    const offset = Number(input.offset);
    if (!Number.isFinite(offset) || offset !== upload.received) {
      throw new Error(`Upload chunk offset ${String(input.offset)} does not match received ${upload.received}`);
    }
    const decoded = atob(String(input.dataBase64 || ""));
    if (decoded.length > UPLOAD_CHUNK_BYTES) {
      throw new Error(`Upload chunk exceeds ${UPLOAD_CHUNK_BYTES} bytes`);
    }
    upload.received += decoded.length;
    if (upload.received > UPLOAD_MAX_BYTES) throw new Error(`Upload exceeds the ${UPLOAD_MAX_BYTES}-byte limit`);
    return { received: upload.received };
  }
  if (method === "docker/files/upload/finish") {
    const uploadId = String(input.uploadId || "");
    const upload = uploads.get(uploadId);
    if (!upload) throw new Error(`Unknown upload '${uploadId}'`);
    uploads.delete(uploadId);
    const staging = stagingPath(upload.name, true);
    stagedSizes.set(staging, upload.received);
    return { stagingPath: staging, size: upload.received };
  }
  if (method === "docker/files/upload/cancel") {
    const uploadId = String(input.uploadId || "");
    // 取消删除临时件：未知/已取消的 id 同样返回 canceled:true（幂等）。
    uploads.delete(uploadId);
    return { canceled: true };
  }
  return UNHANDLED;
}
