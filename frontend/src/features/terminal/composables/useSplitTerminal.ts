/**
 * Session/frame/input lifecycle for the secondary (split) terminal pane.
 *
 * The split pane owns a second PTY channel, but deliberately shares the main
 * pane's authenticated SSH transport (`reuseAuthenticatedTransport`), so a
 * bastion does not prompt for MFA again. The xterm instance itself stays in
 * the component; this composable owns everything between the host bridge and
 * that terminal: open/replay/close, ordered output frames, input queue and
 * resize notifications.
 */
import { ref, type Ref } from "vue";
import { bridgeBinaryBytes } from "../../../../../shared/frontend/binaryEvent";
import { randomUUID } from "../../../shared/lib/uuid";
import { createTerminalInputQueue } from "../lib/terminalInputQueue";
import {
  buildSplitSessionOpenParams,
  createSplitFrameSequencer,
  parseSplitOutputFrame,
  SPLIT_DEFAULT_COLS,
  SPLIT_DEFAULT_ROWS,
  SPLIT_OUTPUT_CHANNEL_PREFIX,
  splitSessionInputChannel,
  type SplitFrameSequencer,
} from "../lib/terminalSplit";

export type SplitTerminalStatus = "idle" | "connecting" | "connected" | "error" | "closed";

const OPEN_TIMEOUT_MS = 120_000;
const REPLAY_TIMEOUT_MS = 20_000;
const INPUT_SEND_TIMEOUT_MS = 8_000;

interface SplitSessionInfo {
  sessionId?: string;
}

interface SplitReplayResult {
  complete?: boolean;
  firstAvailableSequence?: number;
  tailSequence?: number;
}

export interface UseSplitTerminalOptions {
  connectionId: () => string;
  workbenchId: () => string;
  mainSessionId: () => string;
  t: (key: string, values?: Record<string, string | number>) => string;
  /** Raw terminal output bytes; the component writes them to xterm. */
  onData: (data: Uint8Array) => void;
  onState?: (state: string) => void;
  onOpened?: (sessionId: string) => void;
  onError?: (message: string) => void;
  /** Test seam; production uses the lib/uuid shim. */
  createSessionId?: () => string;
}

export interface UseSplitTerminal {
  status: Ref<SplitTerminalStatus>;
  sessionId: Ref<string>;
  errorMessage: Ref<string>;
  open(cols?: number, rows?: number): Promise<void>;
  close(): void;
  sendInput(data: string): void;
  resize(cols: number, rows: number): void;
}

