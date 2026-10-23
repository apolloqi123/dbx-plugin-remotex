// engineRunner（FIG 批次 2-2）：FigCompletionSource 的 inline-worker 运行层。
// 依据 roadmap 批次 2-2 与方案 §43/§44：引擎可迁入 worker 线程（`?worker&inline`
// data-URL 内联，保持单文件构建约束），宿主不支持或 worker 连续崩溃则永久
// 回落主线程直跑——调用方（CompletionController / App.vue）无感。
//
// 同步/异步取舍（冻结接缝 resolve 声明为同步签名）：
// - 主线程 fallback / 降级模式：resolve 严格同步返回 CompletionResponse | null
//   （零时序漂移，与直连 source 完全一致）。
// - worker 模式：postMessage 往返本质异步，resolve 运行时返回 Promise
//   （thenable）。这正是 CompletionController.dispatch 既有 thenable 防御
//   分支（源码注释"Worker 化留位"）的消费契约：await 后按 requestId /
//   revision / sessionId 三重 guard 交付（契约 §2.1 revision 纪律）。
//   因此 runner 侧不需要额外排队：过期的在途回执先被 pending 映射按
//   requestId 收敛（每请求恰一次结清），再由 controller 三重 guard 静默
//   丢弃；worker 崩溃时在途请求全部以 null 结清（同 pass-through 语义，
//   §44 "drop current completion"）。
//
// feature-detect：typeof Worker 存在且 inline worker 构造成功（try/catch——
// CSP 拦 data-URL worker 时构造会 throw）→ worker 模式；否则主线程直跑。

import type { CompletionResponse } from "../core/types";
import type { FigCompletionSource, FigSourceRequest } from "../fig/source";
// vite/client（tsconfig types: ["vite/client"]）已声明 `*?worker&inline`：
// 默认导出 Worker 构造器；inline = 代码以 data-URL 内嵌进主 bundle，零额外
// chunk。单测经 vi.mock 替换本 import 并注入 createWorker，绝不触发真实构造。
import EngineWorker from "./engine.worker.ts?worker&inline";
import {
  isEngineWorkerInitError,
  isEngineWorkerReady,
  isEngineWorkerResponse,
  type EngineWorkerInit,
  type EngineWorkerRequest,
} from "./engineProtocol";

// ---------------------------------------------------------------------------
// worker 线协议（engineProtocol.ts；worker 侧同源引用，避免运行时环）
// ---------------------------------------------------------------------------

export type {
  EngineWorkerInbound,
  EngineWorkerInit,
  EngineWorkerInitError,
  EngineWorkerOutbound,
  EngineWorkerReady,
  EngineWorkerRequest,
  EngineWorkerResponse,
} from "./engineProtocol";

export interface EngineRunnerOptions {
  /** 主线程引擎工厂：fallback / 永久降级路径的唯一来源（构造时即实例化，
   * 两种模式复用同一实例，保证 id 与行为一致）。 */
  createSource: () => FigCompletionSource;
  /** 测试/宿主注入点：worker 工厂；默认走 defaultSpawn 的真实 inline worker
   * feature-detect。注入工厂只需保证可构造，不受 Worker 全局探测约束。 */
  createWorker?: () => Worker | null;
  /** 语料 init 载荷（figManifestWire JSON）：提供时 worker 就绪（ready 回执）
   * 前全部 resolve 走主线程 source——inline worker 不再内嵌第二份语料。
   * 省略时（单测/自定义 worker）视为构造即就绪，行为与批次 2-2 一致。 */
  workerInit?: () => string;
}

/** resolve 的运行时返回：同步值（主线程模式）或 Promise（worker 模式）。 */
export type EngineRunnerResolveResult =
  | CompletionResponse
  | null
  | Promise<CompletionResponse | null>;

type PendingFulfill = (response: CompletionResponse | null) => void;

type WorkerFactory = () => Worker | null;

