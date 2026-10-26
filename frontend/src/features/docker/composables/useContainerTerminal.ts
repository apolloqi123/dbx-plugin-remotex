// 容器 PTY 终端的宿主桥接线：open/close/resize、二进制输入输出、状态事件。
// xterm 实例与视图在组件里；这里只管协议与生命周期，便于单测。
//
// 与 ssh 终端不同，容器终端的 terminalId 由客户端预生成并随 open 下发，
// 因此 onBinary 必须先于 open RPC 订阅：sidecar 在 open 返回前就可能把首屏
// 输出/提示符写进 `docker/terminal/out/{terminalId}`。
import { ref, type Ref } from "vue";
import { bridgeBinaryBytes } from "../../../../../shared/frontend/binaryEvent";
import { randomUUID } from "../../../shared/lib/uuid";
import { createTerminalInputQueue } from "../../terminal/lib/emulator/terminalInputQueue";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";

export type ContainerTerminalStatus = "idle" | "connecting" | "connected" | "exited" | "error" | "disabled";

export interface ContainerTerminalStateEvent {
  terminalId?: string;
  state?: string;
  exitCode?: number;
  error?: string;
}

export interface UseContainerTerminalOptions {
  containerId: () => string;
  sessionId: () => string | undefined;
  connectionId: () => string | undefined;
  canWrite: () => boolean;
  running: () => boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  /** Raw PTY bytes; the component writes them to xterm. */
  onData: (data: Uint8Array) => void;
  onState?: (state: ContainerTerminalStatus, detail?: { exitCode?: number; error?: string }) => void;
  /** Test seam; production uses the lib/uuid shim. */
  createTerminalId?: () => string;
}

export interface UseContainerTerminal {
  status: Ref<ContainerTerminalStatus>;
  errorMessage: Ref<string>;
  open(cols: number, rows: number): Promise<void>;
  close(): Promise<void>;
  retry(cols: number, rows: number): Promise<void>;
  sendInput(data: string): void;
  resize(cols: number, rows: number): void;
  handleStateEvent(event: ContainerTerminalStateEvent): void;
}

const OPEN_TIMEOUT_MS = 35_000;
const INPUT_SEND_TIMEOUT_MS = 8_000;
/** close 等待 runner 释放 channel 的上限（后端 2s + 桥开销）。 */
const CLOSE_TIMEOUT_MS = 4_000;
const OUTPUT_CHANNEL_PREFIX = "docker/terminal/out/";
const INPUT_CHANNEL_PREFIX = "docker/terminal/in/";

