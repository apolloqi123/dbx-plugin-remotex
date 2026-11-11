// @vitest-environment happy-dom
// SSH 帧洪峰清空缓冲后的缺口回放回归：清空 pending map 会让 firstPending 变
// Infinity、洞检测永不触发，若不主动补拉，游标会永久停在洞前（画面冻结，且
// 断连状态帧可能永远送不到）。local/telnet/serial 三路已经有/补齐同构修复。
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useTerminalWriter, type TerminalFrameStream, type TerminalWriterOptions } from "./useTerminalWriter";

const globalWindow = window as unknown as { dbxPlugin?: unknown };

const REPLAY_RESULT = { frameCount: 0, firstAvailableSequence: 1, tailSequence: 0, complete: true };

interface Harness {
  writer: ReturnType<typeof useTerminalWriter>;
  frameStream: TerminalFrameStream;
  invoke: ReturnType<typeof vi.fn>;
  scheduleSessionReconnect: ReturnType<typeof vi.fn>;
  showError: ReturnType<typeof vi.fn>;
}

function mountWriter(invokeImpl?: (method: string, params: Record<string, unknown>) => Promise<unknown>): Harness {
  const invoke = vi.fn(invokeImpl ?? (async () => REPLAY_RESULT));
  globalWindow.dbxPlugin = { invoke };

  const frameStream: TerminalFrameStream = {
    pendingFrames: new Map(),
    lastSequence: 0,
    replayNoProgress: 0,
    activeSessionId: "s1",
  };
  const scheduleSessionReconnect = vi.fn();
  const showError = vi.fn();

  // 工厂初始化只创建 gate/colorizer/throttle（不调用这些 getter），drain 路径
  // 只用到下面这一小撮字段；其余字段以空对象占位并做类型双转，避免为 46 个
  // 必填字段逐一伪造而掩盖被测行为。
  const options = {
    t: (key: string) => key,
    showNotice: vi.fn(),
    showError,
    terminal: () => undefined,
    isTransferBusy: () => false,
    frameStream,
    session: ref({ sessionId: "s1" }),
    terminalState: ref("connected"),
    terminalError: ref(""),
    terminalCwd: ref(""),
    followDirectory: ref(false),
    directoryTrackingSupported: ref(false),
    persistState: vi.fn(),
    consumeFrameViaZmodem: vi.fn(),
    terminalTransportWatch: () => ({ reportUnavailable: vi.fn() }),
    isReconnectPending: () => false,
    scheduleSessionReconnect,
    isDisposed: () => false,
  } as unknown as TerminalWriterOptions;

  return { writer: useTerminalWriter(options), frameStream, invoke, scheduleSessionReconnect, showError };
}

function addFrames(stream: TerminalFrameStream, from: number, to: number) {
  for (let sequence = from; sequence <= to; sequence += 1) {
    stream.pendingFrames.set(sequence, { stream: 0, data: new Uint8Array() });
  }
}

afterEach(() => {
  delete globalWindow.dbxPlugin;
  vi.restoreAllMocks();
});

describe("useTerminalWriter flood-clear replay (SSH main channel)", () => {
  it("proactively replays after a flood clear instead of freezing at the hole", async () => {
    const { writer, frameStream, invoke } = mountWriter();
    // 帧 1 丢失；后续 1029 帧 (>1024) 触发洪峰清空。while 循环消费不到任何帧
    // （lastSequence+1 === 1 不在 map），若清空后不主动补拉则 replay 永不发起。
    addFrames(frameStream, 2, 1030);

    writer.drainTerminalFrames();

    expect(frameStream.pendingFrames.size).toBe(0);
    expect(invoke).toHaveBeenCalledWith("ssh/terminal/replay", { sessionId: "s1", afterSequence: 0 });
    // 让 replay 的 .then/.finally 跑完（避免未处理的 promise 影响后续用例）。
    await Promise.resolve();
    await Promise.resolve();
  });

  it("captures a finite resync target before clearing so a session-gone replay can resync", async () => {
    const { writer, frameStream, invoke, scheduleSessionReconnect, showError } = mountWriter(
      async () => { throw new Error("SSH session was not found or expired"); },
    );
    addFrames(frameStream, 2, 1030);

    writer.drainTerminalFrames();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    // 洪峰前的第一帧是 2，resync 目标必须是 1，而不是清空后算出的 Infinity - 1。
    expect(frameStream.lastSequence).toBe(1);
    expect(Number.isFinite(frameStream.lastSequence)).toBe(true);
    expect(showError).not.toHaveBeenCalled();
    expect(scheduleSessionReconnect).toHaveBeenCalledOnce();
  });

  it("still replays a normal (below-limit) gap", async () => {
    const { writer, frameStream, invoke } = mountWriter();
    addFrames(frameStream, 2, 5);

    writer.drainTerminalFrames();

    expect(frameStream.pendingFrames.size).toBe(4);
    expect(invoke).toHaveBeenCalledWith("ssh/terminal/replay", { sessionId: "s1", afterSequence: 0 });
    await Promise.resolve();
    await Promise.resolve();
  });
});