/** 默认 worker 工厂 = 真实路径的完整 feature-detect：无 Worker 全局（旧
 * WebView / 非 DOM 运行时）→ null；构造 throw（CSP 拦 data-URL worker）→
 * null。两种情况都落入主线程直跑。 */
function defaultSpawn(): Worker | null {
  if (typeof Worker === "undefined") return null;
  try {
    return new EngineWorker();
  } catch {
    return null;
  }
}

class EngineRunner {
  /** 冻结接缝的标识面：两种模式透传底层 source 的 id。 */
  readonly id: string;

  private readonly syncSource: FigCompletionSource;
  private readonly spawn: WorkerFactory;
  private readonly initPayload: (() => string) | undefined;
  private initTimer: ReturnType<typeof setTimeout> | null = null;
  private worker: Worker | null = null;
  private readonly pending = new Map<number, PendingFulfill>();
  /** 永久降级后不再触碰 worker（主线程定版）。 */
  private degraded = false;
  /** 语料 init 握手完成前 worker 不接活（resolve 走主线程 source，零等待）。 */
  private workerReady = false;
  /** §44：允许一次崩溃重启；第二次失败永久降级主线程。 */
  private restartUsed = false;

  constructor(options: EngineRunnerOptions) {
    this.syncSource = options.createSource();
    this.id = this.syncSource.id;
    this.spawn = options.createWorker ?? defaultSpawn;
    this.initPayload = options.workerInit;
    const worker = this.trySpawn();
    if (worker === null) {
      this.degraded = true; // 环境不支持 / 构造 throw（CSP）→ 主线程定版
    } else {
      this.attach(worker);
      this.worker = worker;
      this.prime(worker);
    }
  }

  resolve(request: FigSourceRequest): EngineRunnerResolveResult {
    const worker = this.worker;
    if (this.degraded || worker === null || !this.workerReady) {
      return this.syncSource.resolve(request); // 严格同步，调用方无感
    }
    return new Promise<CompletionResponse | null>((fulfill) => {
      this.pending.set(request.requestId, fulfill);
      try {
        const message: EngineWorkerRequest = {
          kind: "resolve",
          requestId: request.requestId,
          revision: request.revision,
          sessionId: request.sessionId,
          line: request.line,
          trigger: request.trigger,
        };
        worker.postMessage(message);
      } catch {
        // postMessage 本身抛错（通道已坏）：按 §44 崩溃处理，本次请求结清。
        this.pending.delete(request.requestId);
        this.onCrash();
        fulfill(null);
      }
    });
  }

  /** 构造一次 worker；工厂返回 null 或 throw（CSP 拦 data-URL）→ 视为不可用。 */
  private trySpawn(): Worker | null {
    try {
      return this.spawn();
    } catch {
      return null;
    }
  }

  /** 语料 init：worker 就绪前 resolve 走主线程（见 resolve 的 workerReady 判定）。
   * 未配置载荷 → 构造即就绪（单测/自定义 worker 的批次 2-2 行为）。 */
  private prime(worker: Worker): void {
    if (this.initPayload === undefined) {
      this.workerReady = true;
      return;
    }
    this.workerReady = false;
    // 语料序列化 ~50 ms（5 MB JSON）：推迟到当前任务之后发送，避免工作台
    // 首帧卡顿；就绪窗口内的 resolve 由主线程 source 兜底（结果一致）。
    this.cancelInit();
    this.initTimer = setTimeout(() => {
      this.initTimer = null;
      if (this.degraded || this.worker !== worker) return;
      try {
        const payload = this.initPayload;
        if (payload === undefined) return;
        const message: EngineWorkerInit = { kind: "init", manifest: payload() };
        worker.postMessage(message);
      } catch {
        // 序列化 / 通道异常：按 worker 崩溃处理（重启一次后永久降级）。
        this.onCrash();
      }
    }, 0);
  }

  private cancelInit(): void {
    if (this.initTimer !== null) {
      clearTimeout(this.initTimer);
      this.initTimer = null;
    }
  }

