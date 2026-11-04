/** Clipboard file payloads shared by the RDP upload/save pipelines. */

export interface ClipboardFileMeta {
  name: string;
  relativePath?: string;
  size: number;
}

/** Validated host handle: metadata is safe to forward, handleId reads the bytes. */
export interface HostClipboardFile extends ClipboardFileMeta {
  handleId: string;
  contentType?: string;
}

export interface RemoteClipboardFile {
  name: string;
  relativePath?: string;
  size: number;
  directory: boolean;
}

export interface ClipboardTransferProgress {
  phase: "uploading" | "downloading" | "saving";
  /** Bytes transferred / total bytes for the whole batch. */
  done: number;
  total: number;
  percent: number;
  filesDone?: number;
  filesTotal?: number;
}

export const CLIPBOARD_FILE_LIMIT = 2000;
export const CLIPBOARD_TOTAL_BYTES_LIMIT = 512 * 1024 * 1024;
export const CLIPBOARD_DEFAULT_CHUNK_BYTES = 1024 * 1024;

/** 32 KiB keeps the fallback spread below engine argument limits. */
const BASE64_CHUNK_BYTES = 0x8000;

function cleanSegment(value: string): string {
  return value.replace(/[\u0000-\u001f\u007f]/g, "").trim();
}

/** Host names are basenames; tolerate a full path by keeping the last segment. */
function safeName(value: unknown): string {
  if (typeof value !== "string") return "";
  const parts = value.replace(/\\/g, "/").split("/");
  const base = cleanSegment(parts[parts.length - 1] ?? "");
  return base === "." || base === ".." ? "" : base;
}

/** Relative paths rebuild folders remotely: drop absolute/`.`/`..` segments. */
function safeRelativePath(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const segments = value
    .replace(/\\/g, "/")
    .split("/")
    .map(cleanSegment)
    .filter((segment) => segment && segment !== "." && segment !== "..");
  return segments.length ? segments.join("/") : undefined;
}

function safeSize(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0;
}

/** Validate host pick/drop handles before they reach the remote clipboard. */
export function normalizeHostFiles(files: readonly unknown[]): HostClipboardFile[] {
  const normalized: HostClipboardFile[] = [];
  for (const value of files) {
    if (!value || typeof value !== "object") continue;
    const record = value as Record<string, unknown>;
    const handleId = typeof record.handleId === "string" ? record.handleId.trim() : "";
    const name = safeName(record.name);
    if (!handleId || !name) continue;
    const relativePath = safeRelativePath(record.relativePath);
    const contentType = typeof record.contentType === "string" ? record.contentType : undefined;
    normalized.push({
      handleId,
      name,
      size: safeSize(record.size),
      ...(relativePath ? { relativePath } : {}),
      ...(contentType ? { contentType } : {}),
    });
  }
  if (normalized.length > CLIPBOARD_FILE_LIMIT) {
    throw new Error(`Too many clipboard files (${normalized.length}); the limit is ${CLIPBOARD_FILE_LIMIT}`);
  }
  const totalBytes = normalized.reduce((sum, file) => sum + file.size, 0);
  if (totalBytes > CLIPBOARD_TOTAL_BYTES_LIMIT) {
    throw new Error(`Clipboard files exceed the ${CLIPBOARD_TOTAL_BYTES_LIMIT / (1024 * 1024)} MiB limit`);
  }
  return normalized;
}

interface Base64Bridge {
  encodeBase64?: (value: Uint8Array | ArrayBuffer) => string;
  decodeBase64?: (value: string) => Uint8Array;
}

function base64Bridge(): Base64Bridge | null {
  if (typeof window === "undefined") return null;
  return (window as unknown as { dbxPlugin?: Base64Bridge }).dbxPlugin ?? null;
}

/** Host bridge first; btoa fallback processes fixed chunks (never a full spread). */
export function base64FromBytes(bytes: Uint8Array): string {
  const bridge = base64Bridge();
  if (typeof bridge?.encodeBase64 === "function") {
    try {
      return bridge.encodeBase64(bytes);
    } catch {
      // Broken bridge: fall through to the local encoder.
    }
  }
  if (typeof btoa !== "function") throw new Error("base64 encoding is unavailable");
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += BASE64_CHUNK_BYTES) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + BASE64_CHUNK_BYTES));
  }
  return btoa(binary);
}

export function bytesFromBase64(text: string): Uint8Array {
  const bridge = base64Bridge();
  if (typeof bridge?.decodeBase64 === "function") {
    try {
      const decoded = bridge.decodeBase64(text);
      if (decoded instanceof Uint8Array) return decoded;
    } catch {
      // Broken bridge: fall through to the local decoder.
    }
  }
  if (typeof atob !== "function") throw new Error("base64 decoding is unavailable");
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}