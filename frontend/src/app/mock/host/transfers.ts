/**
 * 传输/录制域夹具：sftp/transfer/*（列表/历史/上传/下载/取消）、
 * ssh/recording/* 与 local/recording/*。
 *
 * 上传 finish 立即返回、进度经 progress 事件回传；下载分块走 binary 通道
 * （8 字节 LE/BE 偏移头 + 负载），与真实 sidecar 的 taskId/offset 语义一致。
 */

import { UNHANDLED, type Unhandled, emitHostBinary, emitHostEvent } from "./shared";
import { context, fixtureParams } from "./fixture";
import { mockFileSystem } from "../mockFileSystem";

const fixtureDownloads = new Map<string, { fileName: string; size: number; offset: number }>();
const fixtureUploadCount = { value: 0 };
// 可视化 mock 也需要具备真实的“清空后刷新为空”语义，否则历史工具栏
// 的刷新/清空按钮看起来没有效果。
let mockTransferHistoryCleared = false;

// 录制回放 fixture：一条 ~24s 的输出事件流，供录制浮条与回放弹窗（进度条/
// 倍速/GIF 导出）走查。data 为直写 xterm 的原始文本（asciicast eventdata）。
const RECORDING_FIXTURE_EVENTS = [
  { time: 0, type: "o", data: "user@server:~$ ./deploy.sh --env=production\r\n" },
  { time: 1.2, type: "o", data: "[1/4] building image …\r\n" },
  { time: 4.5, type: "o", data: "[2/4] pushing registry.demo.internal/app:1.4.2\r\n" },
  { time: 9.8, type: "o", data: "[3/4] rolling update 3/3 replicas ✓\r\n" },
  { time: 15.4, type: "o", data: "[4/4] health check passed (200 OK)\r\n" },
  { time: 18.0, type: "o", data: "deploy finished SUCCESS in 17.6s\r\n" },
  { time: 23.5, type: "o", data: "user@server:~$ " },
];
const RECORDING_FIXTURE_SUMMARY = {
  recordingId: "visual-recording-1",
  sessionId: "visual-session",
  host: "server.demo.internal",
  startedAt: Date.now() - 3_600_000,
  durationSecs: 23.5,
  bytes: 4096,
};

/** sftp/upload/<taskId> 二进制分块：ack 偏移 + staging 进度事件。 */
export function handleSftpUploadBinary(channel: string, bytes: Uint8Array): void {
  const offset = Number(new DataView(bytes.buffer, bytes.byteOffset, 8).getBigUint64(0, false));
  const taskId = channel.slice("sftp/upload/".length);
  const nextOffset = offset + Math.max(0, bytes.byteLength - 8);
  emitHostEvent({ method: "sftp/upload/ack", params: { taskId, nextOffset } });
  // 镜像真实 sidecar 的 staging 进度事件（phase 字段见 issue #60 修复）。
  emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, direction: "upload", transferred: nextOffset, size: 0, phase: "staging", status: "running" } });
}

