import type { Ref } from "vue";
import type {
  ClipboardTransferProgress,
  RemoteClipboardFile,
} from "./clipboardFiles";

/** RDP CLIPRDR file clipboard (host ↔ remote) wire and API types. */

export interface RdpFileClipboardOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  sessionId: Ref<string | null>;
  /** Session phase gate: true only while the desktop is running and reachable. */
  active: () => boolean;
  /** rdp_clipboard config; defaults to enabled. */
  enabled?: () => boolean;
}

export interface RdpFileClipboardApi {
  busy: Ref<boolean>;
  progress: Ref<ClipboardTransferProgress | null>;
  remoteFiles: Ref<{ files: RemoteClipboardFile[]; totalBytes: number } | null>;
  archiveReady: Ref<{ name: string; size: number; fileCount: number } | null>;
  /** Remote files finished downloading (and, on platforms with a file
   *  clipboard, were placed on it so the file manager can paste them). */
  remoteReady: Ref<{ fileCount: number; pasteboard: boolean } | null>;
  /** Sidecar can read/write the host file clipboard (native Ctrl+C/V). */
  nativeSupported: Ref<boolean>;
  /** Remote server accepted CLIPRDR file streams (Windows servers). */
  serverFiles: Ref<boolean>;
  /** The remote file capability is known (CLIPRDR Ready or unavailable). */
  serverFilesKnown: Ref<boolean>;
  /** Updates the remote file capability from status / clipboard-status. */
  setServerCapability(supported: boolean): void;
  error: Ref<string>;
  sendFiles(files: unknown[]): Promise<boolean>;
  /** Metadata is available; explicitly start remote→host transfer. */
  startRemoteDownload(): Promise<boolean>;
  saveRemoteArchive(): Promise<boolean>;
  dismissRemoteFiles(): Promise<void>;
  handleEvent(method: string, params: Record<string, unknown>): void;
  startDropListener(): void;
  stopDropListener(): void;
  /** Session change hook: drop refs without touching the host bridge. */
  reset(): void;
}

export interface HostFileTransfer {
  pick?(options?: { accept?: string; multiple?: boolean }): Promise<unknown>;
  read?(handleId: string, offset: number, length?: number): Promise<{ dataBase64?: string; length?: number; eof?: boolean }>;
  onDrop?(listener: (files: unknown[], drop?: { dropId?: string; truncated?: boolean }) => void): () => void;
  beginSave?(options: { name: string; contentType?: string; size?: number }): Promise<{ handleId: string; chunkBytes?: number } | null>;
  write?(handleId: string, offset: number, data: Uint8Array): Promise<{ written?: number; nextOffset?: number }>;
  finish?(handleId: string): Promise<void>;
  cancel?(handleId?: string): Promise<void>;
}

export interface BeginResult {
  uploadId: string;
  chunkBytes?: number;
}

export interface ClipboardReadResult {
  dataBase64?: string;
  eof?: boolean;
  total?: number;
  archiveName?: string;
}

export interface ClipboardStatusResult {
  remoteFiles?: unknown;
  remoteTotalBytes?: number;
  archiveReady?: boolean;
  archiveName?: string;
  archiveSize?: number;
  fileCount?: number;
  downloading?: boolean;
  filesDone?: number;
  filesTotal?: number;
  bytesDone?: number;
  bytesTotal?: number;
  completed?: boolean;
  pasteboard?: boolean;
  /** Host file clipboard availability (macOS / Windows / Linux helper). */
  nativeFiles?: boolean;
  /** Remote server accepts CLIPRDR file streams. */
  serverFiles?: boolean;
  /** CLIPRDR reached Ready (false means the capability is still unknown). */
  fileClipboardReady?: boolean;
  error?: string;
}
