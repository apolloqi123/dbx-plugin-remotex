// Shared workbench value shapes. Extracted verbatim from App.vue (T21b).

export interface SftpEntry {
  name: string;
  uri: string;
  kind: "file" | "directory" | "symlink" | "other";
  size?: number;
  modifiedAt?: number;
  permissions?: string;
  contentType?: string;
  /** 属主用户/属组（includeOwner 时由 sidecar 返回；缺失显示 "-"）。 */
  owner?: string;
  group?: string;
  /** M14-B：显示名不可忠实还原（wire 名含 U+FFFD）；传输仍走 uri。 */
  lossy?: boolean;
}

export interface DownloadInfo {
  taskId: string;
  fileName: string;
  size: number;
  chunkSize: number;
  // 断点续传：start 带 offset 时回显的恢复起点。
  resumeOffset?: number;
  // 目录下载（tree/start）扩展：扫描得到的整树规模。
  fileCount?: number;
  dirCount?: number;
  skippedCount?: number;
}

export interface ExecResult {
  output: string;
  exitCode: number;
}

export interface DiskUsage {
  filesystem: string;
  mount: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  percentUsed: number;
}
