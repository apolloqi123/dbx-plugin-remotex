// 传输弹层行模型（纯逻辑）：把活跃任务、落盘历史与断点续传统一成同一种
// 展示行，供筛选条与 TransferRow 复用。不依赖 Vue，便于单测排序/去重/计数。
import type { TransferHistoryEntry } from "./transferHistory";
import type { ResumableUploadTask } from "./transferResume";
import type { TransferTask } from "./transferQueue";

export type TransferFilterKey = "all" | "upload" | "download" | "paused" | "completed";
export type TransferRowSource = "live" | "history" | "resumable";
export type TransferRowStatus = TransferTask["status"] | "resumable";

export const TRANSFER_FILTER_KEYS: readonly TransferFilterKey[] = ["all", "upload", "download", "paused", "completed"];

export interface TransferRowView {
  key: string;
  taskId: string;
  source: TransferRowSource;
  fileName: string;
  direction: "upload" | "download";
  status: TransferRowStatus;
  percent: number;
  sizeLabel: string;
  detailLabel: string;
  speedLabel?: string;
  timeLabel?: string;
  localPath?: string;
  remotePath?: string;
  currentFile?: string;
  error?: string;
  compression?: "gzip";
  isFolder: boolean;
  paused: boolean;
  canPause: boolean;
  canCancel: boolean;
  canResume: boolean;
  canReveal: boolean;
  canOpenFile: boolean;
}

export interface BuildTransferRowsInput {
  transferList: TransferTask[];
  history: TransferHistoryEntry[];
  resumableTasks: ResumableUploadTask[];
  pausedTaskIds: Set<string>;
  speeds: Record<string, number>;
  formatBytes: (value?: number) => string;
  transferPercent: (task: TransferTask) => number;
  transferShownBytes: (task: TransferTask) => number;
  transferPausable: (status: string) => boolean;
  /** 无文件夹名可还原时（旧版临时 tar）使用的文件夹标签。 */
  folderLabel: string;
}

/** 文件夹整包上传的内部归档名（含旧版无 stem 的随机名）。 */
export function isFolderArchiveName(fileName: string): boolean {
  return /^\.sshx-upload-.+\.tar$/i.test(fileName);
}

/** 文件夹整包上传使用 `.sshx-upload-<stem>-<suffix>.tar` 内部名：能从名字
 *  还原文件夹名就还原，旧版无 stem 的只显示通用“文件夹上传”标签。 */
