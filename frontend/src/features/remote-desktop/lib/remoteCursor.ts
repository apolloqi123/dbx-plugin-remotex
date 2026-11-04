// 远程桌面服务端光标：RDP/VNC 共用同一事件形状（default/hidden/position/
// bitmap）与 CSS cursor 转换，避免两套画布各写一份 bitmap → PNG 逻辑。
import type { RdpPointerEvent } from "./rdpFrame";

export interface RemoteCursorState {
  /** false = position-only event, keep the current cursor. */
  changed: boolean;
  cursor: string;
}

/** Cursor bitmaps are capped at 128 px (WebKit's usual custom-cursor limit). */
const MAX_CURSOR_SIZE = 128;
const CURSOR_CACHE_LIMIT = 32;
const cursorCache = new Map<string, string | null>();

export function cursorFromPointer(pointer: RdpPointerEvent): RemoteCursorState {
  if (pointer.type === "position") return { changed: false, cursor: "" };
  if (pointer.type === "default") return { changed: true, cursor: "" };
  if (pointer.type === "hidden") return { changed: true, cursor: "none" };
  const url = bitmapCursorUrl(pointer);
  return { changed: true, cursor: url ? `url(${url}) ${Math.max(0, pointer.hotspotX)} ${Math.max(0, pointer.hotspotY)}, default` : "" };
}

/** bitmap 光标：原始 RGBA → canvas → PNG data URL（CSS cursor 只吃图片 URL）。 */
function bitmapCursorUrl(pointer: Extract<RdpPointerEvent, { type: "bitmap" }>): string | null {
  if (pointer.width <= 0 || pointer.height <= 0 || pointer.width > MAX_CURSOR_SIZE || pointer.height > MAX_CURSOR_SIZE) return null;
  const key = `${pointer.width}x${pointer.height}:${pointer.rgbaBase64}`;
  if (cursorCache.has(key)) return cursorCache.get(key) ?? null;
  const url = encodeBitmapCursor(pointer);
  if (cursorCache.size >= CURSOR_CACHE_LIMIT) {
    const oldest = cursorCache.keys().next().value;
    if (oldest !== undefined) cursorCache.delete(oldest);
  }
  cursorCache.set(key, url);
  return url;
}

function encodeBitmapCursor(pointer: Extract<RdpPointerEvent, { type: "bitmap" }>): string | null {
  let bytes: Uint8Array;
  try {
    bytes = window.dbxPlugin.decodeBase64(pointer.rgbaBase64);
  } catch {
    return null;
  }
  const expected = pointer.width * pointer.height * 4;
  if (bytes.byteLength < expected) return null;
  const canvas = document.createElement("canvas");
  canvas.width = pointer.width;
  canvas.height = pointer.height;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.putImageData(new ImageData(new Uint8ClampedArray(bytes.subarray(0, expected)), pointer.width, pointer.height), 0, 0);
  return canvas.toDataURL("image/png");
}