  /** 事件绑定带代际校验：被取代的旧 worker 的迟到事件一律忽略。 */
  private attach(worker: Worker): void {
    worker.onmessage = (event: MessageEvent<unknown>) => {
      if (this.worker !== worker) return;
      this.onWorkerMessage(event.data);
    };
    worker.onerror = () => {
      if (this.worker !== worker) return;
      this.onCrash();
    };
    worker.onmessageerror = () => {
      if (this.worker !== worker) return;
      this.onCrash();
    };
  }

  private onWorkerMessage(data: unknown): void {
    if (isEngineWorkerReady(data)) {
      this.workerReady = true; // 语料装载完成：后续 resolve 走 worker
      return;
    }
    if (isEngineWorkerInitError(data)) {
      this.onCrash(); // init 失败 = 崩溃路径（重启一次后永久降级）
      return;
    }
    if (!isEngineWorkerResponse(data)) return; // 未知消息：静默忽略（双保险）
    const fulfill = this.pending.get(data.requestId);
    if (fulfill === undefined) return; // 重复/过期回执：忽略
    this.pending.delete(data.requestId);
    fulfill(data.response ?? null);
  }

  /** §44 Worker Crash Recovery：丢弃在途 → 重启一次 → 再失败永久降级。 */
  private onCrash(): void {
    if (this.degraded || this.worker === null) return;
    this.dropPending();
    const dead = this.worker;
    this.worker = null;
    this.workerReady = false;
    try {
      dead.terminate();
    } catch {
      // 崩溃 worker 的终止失败不得阻断恢复路径
    }
    if (this.restartUsed) {
      this.degrade();
      return;
    }
    this.restartUsed = true;
    const replacement = this.trySpawn();
    if (replacement === null) {
      this.degrade(); // 重启构造失败（如 CSP）：直接永久降级
      return;
    }
    this.attach(replacement);
    this.worker = replacement;
    this.prime(replacement); // 重启的 worker 同样需要重新装载语料
  }

  /** 丢弃在途（§44 step 1）：全部以 null 结清 → controller 侧转 pass-through，
   * 表现为"当前补全丢弃、菜单关闭、PTY 输入链路无损"。 */
  private dropPending(): void {
    const entries = [...this.pending.values()];
    this.pending.clear();
    for (const fulfill of entries) {
      try {
        fulfill(null);
      } catch {
        // 结清回调异常不得阻断降级
      }
    }
  }

  /** 终结 runner：在途请求以 null 结清、worker 终止、永久转主线程直跑。
   * 供 composable 卸载回收——inline worker 无 GC 兜底，不 dispose 会在
   * 工作台重挂载（webview 重建/HMR）时累积常驻线程。dispose 后仍可安全
   * resolve（退化为 syncSource 直跑）。 */
  dispose(): void {
    this.degraded = true;
    this.cancelInit();
    this.dropPending();
    const worker = this.worker;
    this.worker = null;
    if (worker === null) return;
    try {
      worker.terminate();
    } catch {
      // 终止失败不影响回收语义
    }
  }

  private degrade(): void {
    this.degraded = true;
    this.cancelInit();
    this.worker = null;
    this.workerReady = false;
  }
}

/**
 * 构造引擎 runner。返回值满足冻结接缝 FigCompletionSource（含主线程同步
 * 语义）；worker 模式下 resolve 运行时返回 Promise（thenable），由
 * CompletionController.dispatch 的既有 thenable 防御分支消费（见文件头
 * "同步/异步取舍"）——此处是全仓库唯一的类型桥接点。额外暴露 dispose()
 * 供消费方在组件卸载时回收 worker。
 */
export function createEngineRunner(options: EngineRunnerOptions): FigCompletionSource & { dispose(): void } {
  const runner = new EngineRunner(options);
  return runner as unknown as FigCompletionSource & { dispose(): void };
}
