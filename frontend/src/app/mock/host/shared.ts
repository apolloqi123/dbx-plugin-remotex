/**
 * 可视化夹具（mockDbxHost）的共享桥面工具：监听器集合、事件/二进制投递、
 * 终端帧与编码、字节载荷解码，以及 invoke 域处理器共用的 UNHANDLED 哨兵。
 *
 * 这些模块只服务于 dev/test 夹具，不进入插件运行时；对外公开面仍是
 * app/mockDbxHost.ts 的 window.dbxPlugin 安装副作用。
 */

export const eventListeners = new Set<(event: DbxPluginEvent) => void>();
export const binaryListeners = new Set<(event: DbxPluginBinaryEvent) => void>();
export const appearanceListeners = new Set<(appearance: DbxPluginAppearance) => void>();
export const contextListeners = new Set<(context: Record<string, unknown>) => void>();

export function emitHostEvent(event: DbxPluginEvent): void {
  for (const listener of eventListeners) listener(event);
}

export function emitHostBinary(event: DbxPluginBinaryEvent): void {
  for (const listener of binaryListeners) listener(event);
}

export function terminalFrame(sequence: number, text: string) {
  const data = new TextEncoder().encode(text);
  const frame = new Uint8Array(9 + data.length);
  frame[0] = 0;
  new DataView(frame.buffer).setBigUint64(1, BigInt(sequence), false);
  frame.set(data, 9);
  return frame;
}

export function base64(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** 桥投递的二进制/字符串载荷归一（与真实宿主 sendBinary 入参形态一致）。 */
export function toBytes(data: Uint8Array | ArrayBuffer | string): Uint8Array {
  return typeof data === "string"
    ? Uint8Array.from(atob(data), (value) => value.charCodeAt(0))
    : data instanceof Uint8Array
      ? data
      : new Uint8Array(data);
}

/** invoke 域处理器的“未命中”哨兵：父入口据此继续找下一个域。 */
export const UNHANDLED: unique symbol = Symbol("mockDbxHost.unhandled");

export type Unhandled = typeof UNHANDLED;
