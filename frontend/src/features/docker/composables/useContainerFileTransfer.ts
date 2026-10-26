// 容器文件传输编排（Task 11 SSH / Task 12 本机）。
//
// 统一入口：`download(file)` 先 `docker/files/export` 生成暂存件，再按
// `location`/会话落盘（SSH → SFTP 下载队列；本机 → saveLocal），最后**恒定**
// 调用 `docker/files/cleanup`（失败只提示，不改变传输结果）。
// `upload(file)` 的目标是「当前目录 + 原文件名」：SSH 走 prepareUpload →
// 现有 uploadSource（暂存目录/文件名）→ import；本机走 begin/chunk/finish →
// import，256 KiB 分片、字节进度与取消。
//
// SFTP 下载/上传实现不在这里：由 `lib/dockerFileTransfer` 的桥在 setup 期注入
// （测试直接传 vi.fn()）。
import { ref, type Ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import { formatBytes } from "../../../shared/lib/format";
import { joinContainerPath } from "../lib/dockerFilePaths";
import {
  CONTAINER_TRANSFER_MAX_BYTES,
  CONTAINER_UPLOAD_CHUNK_BYTES,
  bytesToBase64,
  containerTargetParams,
  isFileTooLarge,
  parseExportResponse,
  stagingDirectory,
  stagingFileName,
} from "../lib/dockerFileTransfer";
import { containerStatPayload } from "./useContainerFiles";

export interface ContainerTransferFile {
  name: string;
  path: string;
  kind?: "dir" | "file";
  size?: number;
}

export interface ContainerUploadProgress {
  name: string;
  size: number;
  transferred: number;
  percent: number;
}

export interface ContainerTransferAction {
  label: string;
  run: () => void;
}

export interface ContainerFileTransferOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  /** 值或 getter；缺省 = 本机 daemon。 */
  sessionId?: string | (() => string | undefined);
  containerId: string | (() => string);
  /** 上传目标目录（当前浏览路径）；缺省 `/`。 */
  currentPath?: string | (() => string);
  canWrite?: boolean | (() => boolean);
  showError: (cause: unknown) => void;
  showNotice: (message: string, actions?: ContainerTransferAction[]) => void;
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
  confirmDialog: (message: string, options?: { danger?: boolean }) => Promise<boolean>;
  /** 每次请求附带的引擎参数（cli/socket/host）。 */
  engineParams?: () => Record<string, string>;
}

function readValue<T>(value: T | (() => T) | undefined, fallback: T): T {
  if (typeof value === "function") return (value as () => T)();
  return value === undefined ? fallback : value;
}