export function useSplitTerminal(options: UseSplitTerminalOptions): UseSplitTerminal {
  const status = ref<SplitTerminalStatus>("idle");
  const sessionId = ref("");
  const errorMessage = ref("");

  let requestedSessionId = "";
  let generation = 0;
  let sequencer: SplitFrameSequencer | undefined;
  let unsubscribeBinary: (() => void) | undefined;
  let replayInFlight = false;
  let stopped = true;
  let errorReported = false;
  // 最近一次已发送（或随 open 下发）的 PTY 尺寸：ResizeObserver 的初始回调
  // 会在会话打开后补发同尺寸 resize，远端 shell 收到 window-change 后重绘
  // 提示符，表现为一行两个 `[user@host ~]#`。同尺寸一律不发。
  let lastResizeCols = 0;
  let lastResizeRows = 0;

  const encoder = new TextEncoder();

  const inputQueue = createTerminalInputQueue({
    send: (targetSessionId, payload) => window.dbxPlugin.sendBinary(splitSessionInputChannel(targetSessionId), payload),
    sendTimeoutMs: INPUT_SEND_TIMEOUT_MS,
    onError: (cause) => {
      inputQueue.reset();
      if (!stopped) fail(cause);
    },
  });

  function errorText(cause: unknown): string {
    if (cause instanceof Error && cause.message.trim()) return cause.message.trim();
    if (typeof cause === "string" && cause.trim()) return cause.trim();
    const text = String(cause ?? "").trim();
    return text || options.t("error");
  }

  function fail(cause: unknown) {
    if (errorReported) return;
    errorReported = true;
    stopped = true;
    errorMessage.value = errorText(cause);
    status.value = "error";
    options.onError?.(errorMessage.value);
  }

  function newRequestedSessionId(): string {
    if (options.createSessionId) return options.createSessionId();
    // lib/uuid shim: the global UUID helper is absent on plain-HTTP LAN hosts.
    return randomUUID();
  }

  function closeRemote(targetSessionId: string) {
    const api = window.dbxPlugin;
    if (!api || !targetSessionId) return;
    void api.invoke("ssh/session/close", { sessionId: targetSessionId }).catch(() => undefined);
  }

  function stopListening() {
    unsubscribeBinary?.();
    unsubscribeBinary = undefined;
  }

  function createSequencer(): SplitFrameSequencer {
    return createSplitFrameSequencer(
      (data) => {
        if (!stopped) options.onData(data);
      },
      (state) => {
        options.onState?.(state);
        // 分屏不参与目录跟随；主会话把该状态当作信息提示，分屏不能因此
        // 停止输出（原生 shell 常无法注入提示钩子，这是正常降级）。
        if (state === "directory-tracking-unavailable") return;
        fail(state === "ssh-transport-disconnected" ? options.t("transportDisconnected") : state || options.t("disconnected"));
      },
      {
        onGap: (afterSequence) => {
          if (!stopped) void requestGapReplay(afterSequence);
        },
      },
    );
  }

  function handleBinary(event: DbxPluginBinaryEvent) {
    if (stopped) return;
    if (!event.channel.startsWith(SPLIT_OUTPUT_CHANNEL_PREFIX)) return;
    const frameSessionId = event.channel.slice(SPLIT_OUTPUT_CHANNEL_PREFIX.length);
    // Accept the speculative requested id until open resolves, then only the
    // authoritative session id: older sidecars may ignore requestedSessionId.
    if (sessionId.value) {
      if (frameSessionId !== sessionId.value) return;
    } else if (frameSessionId !== requestedSessionId) {
      return;
    }
    const api = window.dbxPlugin;
    if (!api) return;
    let payload: Uint8Array;
    try {
      payload = bridgeBinaryBytes(event, api.decodeBase64);
    } catch {
      return;
    }
    const frame = parseSplitOutputFrame(payload);
    if (!frame) return;
    sequencer?.push(frame);
  }

  async function replaySession(targetSessionId: string, afterSequence: number): Promise<SplitReplayResult> {
    const api = window.dbxPlugin;
    if (!api) throw new Error(options.t("hostApiUnavailable"));
    replayInFlight = true;
    try {
      return await api.invoke<SplitReplayResult>(
        "ssh/terminal/replay",
        { sessionId: targetSessionId, afterSequence },
        { timeoutMs: REPLAY_TIMEOUT_MS },
      );
    } finally {
      replayInFlight = false;
    }
  }

  /** A gap detected while frames are live: repair from the last contiguous
   * sequence once, without turning a permanently missing frame into a storm. */
  async function requestGapReplay(afterSequence: number) {
    const targetSessionId = sessionId.value;
    if (!targetSessionId || replayInFlight || stopped) return;
    replayInFlight = true;
    try {
      await window.dbxPlugin.invoke(
        "ssh/terminal/replay",
        { sessionId: targetSessionId, afterSequence },
        { timeoutMs: REPLAY_TIMEOUT_MS },
      );
    } catch (cause) {
      if (!stopped && sessionId.value === targetSessionId) fail(cause);
    } finally {
      replayInFlight = false;
    }
  }

  async function open(cols: number = SPLIT_DEFAULT_COLS, rows: number = SPLIT_DEFAULT_ROWS) {
    generation += 1;
    const currentGeneration = generation;
    const api = window.dbxPlugin;
    const spawnCols = Math.max(1, Math.floor(cols));
    const spawnRows = Math.max(1, Math.floor(rows));
    lastResizeCols = spawnCols;
    lastResizeRows = spawnRows;

    stopListening();
    inputQueue.reset();
    sequencer?.reset();
    requestedSessionId = newRequestedSessionId();
    sessionId.value = "";
    errorMessage.value = "";
    errorReported = false;
    replayInFlight = false;
    stopped = false;
    status.value = "connecting";
    sequencer = createSequencer();


    // Subscribe before the long-running open RPC returns so output emitted
    // under requestedSessionId is not lost.
    if (api) unsubscribeBinary = api.onBinary(handleBinary);

    try {
      if (!api) throw new Error(options.t("hostApiUnavailable"));
      const info = await api.invoke<SplitSessionInfo>(
        "ssh/session/open",
        buildSplitSessionOpenParams({
          connectionId: options.connectionId(),
          workbenchId: options.workbenchId(),
          mainSessionId: options.mainSessionId(),
          requestedSessionId,
          cols: spawnCols,
          rows: spawnRows,
        }),
        { timeoutMs: OPEN_TIMEOUT_MS },
      );

      // The pane was closed/reopened while open was in flight: the returned
      // PTY is an orphan and must not leak on the shared transport.
      if (currentGeneration !== generation) {
        if (info?.sessionId) closeRemote(info.sessionId);
        return;
      }
      if (!info?.sessionId) throw new Error(options.t("error"));

      if (info.sessionId !== requestedSessionId) {
        // The returned id is authoritative. Frames captured under the
        // speculative id are discarded; replay from 0 recovers the stream.
        sequencer.reset();
      }
      sessionId.value = info.sessionId;

      if (stopped) return;

      const replay = await replaySession(info.sessionId, 0);
      if (currentGeneration !== generation) return;
      if (stopped) return;
      if (replay?.complete === false) throw new Error(options.t("sessionUnrecoverable"));

      status.value = "connected";
      options.onOpened?.(info.sessionId);
    } catch (cause) {
      if (currentGeneration !== generation) return;
      fail(cause);
    }
  }

  function close() {
    generation += 1;
    stopped = true;
    lastResizeCols = 0;
    lastResizeRows = 0;
    const targetSessionId = sessionId.value;
    sessionId.value = "";
    requestedSessionId = "";
    sequencer?.reset();
    sequencer = undefined;
    replayInFlight = false;
    inputQueue.reset();
    stopListening();
    status.value = "closed";
    if (targetSessionId) closeRemote(targetSessionId);
  }

  function sendInput(data: string) {
    if (stopped || status.value !== "connected") return;
    const targetSessionId = sessionId.value;
    if (!targetSessionId || data.length === 0) return;
    // Keep bytes as xterm produced them; the input queue adds the 8-byte
    // sequence prefix expected by ssh/terminal/in/{id}.
    inputQueue.enqueue(targetSessionId, encoder.encode(data));
  }

  function resize(cols: number, rows: number) {
    if (stopped) return;
    const targetSessionId = sessionId.value;
    const api = window.dbxPlugin;
    if (!targetSessionId || !api) return;
    if (!Number.isFinite(cols) || !Number.isFinite(rows)) return;
    const normalizedCols = Math.max(1, Math.floor(cols));
    const normalizedRows = Math.max(1, Math.floor(rows));
    if (normalizedCols === lastResizeCols && normalizedRows === lastResizeRows) return;
    lastResizeCols = normalizedCols;
    lastResizeRows = normalizedRows;
    void api
      .notify("ssh/terminal/resize", { sessionId: targetSessionId, cols: normalizedCols, rows: normalizedRows })
      .catch(() => undefined);
  }

  return { status, sessionId, errorMessage, open, close, sendInput, resize };
}
