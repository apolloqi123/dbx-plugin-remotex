import { getCurrentScope, onScopeDispose, ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import {
  CLIPBOARD_DEFAULT_CHUNK_BYTES,
  base64FromBytes,
  bytesFromBase64,
  normalizeHostFiles,
  type ClipboardFileMeta,
  type ClipboardTransferProgress,
  type HostClipboardFile,
  type RemoteClipboardFile,
} from "../lib/clipboardFiles";
import type {
  BeginResult,
  ClipboardReadResult,
  ClipboardStatusResult,
  HostFileTransfer,
  RdpFileClipboardApi,
  RdpFileClipboardOptions,
} from "../lib/rdpFileClipboardTypes";

export type { RdpFileClipboardApi } from "../lib/rdpFileClipboardTypes";

function messageOf(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  return typeof cause === "string" ? cause : "";
}

function numeric(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, value) : 0;
}

function chunkSizeOf(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : CLIPBOARD_DEFAULT_CHUNK_BYTES;
}

function percentOf(done: number, total: number): number {
  if (total <= 0) return done > 0 ? 100 : 0;
  return Math.min(100, Math.round((done / total) * 100));
}

function remoteFilesFrom(value: unknown): RemoteClipboardFile[] {
  if (!Array.isArray(value)) return [];
  const files: RemoteClipboardFile[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    const name = typeof record.name === "string" ? record.name.trim() : "";
    if (!name || name === "." || name === "..") continue;
    const relativePath = typeof record.relativePath === "string" && record.relativePath ? record.relativePath : undefined;
    files.push({
      name,
      size: numeric(record.size),
      directory: record.directory === true,
      ...(relativePath ? { relativePath } : {}),
    });
  }
  return files;
}

function metadataOf(files: readonly HostClipboardFile[]): ClipboardFileMeta[] {
  return files.map((file) => ({
    name: file.name,
    size: file.size,
    ...(file.relativePath ? { relativePath: file.relativePath } : {}),
  }));
}

function hostFileTransfer(): HostFileTransfer | null {
  if (typeof window === "undefined") return null;
  const api = (window as unknown as { dbxPlugin?: { fileTransfer?: HostFileTransfer } }).dbxPlugin;
  return api?.fileTransfer ?? null;
}

/** Host writes may transfer the backing buffer; keep the view tightly packed. */
function tightlyPacked(bytes: Uint8Array): Uint8Array {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes : bytes.slice();
}

/** RDP clipboard file transfer (host <-> remote).
 *
 * Invariants:
 * - Chunk offsets are per file and advance by the bytes actually read; `eof`
 *   ends a file (declared sizes are metadata, not a read bound).
 * - Every failed upload aborts the backend uploadId so staging storage is freed.
 * - Saving streams into a host handle; any failure cancels that handle so a
 *   partial archive is never finished.
 * - Drops are ignored while the session is inactive or the config is disabled.
 */