export function useContainerFileTransfer(options: ContainerFileTransferOptions) {
  const readSessionId = (): string => {
    const value = typeof options.sessionId === "function" ? options.sessionId() : options.sessionId;
    return value || "";
  };
  const readContainerId = (): string => readValue(options.containerId, "");
  const readCurrentPath = (): string => readValue(options.currentPath, "/") || "/";
  const readCanWrite = (): boolean => readValue(options.canWrite, true);

  const busyPath = ref("");
  const uploading = ref<ContainerUploadProgress>();

  /** 本机分片上传的取消令牌与注册表 id（取消后不再发 chunk）。 */
  let uploadToken: { canceled: boolean } | undefined;
  let localUploadId = "";

  function targetParams(): Record<string, unknown> {
    return containerTargetParams(readSessionId());
  }

  function requestParams(): Record<string, unknown> {
    return {
      ...targetParams(),
      containerId: readContainerId(),
      ...(options.engineParams?.() ?? {}),
    };
  }

  function tooLargeError(size: number | undefined): Error {
    return new Error(
      options.t("docker.filesTooLarge", {
        size: formatBytes(size ?? 0),
        limit: formatBytes(CONTAINER_TRANSFER_MAX_BYTES),
      }),
    );
  }

  function fail(cause: unknown): never {
    options.showError(cause);
    throw cause;
  }

  /** 暂存清理：失败只提示（不覆盖主结果，也不把成功改成失败）。 */
  async function cleanup(stagingPath: string): Promise<void> {
    try {
      await rpc("docker/files/cleanup", { ...targetParams(), stagingPath });
    } catch {
      options.showNotice(options.t("docker.filesCleanupFailed"));
    }
  }

  /** 本机暂存 → 下载目录：单次 RPC，成功后给出「在文件夹中显示」。 */
  async function saveLocal(exported: { stagingPath: string; name: string }): Promise<void> {
    const saved = await rpc<{ localPath?: string }>("docker/files/saveLocal", {
      ...targetParams(),
      stagingPath: exported.stagingPath,
      name: exported.name,
    });
    const localPath = typeof saved?.localPath === "string" ? saved.localPath : "";
    if (!localPath) {
      options.showNotice(options.t("downloaded", { name: exported.name }));
      return;
    }
    options.showNotice(options.t("downloadedTo", { name: exported.name, path: localPath }), [
      {
        label: options.t("revealInFolder"),
        run: () => {
          void rpc("local/reveal", { path: localPath }).catch((cause) => options.showError(cause));
        },
      },
    ]);
  }

  /** 容器文件 → 暂存 → 本机/SFTP → 清理。 */
  async function download(file: ContainerTransferFile): Promise<void> {
    if (file.kind === "dir") return fail(new Error(options.t("docker.filesDirNotDownloadable")));
    if (isFileTooLarge(file.size)) return fail(tooLargeError(file.size));
    busyPath.value = file.path;
    let staging = "";
    try {
      const exported = parseExportResponse(
        await rpc<unknown>("docker/files/export", { ...requestParams(), path: file.path }),
      );
      if (!exported) throw new Error(options.t("docker.filesTransferFailed"));
      staging = exported.stagingPath;
      if (isFileTooLarge(exported.size)) throw tooLargeError(exported.size);
      if (exported.location === "local" || !readSessionId()) {
        await saveLocal(exported);
      } else {
        await options.downloadRemoteFile({
          remotePath: exported.stagingPath,
          name: exported.name,
          size: exported.size,
        });
      }
    } catch (cause) {
      options.showError(cause);
      throw cause;
    } finally {
      if (staging) await cleanup(staging);
      busyPath.value = "";
    }
  }

  /** 目标已存在时先确认覆盖（stat 失败 = 不存在，继续上传）。 */
  async function confirmOverwriteIfExists(destPath: string): Promise<boolean> {
    let exists = false;
    try {
      const stat = containerStatPayload(
        await rpc<unknown>("docker/files/stat", { ...requestParams(), path: destPath }),
      );
      exists = !stat.unavailable && stat.kind !== "";
    } catch {
      exists = false;
    }
    if (!exists) return true;
    return options.confirmDialog(
      options.t("docker.filesOverwriteConfirm", { name: stagingFileName(destPath) }),
    );
  }

  /** SSH 上传：prepareUpload → SFTP 队列写入暂存件 → import → 清理。 */
  async function uploadOverSsh(file: File, destPath: string): Promise<void> {
    let staging = "";
    try {
      const prepared = await rpc<{ stagingPath?: string }>("docker/files/prepareUpload", {
        ...requestParams(),
        name: file.name,
      });
      staging = typeof prepared?.stagingPath === "string" ? prepared.stagingPath : "";
      if (!staging) throw new Error(options.t("docker.filesTransferFailed"));
      const targetDir = stagingDirectory(staging);
      const stagingName = stagingFileName(staging) || file.name;
      const delivered = await options.uploadSource(
        stagingName,
        file.size,
        async (offset, length) =>
          new Uint8Array(await file.slice(offset, offset + length).arrayBuffer()),
        undefined,
        targetDir,
        { duplicatePreCheckedAbsent: true, displayName: file.name },
      );
      // uploadSource 返回 false = 队列放弃（无会话/重复目标跳过）：不 import。
      if (!delivered) return;
      await rpc("docker/files/import", {
        ...requestParams(),
        stagingPath: staging,
        destPath,
      });
    } catch (cause) {
      options.showError(cause);
      throw cause;
    } finally {
      if (staging) await cleanup(staging);
    }
  }

  /** 本机上传：begin → 256 KiB 顺序分片 → finish → import → 清理。 */
  async function uploadToLocal(file: File, destPath: string): Promise<void> {
    const token: { canceled: boolean } = { canceled: false };
    uploadToken = token;
    uploading.value = { name: file.name, size: file.size, transferred: 0, percent: 0 };
    let uploadId = "";
    let staging = "";
    try {
      const begin = await rpc<{ uploadId?: string; maxBytes?: number }>(
        "docker/files/upload/begin",
        { target: "local", name: file.name },
      );
      uploadId = typeof begin?.uploadId === "string" ? begin.uploadId : "";
      if (!uploadId) throw new Error(options.t("docker.filesTransferFailed"));
      localUploadId = uploadId;
      const limit = typeof begin?.maxBytes === "number" ? begin.maxBytes : CONTAINER_TRANSFER_MAX_BYTES;
      if (file.size > limit) throw tooLargeError(file.size);
      let offset = 0;
      while (offset < file.size) {
        if (token.canceled) throw new Error(options.t("docker.filesCancel"));
        const end = Math.min(file.size, offset + CONTAINER_UPLOAD_CHUNK_BYTES);
        const bytes = new Uint8Array(await file.slice(offset, end).arrayBuffer());
        const response = await rpc<{ received?: number }>("docker/files/upload/chunk", {
          target: "local",
          uploadId,
          offset,
          dataBase64: bytesToBase64(bytes),
        });
        // 顺序校验：后端只接受 offset == 已接收字节数，回包必须正好推进到 end。
        if (typeof response?.received !== "number" || response.received !== end) {
          throw new Error(options.t("docker.filesTransferFailed"));
        }
        offset = end;
        uploading.value = {
          name: file.name,
          size: file.size,
          transferred: offset,
          percent: file.size ? Math.round((offset / file.size) * 100) : 100,
        };
      }
      if (token.canceled) throw new Error(options.t("docker.filesCancel"));
      const finished = await rpc<{ stagingPath?: string }>("docker/files/upload/finish", {
        target: "local",
        uploadId,
      });
      staging = typeof finished?.stagingPath === "string" ? finished.stagingPath : "";
      if (!staging) throw new Error(options.t("docker.filesTransferFailed"));
      localUploadId = "";
      uploadId = "";
      await rpc("docker/files/import", {
        target: "local",
        containerId: readContainerId(),
        stagingPath: staging,
        destPath,
      });
    } catch (cause) {
      if (uploadId) {
        localUploadId = "";
        await rpc("docker/files/upload/cancel", { target: "local", uploadId }).catch(() => undefined);
      }
      if (token.canceled) options.showNotice(options.t("docker.filesCancel"));
      else options.showError(cause);
      throw cause;
    } finally {
      if (staging) await cleanup(staging);
      if (uploadToken === token) uploadToken = undefined;
      uploading.value = undefined;
    }
  }

  /** 上传入口：只读/超限在创建任何暂存前拒绝。 */
  async function upload(file: File): Promise<void> {
    if (!readCanWrite()) return fail(new Error(options.t("docker.filesUnsupported")));
    if (isFileTooLarge(file.size)) return fail(tooLargeError(file.size));
    const destPath = joinContainerPath(readCurrentPath(), file.name);
    busyPath.value = destPath;
    try {
      if (readSessionId()) {
        if (!(await confirmOverwriteIfExists(destPath))) return;
        await uploadOverSsh(file, destPath);
      } else {
        await uploadToLocal(file, destPath);
      }
    } finally {
      busyPath.value = "";
    }
  }

  /** 取消本机分片上传（SSH 上传由传输面板的取消入口负责）。 */
  async function cancelUpload(): Promise<void> {
    if (uploadToken) uploadToken.canceled = true;
    const uploadId = localUploadId;
    if (!uploadId) return;
    localUploadId = "";
    try {
      await rpc("docker/files/upload/cancel", { target: "local", uploadId });
    } catch {
      // 取消是尽力而为：后端 30 分钟 TTL 会兜底删除临时文件。
    }
  }

  /** 组件卸载：取消在途的本机上传（暂存由调用方 finally 清理）。 */
  function dispose(): void {
    if (uploadToken) uploadToken.canceled = true;
    void cancelUpload();
  }

  return {
    busyPath,
    uploading: uploading as Ref<ContainerUploadProgress | undefined>,
    download,
    upload,
    cancelUpload,
    dispose,
  };
}
