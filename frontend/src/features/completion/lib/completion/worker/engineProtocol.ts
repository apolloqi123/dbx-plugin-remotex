// engineRunner / engine.worker 共享的线协议（批次 2-2 + 语料去重 init 握手）。
// 独立模块避免 worker ↔ runner 的运行时环（engineRunner 含 `?worker&inline`
// 构造侧引用，worker 侧只允许 type/guard 依赖）。

import type { CompletionResponse } from "../core/types";
import type { FigSourceRequest } from "../fig/source";

/** 宿主 → worker：解析请求（字段与冻结 FigSourceRequest 一一对应）。 */
export interface EngineWorkerRequest {
  kind: "resolve";
  requestId: number;
  revision: number;
  sessionId: string;
  line: string;
  trigger: FigSourceRequest["trigger"];
}

/** worker → 宿主：解析回执。response 为 null 等价 pass-through 空响应。 */
export interface EngineWorkerResponse {
  kind: "response";
  requestId: number;
  revision: number;
  sessionId: string;
  response: CompletionResponse | null;
}

/** 宿主 → worker：语料数据面（figManifestWire 序列化；函数值转标记）。
 * 语料静态副本只在主线程，worker 不再内嵌第二份。 */
export interface EngineWorkerInit {
  kind: "init";
  manifest: string;
}

/** worker → 宿主：语料装载完成，resolve 通道可用。 */
export interface EngineWorkerReady {
  kind: "ready";
}

/** worker → 宿主：语料装载失败（畸形载荷/构造异常）——宿主按崩溃路径重启
 * 一次，再失败永久降级主线程。 */
export interface EngineWorkerInitError {
  kind: "init-error";
  message?: string;
}

export type EngineWorkerInbound = EngineWorkerInit | EngineWorkerRequest;
export type EngineWorkerOutbound = EngineWorkerResponse | EngineWorkerReady | EngineWorkerInitError;

export function isEngineWorkerRequest(data: unknown): data is EngineWorkerRequest {
  if (typeof data !== "object" || data === null) return false;
  const candidate = data as Partial<EngineWorkerRequest>;
  return candidate.kind === "resolve" && typeof candidate.requestId === "number";
}

export function isEngineWorkerResponse(data: unknown): data is EngineWorkerResponse {
  if (typeof data !== "object" || data === null) return false;
  const candidate = data as Partial<EngineWorkerResponse>;
  return (
    candidate.kind === "response" &&
    typeof candidate.requestId === "number" &&
    typeof candidate.revision === "number" &&
    typeof candidate.sessionId === "string"
  );
}

export function isEngineWorkerInit(data: unknown): data is EngineWorkerInit {
  if (typeof data !== "object" || data === null) return false;
  const candidate = data as Partial<EngineWorkerInit>;
  return candidate.kind === "init" && typeof candidate.manifest === "string";
}

export function isEngineWorkerReady(data: unknown): data is EngineWorkerReady {
  return typeof data === "object" && data !== null && (data as Partial<EngineWorkerReady>).kind === "ready";
}

export function isEngineWorkerInitError(data: unknown): data is EngineWorkerInitError {
  return typeof data === "object" && data !== null && (data as Partial<EngineWorkerInitError>).kind === "init-error";
}
