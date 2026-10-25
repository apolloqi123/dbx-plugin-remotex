/** 宿主转发原生拖拽坐标时，DBX 0.6.x 在 macOS 上把 wry 的逻辑点又除了一次
 *  `devicePixelRatio`，再减去 iframe 在窗口中的偏移：插件收到的是
 *  `(真实点 + iframe 偏移) / scale − iframe 偏移`。真实指针可由 sidecar 的
 *  光标探针取得，于是同一时刻的 (光标, 宿主点) 能解出 iframe 偏移，
 *  之后宿主点即可线性换算回 iframe 坐标。
 *
 *  见 backend/src/platform/pointer.rs（只读光标与窗口几何，不读内容）。 */

export interface HostPointerCursor {
  x: number;
  y: number;
}

export interface HostPointerWindow {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface HostPointerSample {
  platform: string;
  cursor: HostPointerCursor | null;
  window: HostPointerWindow | null;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function readPoint(value: unknown): HostPointerCursor | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (!finiteNumber(record.x) || !finiteNumber(record.y)) return null;
  return { x: record.x, y: record.y };
}

function readWindow(value: unknown): HostPointerWindow | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (
    !finiteNumber(record.x) ||
    !finiteNumber(record.y) ||
    !finiteNumber(record.width) ||
    !finiteNumber(record.height) ||
    record.width <= 0 ||
    record.height <= 0
  ) {
    return null;
  }
  return { x: record.x, y: record.y, width: record.width, height: record.height };
}

/** 读取一次光标/宿主窗口几何；旧宿主、非桌面 or 探针失败时为 null。 */
export async function sampleHostPointer(): Promise<HostPointerSample | null> {
  const api = typeof window === "undefined" ? undefined : window.dbxPlugin;
  if (!api?.invoke) return null;
  try {
    const raw = await api.invoke<unknown>("local/pointer/sample", {});
    if (!raw || typeof raw !== "object") return null;
    const record = raw as Record<string, unknown>;
    return {
      platform: typeof record.platform === "string" ? record.platform : "",
      cursor: readPoint(record.cursor),
      window: readWindow(record.window),
    };
  } catch {
    return null;
  }
}
