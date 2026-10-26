// 容器文件传输的纯逻辑与跨域桥（Task 11/12）。
//
// 传输链路的边界：容器页只拿得到 t/container/sessionId/connectionId/canWrite，
// 而真正把字节搬进搬出的是 SFTP 下载链（steps7-sftp 的 downloadRemoteFile）与
// 上传队列（uploadSource）。两者经 `setDockerTransferBridge` 在 setup 期登记，
// 容器页经 `getDockerTransferBridge` 取用，避免把 App 级状态塞进 props。

export const CONTAINER_TRANSFER_MAX_BYTES = 512 * 1024 * 1024;
export const CONTAINER_UPLOAD_CHUNK_BYTES = 256 * 1024;

/** 会话存在 → SSH；缺省 → sidecar 所在机器的 daemon。 */
export type ContainerFileTarget = { sessionId: string } | { target: "local" };

export function containerTargetParams(sessionId: string | undefined | null): ContainerFileTarget {
  return sessionId ? { sessionId } : { target: "local" };
}

/** 单文件上限（上传前与 export 后各判一次）。 */
export function isFileTooLarge(size: number | undefined): boolean {
  return typeof size === "number" && Number.isFinite(size) && size > CONTAINER_TRANSFER_MAX_BYTES;
}

/** 暂存文件的父目录（POSIX；暂存路径由 sidecar 生成，恒为绝对路径）。 */
export function stagingDirectory(path: string): string {
  const index = path.lastIndexOf("/");
  return index <= 0 ? "/" : path.slice(0, index);
}

/** 暂存文件名。 */
export function stagingFileName(path: string): string {
  const index = path.lastIndexOf("/");
  return index < 0 ? path : path.slice(index + 1);
}

/** 分片 → 标准 base64（`upload/chunk` 的 dataBase64）。 */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const block = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += block) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + block));
  }
  return btoa(binary);
}

export interface ContainerFileExport {
  stagingPath: string;
  size: number;
  name: string;
  location: "remote" | "local";
}

/** `docker/files/export` 响应归一：缺 stagingPath 视为失败，其余字段宽容。 */
export function parseExportResponse(raw: unknown): ContainerFileExport | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const record = raw as Record<string, unknown>;
  const stagingPath = typeof record.stagingPath === "string" ? record.stagingPath : "";
  if (!stagingPath) return undefined;
  const size =
    typeof record.size === "number" && Number.isFinite(record.size) && record.size >= 0
      ? record.size
      : 0;
  const name =
    typeof record.name === "string" && record.name ? record.name : stagingFileName(stagingPath);
  return {
    stagingPath,
    size,
    name,
    location: record.location === "local" ? "local" : "remote",
  };
}

/** SFTP 下载/上传队列的最小契约（由 step7-sftp 在 setup 期登记）。 */
export interface DockerTransferBridge {
  downloadRemoteFile: (input: {
    remotePath: string;
    name: string;
    size?: number;
    forceSudo?: boolean;
  }) => Promise<void>;
  uploadSource: (
    name: string,
    size: number,
    readChunk: (offset: number, length: number) => Promise<Uint8Array>,
    resume: undefined,
    targetDir: string,
    options: { duplicatePreCheckedAbsent: boolean; displayName?: string },
  ) => Promise<boolean>;
  /** 本机容器下载的撞名策略：复用 SFTP 下载的偏好与「每次询问」弹窗。
   * undefined = 用户取消；缺省 = 后端自动让位（rename）。 */
  resolveDownloadConflict?: (name: string) => Promise<"rename" | "overwrite" | undefined>;
}

let bridge: DockerTransferBridge | undefined;

export function setDockerTransferBridge(next: DockerTransferBridge | undefined): void {
  bridge = next;
}

export function getDockerTransferBridge(): DockerTransferBridge | undefined {
  return bridge;
}
