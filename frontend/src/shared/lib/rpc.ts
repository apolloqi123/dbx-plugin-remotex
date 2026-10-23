// 宿主 RPC 的薄包装（收敛 invoke 的可用性判断与错误形态）。
//
// 背景（review）：全仓 ~200 处 `window.dbxPlugin.invoke(...)` 直接触达全局，
// 宿主桥缺失时是一句 TypeError（"Cannot read properties of undefined"），
// 排查成本高且不可区分「桥不在」与「调用失败」。rpc<T>() 只做两件事：
// - 桥缺失 → 拒绝并给出明确错误类型（RpcUnavailableError）；
// - 其余参数/超时原样透传（不改变任何 RPC 字符串与调用语义）。
//
// Wave 2 已迁移 65 处（含 VNC 会话与 SFTP 下载/上传链）；其余 ~144 处仍直连
// invoke，主要集中在 App.vue / remote-desktop / completion，后续按域迁移。

/** 宿主 RPC 超时选项（invoke 第三参）。 */
export interface RpcOptions {
  timeoutMs?: number;
}

/** 宿主桥不可用（非插件 webview / 旧宿主 / 测试环境未注入）。 */
export class RpcUnavailableError extends Error {
  readonly method: string;

  constructor(method: string) {
    super(`dbxPlugin bridge is unavailable (rpc: ${method})`);
    this.name = "RpcUnavailableError";
    this.method = method;
  }
}

/** 当前 webview 是否挂载了宿主桥（SSR/纯浏览器预览为 false）。 */
export function rpcAvailable(): boolean {
  if (typeof window === "undefined") return false;
  const api = (window as { dbxPlugin?: { invoke?: unknown } }).dbxPlugin;
  return typeof api?.invoke === "function";
}

/** 调用宿主 RPC：桥缺失时以 RpcUnavailableError 拒绝，其余原样透传。 */
export function rpc<T = unknown>(method: string, params?: unknown, options?: RpcOptions): Promise<T> {
  if (!rpcAvailable()) return Promise.reject(new RpcUnavailableError(method));
  // 未传 options 时保持两参调用，与迁移前直连 invoke 的实参形状一致。
  return options === undefined
    ? window.dbxPlugin.invoke<T>(method, params)
    : window.dbxPlugin.invoke<T>(method, params, options);
}