export function useContainerTerminal(options: UseContainerTerminalOptions): UseContainerTerminal {
  const status = ref<ContainerTerminalStatus>("idle");
  const errorMessage = ref("");

  let terminalId = "";
  // 只有 open 成功过的 terminalId 才值得 close：打开失败时后端未保留槽位，
  // 再发一次 close 既多余也会让调用方多一条 RPC。
  let live = false;
  let generation = 0;
  let unsubscribeBinary: (() => void) | undefined;
  let stopped = true;
  let lastResizeCols = 0;
  let lastResizeRows = 0;
  /** 后端 resize 队列满时回 false：最多补发一次，避免静默卡在旧尺寸。 */
  let resizeRetries = 0;
  /** 最近一次 close RPC；后续 open 必须等它完成再建新 channel。 */
  let pendingClose: Promise<void> = Promise.resolve();

  const encoder = new TextEncoder();

  const inputQueue = createTerminalInputQueue({
    send: (id, payload) => {
      const api = window.dbxPlugin;
      if (!api) return Promise.resolve();
      return api.sendBinary(`${INPUT_CHANNEL_PREFIX}${id}`, payload);
    },
    sendTimeoutMs: INPUT_SEND_TIMEOUT_MS,
    onError: () => inputQueue.reset(),
  });

  function newTerminalId(): string {
    return options.createTerminalId ? options.createTerminalId() : randomUUID();
  }

  function setStatus(next: ContainerTerminalStatus, detail?: { exitCode?: number; error?: string }): void {
    status.value = next;
    options.onState?.(next, detail);
  }

  function stopListening(): void {
    unsubscribeBinary?.();
    unsubscribeBinary = undefined;
  }

  /** 释放本实例持有的一切：递增代际、退订、清空输入队列、关闭远端 PTY。
   *  返回的 Promise 在 close RPC 返回后完成（后端会等 channel 释放），
   *  open 必须先 await 它，避免旧 channel 未关就开新会话触发 MaxSessions。 */
  function teardown(): Promise<void> {
    generation += 1;
    stopped = true;
    stopListening();
    inputQueue.reset();
    const target = live ? terminalId : "";
    terminalId = "";
    live = false;
    lastResizeCols = 0;
    lastResizeRows = 0;
    resizeRetries = 0;
    // No new target, but an earlier close may still be in flight.
    if (!target) return pendingClose;
    const api = window.dbxPlugin;
    if (!api) return pendingClose;
    pendingClose = api
      .invoke("docker/terminal/close", { terminalId: target }, { timeoutMs: CLOSE_TIMEOUT_MS })
      .then(() => undefined)
      .catch(() => undefined);
    return pendingClose;
  }

  function handleBinary(event: DbxPluginBinaryEvent): void {
    if (stopped || !terminalId) return;
    if (event.channel !== `${OUTPUT_CHANNEL_PREFIX}${terminalId}`) return;
    const api = window.dbxPlugin;
    if (!api) return;
    try {
      const bytes = bridgeBinaryBytes(event, api.decodeBase64);
      if (bytes.byteLength) options.onData(bytes);
    } catch {
      // A malformed frame must not kill the session; the next frame recovers.
    }
  }

  async function open(cols: number, rows: number): Promise<void> {
    await teardown();
    const current = generation;
    stopped = false;
    errorMessage.value = "";

    const api = window.dbxPlugin;
    if (!options.canWrite()) {
      const message = options.t("docker.terminalReadOnly");
      errorMessage.value = message;
      setStatus("disabled", { error: message });
      return;
    }
    if (!options.running()) {
      const message = options.t("docker.terminalNotRunning");
      errorMessage.value = message;
      setStatus("disabled", { error: message });
      return;
    }
    if (!api) {
      const message = options.t("docker.terminalOpenFailed", { error: "host bridge unavailable" });
      errorMessage.value = message;
      setStatus("error", { error: message });
      return;
    }

    terminalId = newTerminalId();
    const spawnCols = Math.max(1, Math.floor(cols));
    const spawnRows = Math.max(1, Math.floor(rows));
    lastResizeCols = spawnCols;
    lastResizeRows = spawnRows;
    setStatus("connecting");
    unsubscribeBinary = api.onBinary(handleBinary);

    try {
      const sessionId = options.sessionId();
      const params: Record<string, unknown> = {
        terminalId,
        containerId: options.containerId(),
        cols: spawnCols,
        rows: spawnRows,
        ...dockerEngineParams(loadDockerEngineSettings(options.connectionId() || "")),
        ...(sessionId ? { sessionId } : { target: "local" }),
      };
      const result = await api.invoke<{ terminalId?: string }>("docker/terminal/open", params, {
        timeoutMs: OPEN_TIMEOUT_MS,
      });
      if (current !== generation || stopped) {
        // close()/新 open 已取代本次打开：后端可能已建好 PTY，不能让它泄漏。
        const created = result?.terminalId;
        if (created) void api.invoke("docker/terminal/close", { terminalId: created }).catch(() => undefined);
        return;
      }
      if (!result?.terminalId) throw new Error("missing terminalId");
      // 返回的 id 是权威值；订阅按当前 terminalId 过滤，无需重新订阅。
      terminalId = result.terminalId;
      live = true;
      setStatus("connected");
    } catch (cause) {
      if (current !== generation) return;
      // The bridge may have created the PTY before the response was lost:
      // close the speculative id so a timed-out open cannot hold a slot.
      const failedId = terminalId;
      const message = cause instanceof Error && cause.message ? cause.message : String(cause);
      errorMessage.value = options.t("docker.terminalOpenFailed", { error: message });
      setStatus("error", { error: errorMessage.value });
      stopListening();
      terminalId = "";
      live = false;
      if (failedId) {
        void api
          .invoke("docker/terminal/close", { terminalId: failedId }, { timeoutMs: CLOSE_TIMEOUT_MS })
          .catch(() => undefined);
      }
    }
  }

  function close(): Promise<void> {
    const closing = teardown();
    if (status.value !== "idle") status.value = "idle";
    return closing;
  }

  async function retry(cols: number, rows: number): Promise<void> {
    await open(cols, rows);
  }

  function sendInput(data: string): void {
    if (stopped || status.value !== "connected" || !terminalId || !data) return;
    // xterm 的原始按键字节交给输入队列封装 8 字节序号（与 ssh/local 终端同形）。
    inputQueue.enqueue(terminalId, encoder.encode(data));
  }

  function resize(cols: number, rows: number): void {
    if (stopped || status.value !== "connected" || !terminalId) return;
    if (!Number.isFinite(cols) || !Number.isFinite(rows)) return;
    const nextCols = Math.max(1, Math.floor(cols));
    const nextRows = Math.max(1, Math.floor(rows));
    // ResizeObserver 的初始回调常在 open 后补发同尺寸；同尺寸一律不发，
    // 否则远端 shell 收到 window-change 会重绘提示符。
    if (nextCols === lastResizeCols && nextRows === lastResizeRows) return;
    lastResizeCols = nextCols;
    lastResizeRows = nextRows;
    const api = window.dbxPlugin;
    if (!api) return;
    void api
      .invoke<{ resized?: boolean }>("docker/terminal/resize", { terminalId, cols: nextCols, rows: nextRows })
      .then((result) => {
        if (result?.resized !== false) {
          resizeRetries = 0;
          return;
        }
        // 队列满（runner 正在冲刷输出）：清掉去重标记后补发一次最新尺寸。
        if (resizeRetries >= 1 || stopped || !terminalId) return;
        resizeRetries += 1;
        lastResizeCols = 0;
        lastResizeRows = 0;
        setTimeout(() => {
          if (!stopped && terminalId) resize(nextCols, nextRows);
        }, 150);
      })
      .catch(() => undefined);
  }

  function handleStateEvent(event: ContainerTerminalStateEvent): void {
    if (!terminalId || event.terminalId !== terminalId) return;
    if (event.state === "error") {
      errorMessage.value = event.error?.trim() || options.t("docker.terminalEnded");
      setStatus("error", { error: errorMessage.value });
      stopListening();
      return;
    }
    if (event.state === "exited") {
      setStatus("exited", { exitCode: typeof event.exitCode === "number" ? event.exitCode : undefined });
      stopListening();
    }
  }

  return { status, errorMessage, open, close, retry, sendInput, resize, handleStateEvent };
}