export function displayTransferFileName(fileName: string, folderLabel: string): string {
  const archive = /^\.sshx-upload-(.*)-[a-z0-9]+-[a-z0-9]+\.tar$/i.exec(fileName);
  if (archive) return archive[1] || folderLabel;
  if (isFolderArchiveName(fileName)) return folderLabel;
  return fileName;
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

/** 行内时间戳：`MM-DD HH:mm`（本地时区），与参考面板的紧凑格式一致。 */
export function formatTransferStamp(ms?: number): string {
  if (!ms || !Number.isFinite(ms)) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function liveRow(task: TransferTask, input: BuildTransferRowsInput): TransferRowView {
  const paused = input.pausedTaskIds.has(task.taskId);
  const status = task.status;
  const percent = status === "completed" ? 100 : clampPercent(input.transferPercent(task));
  const speed = input.speeds[task.taskId];
  return {
    key: `live:${task.taskId}`,
    taskId: task.taskId,
    source: "live",
    fileName: task.displayName ?? displayTransferFileName(task.fileName || task.taskId, input.folderLabel),
    direction: task.direction,
    status,
    percent,
    sizeLabel: input.formatBytes(task.size),
    detailLabel: `${input.formatBytes(input.transferShownBytes(task))} / ${input.formatBytes(task.size)}`,
    speedLabel: speed ? `${input.formatBytes(speed)}/s` : undefined,
    timeLabel: formatTransferStamp(task.joinedAt),
    localPath: task.localPath,
    currentFile: task.currentFile,
    error: task.error,
    compression: task.compression,
    isFolder: Boolean(task.displayFolder) || isFolderArchiveName(task.fileName) || (task.fileCount ?? 0) > 1 || Boolean(task.currentFile),
    paused,
    canPause: input.transferPausable(status),
    canCancel: status === "queued" || status === "running",
    canResume: false,
    canReveal: Boolean(task.localPath),
    canOpenFile: Boolean(task.localPath),
  };
}

function historyRow(entry: TransferHistoryEntry, input: BuildTransferRowsInput): TransferRowView {
  const complete = entry.status === "completed";
  const percent = complete ? 100 : entry.size > 0 ? clampPercent((entry.transferred / entry.size) * 100) : 0;
  return {
    key: `history:${entry.taskId}`,
    taskId: entry.taskId,
    source: "history",
    fileName: displayTransferFileName(entry.fileName || entry.taskId, input.folderLabel),
    direction: entry.direction,
    status: entry.status,
    percent,
    sizeLabel: input.formatBytes(entry.size),
    detailLabel: input.formatBytes(entry.size),
    timeLabel: formatTransferStamp(entry.finishedAt ?? entry.startedAt),
    localPath: entry.localPath,
    error: entry.error,
    isFolder: isFolderArchiveName(entry.fileName),
    paused: false,
    canPause: false,
    canCancel: false,
    canResume: false,
    canReveal: Boolean(entry.localPath),
    canOpenFile: Boolean(entry.localPath),
  };
}

function resumableRow(task: ResumableUploadTask, input: BuildTransferRowsInput): TransferRowView {
  const percent = task.size > 0 ? clampPercent((task.resumableBytes / task.size) * 100) : 0;
  return {
    key: `resumable:${task.taskId}`,
    taskId: task.taskId,
    source: "resumable",
    fileName: task.fileName || task.taskId,
    direction: "upload",
    status: "resumable",
    percent,
    sizeLabel: input.formatBytes(task.size),
    detailLabel: `${input.formatBytes(task.resumableBytes)} / ${input.formatBytes(task.size)}`,
    localPath: undefined,
    remotePath: task.remotePath,
    isFolder: false,
    paused: false,
    canPause: false,
    canCancel: false,
    canResume: true,
    canReveal: false,
    canOpenFile: false,
  };
}

// 活跃区排序：进行中 > 缓冲 > 排队 > 失败 > 完成 > 取消；同状态新任务在前。
const LIVE_STATUS_ORDER: Record<string, number> = {
  running: 0,
  staging: 1,
  queued: 2,
  failed: 3,
  completed: 4,
  cancelled: 5,
};

/** 活跃行在前、历史按完成时间新→旧、续传最后；活跃/历史同 taskId 时只留活跃行。 */
export function buildTransferRows(input: BuildTransferRowsInput): TransferRowView[] {
  const live = [...input.transferList].sort(
    (a, b) =>
      (LIVE_STATUS_ORDER[a.status] ?? 9) - (LIVE_STATUS_ORDER[b.status] ?? 9) ||
      (b.joinedAt ?? 0) - (a.joinedAt ?? 0),
  );
  const rows = live.map((task) => liveRow(task, input));
  const liveIds = new Set(input.transferList.map((task) => task.taskId));
  const history = [...input.history]
    .filter((entry) => !liveIds.has(entry.taskId))
    .sort((a, b) => (b.finishedAt ?? b.startedAt ?? 0) - (a.finishedAt ?? a.startedAt ?? 0));
  rows.push(...history.map((entry) => historyRow(entry, input)));
  rows.push(...input.resumableTasks.filter((task) => !liveIds.has(task.taskId)).map((task) => resumableRow(task, input)));
  return rows;
}

export function filterTransferRows(rows: readonly TransferRowView[], filter: TransferFilterKey): TransferRowView[] {
  if (filter === "all") return [...rows];
  if (filter === "upload" || filter === "download") {
    return rows.filter((row) => row.direction === filter);
  }
  if (filter === "paused") return rows.filter((row) => row.paused);
  return rows.filter((row) => row.status === "completed");
}

export function countTransferFilters(rows: readonly TransferRowView[]): Record<TransferFilterKey, number> {
  return {
    all: rows.length,
    upload: rows.filter((row) => row.direction === "upload").length,
    download: rows.filter((row) => row.direction === "download").length,
    paused: rows.filter((row) => row.paused).length,
    completed: rows.filter((row) => row.status === "completed").length,
  };
}

/** 历史条目按 taskId 反查（行模板渲染右键菜单时用）。 */
export function findHistoryEntry(history: readonly TransferHistoryEntry[], taskId: string): TransferHistoryEntry | undefined {
  return history.find((entry) => entry.taskId === taskId);
}