export async function handleTransfersInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "sftp/transfer/list") return { tasks: [] };
  if (method === "ssh/recording/list") return { recordings: [RECORDING_FIXTURE_SUMMARY] };
  if (method === "ssh/recording/get") {
    const input = (params || {}) as Record<string, unknown>;
    const offset = Math.max(0, Math.floor(Number(input.offset) || 0));
    const limit = Math.max(1, Math.floor(Number(input.limit) || 500));
    const events = RECORDING_FIXTURE_EVENTS.slice(offset, offset + limit);
    return { events, total: RECORDING_FIXTURE_EVENTS.length, hasMore: offset + events.length < RECORDING_FIXTURE_EVENTS.length };
  }
  if (method === "ssh/recording/search") {
    // M14：镜像真实契约——无持久索引、即时扫描；名称命中（host/recordingId
    // 包含查询词，大小写不敏感）或内容命中（展平文本包含查询词），回命中摘录。
    const query = String((params as Record<string, unknown>)?.query || "").trim().toLowerCase();
    const matches: Array<Record<string, unknown>> = [];
    if (query) {
      const summary = RECORDING_FIXTURE_SUMMARY;
      const nameMatch = summary.host.toLowerCase().includes(query) || summary.recordingId.toLowerCase().includes(query);
      const stripAnsi = (raw: string) => raw.replace(/\x1b\[[0-9;]*[A-Za-z]/g, "").trim();
      const hits = RECORDING_FIXTURE_EVENTS
        .filter((event) => stripAnsi(event.data).toLowerCase().includes(query))
        .slice(0, 5)
        .map((event) => ({ time: event.time, excerpt: stripAnsi(event.data) }));
      if (nameMatch || hits.length) matches.push({ ...summary, nameMatch, hits });
    }
    return { recordings: matches };
  }
  if (method === "ssh/recording/delete") return { success: true };
  if (method === "ssh/recording/clear") return { success: true, deleted: 1 };
  if (method === "ssh/recording/reveal") return { success: true };
  if (method === "local/recording/start") {
    // 本地终端录制（dock 本地终端录制按钮）：与 ssh/recording 同形；走查里
    // 点录制即走这里，回固定 recordingId 供 stop 配对。
    return { recordingId: "visual-local-recording-1", recording: true };
  }
  if (method === "local/recording/stop") {
    return { ...RECORDING_FIXTURE_SUMMARY, host: "local", recordingId: "visual-local-recording-1" };
  }
  if (method === "sftp/transfer/history") {
    // ?err=transferHistory 模拟历史查询失败，供面板 loadFailed+重试态走查。
    if (fixtureParams.get("err") === "transferHistory") throw new Error("sftp: transfer history unavailable");
    // 镜像真实 sidecar 契约（transfer-history.json 环形 200 + live 合并、
    // 新→旧排序、status 枚举无 queued、时间戳 Unix 毫秒、failed 带 error）。
    const input = (params || {}) as Record<string, unknown>;
    const limitRaw = Number(input.limit);
    const limit = Number.isFinite(limitRaw) && limitRaw > 0 ? Math.min(200, Math.floor(limitRaw)) : 50;
    const now = Date.now();
    const entries: Array<Record<string, unknown>> = mockTransferHistoryCleared ? [] : [
      { taskId: "visual-hist-4", connectionId: context.connectionId, direction: "upload", fileName: "deploy.sh", size: 2481, transferred: 2481, status: "completed", startedAt: now - 400_000, finishedAt: now - 396_000 },
      { taskId: "visual-hist-3", connectionId: context.connectionId, direction: "download", fileName: "server.log", size: 741_248, transferred: 741_248, status: "completed", startedAt: now - 800_000, finishedAt: now - 790_000 },
      { taskId: "visual-hist-2", connectionId: context.connectionId, direction: "download", fileName: "core.dump", size: 268_435_456, transferred: 268_435_456, status: "failed", startedAt: now - 1_600_000, finishedAt: now - 1_590_000, error: "disk quota exceeded" },
      { taskId: "visual-hist-1", connectionId: context.connectionId, direction: "upload", fileName: "backup.tar.gz", size: 52_428_800, transferred: 0, status: "cancelled", startedAt: now - 3_200_000, finishedAt: now - 3_190_000 },
    ];
    return { tasks: entries.slice(0, limit) };
  }
  if (method === "sftp/transfer/history/clear") {
    mockTransferHistoryCleared = true;
    return { success: true };
  }
  if (method === "sftp/upload/start" || method === "sudo/upload/start") return { taskId: `visual-upload-${++fixtureUploadCount.value}`, chunkSize: 262144 };
  if (method === "sftp/upload/finish" || method === "sudo/upload/finish") {
    // 镜像真实契约（issue #60）：finish 立即返回，推送在后台进行，终态经
    // progress 事件回传。mock 无真实推送，直接补发完成事件。sudo/upload 是
    // UploadSudo 车道（参数族同 remotePath，提交走 sudo mv），mock 同构。
    const taskId = String((params as Record<string, unknown>).taskId || "");
    queueMicrotask(() => {
      emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, direction: "upload", transferred: 0, size: 0, phase: "uploading", status: "completed" } });
    });
    return { success: true, phase: "uploading" };
  }
  if (method === "sftp/transfer/cancel") return { success: true };
  if (method === "sftp/download/start") {
    const remotePath = mockFileSystem.normalizePath(String((params as Record<string, unknown>)?.remotePath || "download.bin"));
    const node = mockFileSystem.findNode(remotePath);
    const taskId = `visual-download-${fixtureDownloads.size + 1}`;
    const fileName = node?.kind === "file" ? node.name : remotePath.split("/").pop() || "download.bin";
    const size = node?.kind === "file" ? node.size : 32;
    fixtureDownloads.set(taskId, { fileName, size, offset: 0 });
    emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, sessionId: "visual-session", direction: "download", fileName, transferred: 0, size, status: "queued" } });
    return { taskId, fileName, size, chunkSize: 262144 };
  }
  if (method === "sftp/download/next") {
    const input = params as Record<string, unknown>;
    const taskId = String(input.taskId || "");
    const task = fixtureDownloads.get(taskId)!;
    const offset = Number(input.offset || 0);
    const length = Math.min(262144, task.size - offset);
    const payload = new Uint8Array(8 + length);
    new DataView(payload.buffer).setBigUint64(0, BigInt(offset), false);
    emitHostBinary({ channel: `sftp/download/${taskId}`, data: payload });
    task.offset = offset + length;
    emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, sessionId: "visual-session", direction: "download", fileName: task.fileName, transferred: task.offset, size: task.size, status: "running" } });
    return { length, eof: task.offset >= task.size };
  }
  if (method === "sftp/download/finish") {
    const taskId = String((params as Record<string, unknown>)?.taskId || "");
    const task = fixtureDownloads.get(taskId)!;
    emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, sessionId: "visual-session", direction: "download", fileName: task.fileName, transferred: task.size, size: task.size, status: "completed" } });
    fixtureDownloads.delete(taskId);
    return { success: true };
  }
  if (method === "sudo/download/start") {
    // M14-C DownloadSudo mock：参数名 path（sudo 族），分块/finish 复用
    // fixtureDownloads，与 sftp/download/next、finish 完全同构。
    const remotePath = mockFileSystem.normalizePath(String((params as Record<string, unknown>)?.path || "root.bin"));
    const node = mockFileSystem.findNode(remotePath);
    const taskId = `sudo-download-${fixtureDownloads.size + 1}`;
    const fileName = node?.kind === "file" ? node.name : remotePath.split("/").pop() || "root.bin";
    const size = node?.kind === "file" ? node.size : 32;
    fixtureDownloads.set(taskId, { fileName, size, offset: 0 });
    emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, sessionId: "visual-session", direction: "download", fileName, transferred: 0, size, status: "queued" } });
    return { taskId, fileName, size, chunkSize: 262144, sudo: true };
  }
  if (method === "sudo/download/cancel") {
    const taskId = String((params as Record<string, unknown>)?.taskId || "");
    fixtureDownloads.delete(taskId);
    emitHostEvent({ method: "sftp/transfer/progress", params: { taskId, sessionId: "visual-session", direction: "download", transferred: 0, size: 0, status: "cancelled" } });
    return { success: true };
  }
  return UNHANDLED;
}