export function useRdpFileClipboard(options: RdpFileClipboardOptions): RdpFileClipboardApi {
  const { t, showError, showNotice, sessionId, active } = options;
  const enabled = options.enabled ?? (() => true);

  const busy = ref(false);
  const progress = ref<ClipboardTransferProgress | null>(null);
  const remoteFiles = ref<{ files: RemoteClipboardFile[]; totalBytes: number } | null>(null);
  const archiveReady = ref<{ name: string; size: number; fileCount: number } | null>(null);
  const remoteReady = ref<{ fileCount: number; pasteboard: boolean } | null>(null);
  const nativeSupported = ref(false);
  const serverFiles = ref(false);
  const serverFilesKnown = ref(false);
  const error = ref("");

  let dropUnsubscribe: (() => void) | null = null;
  let dropStarted = false;
  let statusRestored = false;
  let downloadInFlight = false;

  function fail(cause: unknown): void {
    const message = messageOf(cause) || t("rdp.clipboardFiles.error");
    error.value = message;
    progress.value = null;
    showError(cause, "terminal");
  }

  function assertCurrentSession(sid: string): void {
    if (sessionId.value !== sid || !active()) throw new Error("RDP session is no longer active");
  }

  function hostChunk(read: { dataBase64?: string; length?: number }): { length: number; encoded: string } {
    const encoded = typeof read.dataBase64 === "string" ? read.dataBase64 : "";
    // `length` is authoritative when the host reports it; decoding the base64
    // only to count bytes is redundant on every 1 MiB upload chunk.
    if (typeof read.length === "number" && read.length >= 0) {
      return { length: read.length, encoded };
    }
    const decoded = bytesFromBase64(encoded);
    return { length: decoded.length, encoded: encoded || base64FromBytes(decoded) };
  }

  async function sendFiles(files: unknown[]): Promise<boolean> {
    const sid = sessionId.value;
    if (!sid || !active() || !enabled() || busy.value) return false;

    let entries: HostClipboardFile[];
    try {
      entries = normalizeHostFiles(files);
    } catch (cause) {
      fail(cause);
      return false;
    }
    if (!entries.length) return false;

    const bridge = hostFileTransfer();
    if (!bridge || typeof bridge.read !== "function") {
      fail(new Error("host file transfer bridge is unavailable"));
      return false;
    }

    const totalBytes = entries.reduce((sum, file) => sum + file.size, 0);
    let uploadId = "";
    let bytesDone = 0;
    let filesDone = 0;
    busy.value = true;
    error.value = "";
    progress.value = {
      phase: "uploading",
      done: 0,
      total: totalBytes,
      percent: percentOf(0, totalBytes),
      filesDone: 0,
      filesTotal: entries.length,
    };

    try {
      const begun = await rpc<BeginResult>("rdp/clipboard-files/begin", {
        sessionId: sid,
        files: metadataOf(entries),
      });
      if (!begun || typeof begun.uploadId !== "string" || !begun.uploadId) {
        throw new Error("rdp/clipboard-files/begin returned no uploadId");
      }
      uploadId = begun.uploadId;
      const chunkBytes = chunkSizeOf(begun.chunkBytes);

      for (let index = 0; index < entries.length; index += 1) {
        const file = entries[index];
        let offset = 0;
        for (;;) {
          assertCurrentSession(sid);
          const read = await bridge.read(file.handleId, offset, chunkBytes);
          assertCurrentSession(sid);
          const chunk = hostChunk(read);
          if (!chunk.length) {
            if (read.eof === true) break;
            throw new Error("host file read returned no data before EOF");
          }
          await rpc("rdp/clipboard-files/chunk", {
            sessionId: sid,
            uploadId,
            index,
            offset,
            dataBase64: chunk.encoded,
          });
          offset += chunk.length;
          bytesDone += chunk.length;
          assertCurrentSession(sid);
          progress.value = {
            phase: "uploading",
            done: bytesDone,
            total: totalBytes,
            percent: percentOf(bytesDone, totalBytes),
            filesDone,
            filesTotal: entries.length,
          };
          if (read.eof === true) break;
        }
        filesDone += 1;
        progress.value = {
          phase: "uploading",
          done: bytesDone,
          total: totalBytes,
          percent: percentOf(bytesDone, totalBytes),
          filesDone,
          filesTotal: entries.length,
        };
      }

      assertCurrentSession(sid);
      await rpc("rdp/clipboard-files/commit", { sessionId: sid, uploadId });
      showNotice(t("rdp.clipboardFiles.sent", { count: entries.length }));
      progress.value = null;
      return true;
    } catch (cause) {
      if (uploadId) {
        await rpc("rdp/clipboard-files/abort", { sessionId: sid, uploadId }).catch(() => undefined);
      }
      fail(cause);
      return false;
    } finally {
      busy.value = false;
    }
  }

  async function startRemoteDownload(): Promise<boolean> {
    const sid = sessionId.value;
    if (
      !sid ||
      !active() ||
      !enabled() ||
      busy.value ||
      downloadInFlight ||
      archiveReady.value ||
      !remoteFiles.value
    ) {
      return false;
    }
    downloadInFlight = true;
    try {
      const result = await rpc<{ downloading?: boolean }>("rdp/clipboard-files/download", {
        sessionId: sid,
      });
      assertCurrentSession(sid);
      if (result?.downloading === false) return false;
      progress.value = {
        phase: "downloading",
        done: 0,
        total: remoteFiles.value.totalBytes,
        percent: 0,
        filesDone: 0,
        filesTotal: remoteFiles.value.files.length,
      };
      return true;
    } catch (cause) {
      fail(cause);
      return false;
    } finally {
      downloadInFlight = false;
    }
  }

  async function saveRemoteArchive(): Promise<boolean> {
    const sid = sessionId.value;
    if (!sid || !active() || !enabled() || busy.value || (!archiveReady.value && !remoteReady.value)) return false;
    const bridge = hostFileTransfer();
    if (!bridge || typeof bridge.beginSave !== "function" || typeof bridge.write !== "function") return false;

    let handleId = "";
    busy.value = true;
    error.value = "";
    try {
      let archive = archiveReady.value;
      if (!archive) {
        const built = await rpc<{ archiveName?: string; archiveSize?: number }>("rdp/clipboard-files/archive", { sessionId: sid });
        assertCurrentSession(sid);
        if (!built || typeof built.archiveName !== "string" || !built.archiveName) {
          throw new Error("the remote clipboard archive is unavailable");
        }
        const fileCount = remoteReady.value?.fileCount ?? 0;
        archive = { name: built.archiveName, size: numeric(built.archiveSize), fileCount };
        archiveReady.value = archive;
      }
      const target = await bridge.beginSave({
        name: archive.name,
        contentType: "application/zip",
        size: archive.size,
      });
      // A null target means the user dismissed the host save dialog.
      if (!target) return false;
      handleId = target.handleId;
      const chunkBytes = chunkSizeOf(target.chunkBytes);
      let offset = 0;
      let total = archive.size;
      progress.value = { phase: "saving", done: 0, total, percent: percentOf(0, total) };

      for (;;) {
        assertCurrentSession(sid);
        const read = await rpc<ClipboardReadResult>("rdp/clipboard-files/read", {
          sessionId: sid,
          offset,
          length: chunkBytes,
        });
        assertCurrentSession(sid);
        if (typeof read.archiveName === "string" && read.archiveName && read.archiveName !== archive.name) {
          throw new Error("the remote clipboard archive changed during save");
        }
        if (typeof read.total === "number" && Number.isFinite(read.total) && read.total >= 0) total = read.total;
        const bytes = bytesFromBase64(typeof read.dataBase64 === "string" ? read.dataBase64 : "");

        if (bytes.length) {
          const written = await bridge.write(handleId, offset, tightlyPacked(bytes));
          if (typeof written?.written === "number" && written.written < bytes.length) {
            throw new Error("host file write was incomplete");
          }
          offset =
            typeof written?.nextOffset === "number" && written.nextOffset >= offset
              ? written.nextOffset
              : offset + bytes.length;
          const done = Math.min(offset, total);
          progress.value = { phase: "saving", done, total, percent: percentOf(done, total) };
        }

        if (read.eof === true) break;
        if (!bytes.length) throw new Error("rdp/clipboard-files/read returned no data before EOF");
      }

      if (typeof bridge.finish === "function") await bridge.finish(handleId);
      progress.value = null;
      archiveReady.value = null;
      remoteReady.value = null;
      remoteFiles.value = null;
      showNotice(t("rdp.clipboardFiles.saved"));
      void rpc("rdp/clipboard-files/clear", { sessionId: sid }).catch(() => undefined);
      return true;
    } catch (cause) {
      if (handleId && typeof bridge.cancel === "function") {
        await bridge.cancel(handleId).catch(() => undefined);
      }
      fail(cause);
      return false;
    } finally {
      busy.value = false;
    }
  }

  async function dismissRemoteFiles(): Promise<void> {
    const sid = sessionId.value;
    remoteFiles.value = null;
    remoteReady.value = null;
    archiveReady.value = null;
    progress.value = null;
    error.value = "";
    if (sid) await rpc("rdp/clipboard-files/clear", { sessionId: sid }).catch(() => undefined);
  }

  function handleEvent(method: string, params: Record<string, unknown>): void {
    if (!params || params.sessionId !== sessionId.value) return;
    const kind = method.slice(method.lastIndexOf("/") + 1);
    switch (kind) {
      case "available": {
        const files = remoteFilesFrom(params.files);
        remoteFiles.value = files.length ? { files, totalBytes: numeric(params.totalBytes) } : null;
        // A new remote copy supersedes any previously prepared archive.
        archiveReady.value = null;
        remoteReady.value = null;
        if (!busy.value) progress.value = null;
        break;
      }
      case "progress": {
        const done = numeric(params.bytesDone);
        const total = numeric(params.bytesTotal);
        progress.value = {
          phase: "downloading",
          done,
          total,
          percent: percentOf(done, total),
          filesDone: numeric(params.filesDone),
          filesTotal: numeric(params.filesTotal),
        };
        break;
      }
      case "ready": {
        const name = typeof params.archiveName === "string" ? params.archiveName : "";
        const pasteboard = params.pasteboard === true;
        const fileCount = numeric(params.fileCount);
        remoteReady.value = { fileCount, pasteboard };
        archiveReady.value = name ? { name, size: numeric(params.archiveSize), fileCount } : null;
        if (pasteboard) showNotice(t("rdp.clipboardFiles.pasteboardReady"));
        else if (typeof params.pasteboardError === "string" && params.pasteboardError) showNotice(t("rdp.clipboardFiles.pasteboardFailed"));
        // The final 100% progress event precedes ready; do not leave a stale
        // "Receiving…" bar. A save already in flight keeps its own progress.
        if (!busy.value) progress.value = null;
        break;
      }
      case "error": {
        const message =
          typeof params.message === "string" && params.message
            ? params.message
            : t("rdp.clipboardFiles.error");
        error.value = message;
        progress.value = null;
        showError(new Error(message), "terminal");
        break;
      }
      default:
        break;
    }
  }

  async function restoreStatus(): Promise<void> {
    const sid = sessionId.value;
    if (!sid || !active() || !enabled()) return;
    try {
      const status = await rpc<ClipboardStatusResult>("rdp/clipboard-files/status", { sessionId: sid });
      if (sessionId.value !== sid || !status) return;
      nativeSupported.value = status.nativeFiles === true;
      // A not-yet-ready CLIPRDR channel cannot decide this: leave it unknown
      // and let the clipboard-status event fill it in.
      if (status.fileClipboardReady === true) {
        setServerCapability(status.serverFiles === true);
      }
      const files = remoteFilesFrom(status.remoteFiles);
      remoteFiles.value = files.length
        ? { files, totalBytes: numeric(status.remoteTotalBytes) }
        : null;
      archiveReady.value =
        status.archiveReady && typeof status.archiveName === "string" && status.archiveName
          ? {
              name: status.archiveName,
              size: numeric(status.archiveSize),
              fileCount: numeric(status.fileCount),
            }
          : null;
      remoteReady.value =
        status.completed === true
          ? {
              fileCount: numeric(status.fileCount),
              pasteboard: status.pasteboard === true,
            }
          : null;
      if (status.downloading === true) {
        const done = numeric(status.bytesDone);
        const total = numeric(status.bytesTotal);
        progress.value = {
          phase: "downloading",
          done,
          total,
          percent: percentOf(done, total),
          filesDone: numeric(status.filesDone),
          filesTotal: numeric(status.filesTotal),
        };
      }
      if (typeof status.error === "string" && status.error) error.value = status.error;
    } catch {
      // Status is best-effort: older hosts/tests may not implement it.
    }
  }

  function startDropListener(): void {
    if (!statusRestored && active()) {
      statusRestored = true;
      void restoreStatus();
    }
    if (dropStarted) return;
    const bridge = hostFileTransfer();
    if (!bridge || typeof bridge.onDrop !== "function") return;
    try {
      const unsubscribe = bridge.onDrop((files) => {
        if (!active() || !enabled()) return;
        void sendFiles(files);
      });
      dropUnsubscribe = typeof unsubscribe === "function" ? unsubscribe : () => undefined;
      // Only mark started after a successful subscription; a bridge that
      // throws can be retried on the next running session.
      dropStarted = true;
    } catch {
      dropUnsubscribe = null;
    }
  }

  function stopDropListener(): void {
    dropStarted = false;
    const unsubscribe = dropUnsubscribe;
    dropUnsubscribe = null;
    unsubscribe?.();
  }

  function reset(): void {
    progress.value = null;
    remoteFiles.value = null;
    archiveReady.value = null;
    remoteReady.value = null;
    nativeSupported.value = false;
    serverFiles.value = false;
    serverFilesKnown.value = false;
    error.value = "";
    statusRestored = false;
    downloadInFlight = false;
  }

  /** Remote file clipboard capability from status or clipboard-status. */
  function setServerCapability(supported: boolean): void {
    serverFiles.value = supported;
    serverFilesKnown.value = true;
  }

  if (getCurrentScope()) onScopeDispose(stopDropListener);

  return {
    busy,
    progress,
    remoteFiles,
    archiveReady,
    remoteReady,
    nativeSupported,
    serverFiles,
    serverFilesKnown,
    setServerCapability,
    error,
    sendFiles,
    startRemoteDownload,
    saveRemoteArchive,
    dismissRemoteFiles,
    handleEvent,
    startDropListener,
    stopDropListener,
    reset,
  };
}