// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { initialSerialUploadState, type SerialUploadUiState } from "../../../features/serial/lib/serialUpload";
import { registerDockerLogStream } from "../../../features/docker/lib/dockerLogEvents";
import { enqueueUploadAck, type UploadAckWaiters } from "../../../features/transfers/lib/uploadAck";
import { usePluginEvents } from "./usePluginEvents";

/** issue #193a 回归锁：会话生命周期事件按 sessionId 严格隔离。关闭/断开某个
 * tab（或 dock 终端）时，sidecar 对该会话的 disconnected 事件绝不能让持有其他
 * 会话的兄弟工作台进入重连梯子；只有事件 sessionId 与本工作台当前会话一致才
 * 触发 scheduleSessionReconnect。 */

function serialUploadState(): SerialUploadUiState {
  return initialSerialUploadState();
}

type HarnessOptions = Partial<Parameters<typeof usePluginEvents>[0]>;

function buildHarness(overrides: HarnessOptions = {}) {
  const scheduleSessionReconnect = vi.fn();
  const options: Parameters<typeof usePluginEvents>[0] = {
    t: (key: string) => key,
    session: ref<{ sessionId?: string } | undefined>({ sessionId: "session-A" }),
    terminalState: ref("connected"),
    showError: vi.fn(),
    showNotice: vi.fn(),
    terminalDiag: { keys: 0, sends: 0, acks: 0, errors: 0, swallowed: 0 },
    batchBarSourceId: "source-1",
    applyRemoteBatchBarState: vi.fn(),
    hostKeyPrompt: ref(null),
    rdpCertPrompt: ref(null),
    rdpCertRemember: ref(false),
    connectLog: { entries: ref([]), push: vi.fn(), clear: vi.fn() },
    scheduleSessionReconnect,
    reconnectPending: ref(false),
    localSession: ref<{ sessionId: string; shell: string } | null>(null),
    markLocalExited: vi.fn(),
    telnetSession: ref<{ sessionId: string; host: string; port: number } | null>(null),
    telnetState: ref(""),
    telnetError: ref(""),
    markTelnetClosed: vi.fn(),
    vncSession: ref<{ sessionId: string; host: string; port: number } | null>(null),
    vncState: ref(""),
    vncError: ref(""),
    vncSurface: ref(null),
    markVncClosed: vi.fn(),
    serialSession: ref<{ sessionId: string; port: string; baudRate: number } | null>(null),
    markSerialClosed: vi.fn(),
    serialUpload: ref(serialUploadState()),
    rdpSession: ref<{ sessionId: string; host: string; port: number } | null>(null),
    rdpState: ref({ state: "idle", error: "", errorKind: "", attempt: 0, maxAttempts: 0 }),
    rdpSurface: ref(null),
    rdpClipboardChunks: new Map(),
    agentPromptQueue: ref([]),
    agentRunning: ref(undefined),
    recordingActive: ref(false),
    startRecordingClock: vi.fn(),
    handleWatchModified: vi.fn(),
    uploadAckWaiters: new Map(),
    updateTransfer: vi.fn(),
    ...overrides,
  };
  return { handlePluginEvent: usePluginEvents(options).handlePluginEvent, scheduleSessionReconnect };
}

function backendEvent(method: string, params: Record<string, unknown>) {
  return { method, params } as unknown as Parameters<ReturnType<typeof usePluginEvents>["handlePluginEvent"]>[0];
}

describe("usePluginEvents session lifecycle isolation (issue #193a)", () => {
  it("ignores a sibling session's disconnected event and schedules reconnect only for its own session", () => {
    const { handlePluginEvent, scheduleSessionReconnect } = buildHarness();

    // 用户关掉另一个 tab：sidecar 对「那个」会话发 disconnected——本工作台不得反应。
    handlePluginEvent(backendEvent("ssh/session/state", { sessionId: "session-B", state: "disconnected" }));
    expect(scheduleSessionReconnect).not.toHaveBeenCalled();

    // 自己的传输断开才进重连梯子。
    handlePluginEvent(backendEvent("ssh/session/state", { sessionId: "session-A", state: "disconnected" }));
    expect(scheduleSessionReconnect).toHaveBeenCalledTimes(1);
  });

  it("does not reconnect when the workbench has no session bound", () => {
    const { handlePluginEvent, scheduleSessionReconnect } = buildHarness({
      session: ref<{ sessionId?: string } | undefined>(undefined),
    });
    handlePluginEvent(backendEvent("ssh/session/state", { sessionId: "session-A", state: "disconnected" }));
    expect(scheduleSessionReconnect).not.toHaveBeenCalled();
  });

  it("keeps ssh/terminal/error scoped to the owning session as well", () => {
    const { handlePluginEvent, scheduleSessionReconnect } = buildHarness();
    handlePluginEvent(backendEvent("ssh/terminal/error", { sessionId: "session-B", error: "gone" }));
    expect(scheduleSessionReconnect).not.toHaveBeenCalled();
    handlePluginEvent(backendEvent("ssh/terminal/error", { sessionId: "session-A", error: "gone" }));
    expect(scheduleSessionReconnect).toHaveBeenCalledTimes(1);
  });

  it("routes local shell exit events only for the local session this workbench owns", () => {
    const markLocalExited = vi.fn();
    const localSession = ref<{ sessionId: string; shell: string } | null>({ sessionId: "local-1", shell: "/bin/zsh" });
    const { handlePluginEvent } = buildHarness({ localSession, markLocalExited });
    handlePluginEvent(backendEvent("local/session/state", { sessionId: "local-2", state: "exited", exitCode: 0 }));
    expect(markLocalExited).not.toHaveBeenCalled();
    handlePluginEvent(backendEvent("local/session/state", { sessionId: "local-1", state: "exited", exitCode: 0 }));
    expect(markLocalExited).toHaveBeenCalledTimes(1);
  });
});

describe("usePluginEvents docker log routing", () => {
  it("hands chunk/end events to the stream handler table without other branches", () => {
    const handler = vi.fn();
    const dispose = registerDockerLogStream("stream-1", handler);
    try {
      const { handlePluginEvent } = buildHarness();
      handlePluginEvent(backendEvent("docker/logs/chunk", { streamId: "stream-1", text: "hello" }));
      expect(handler).toHaveBeenCalledWith({
        method: "docker/logs/chunk",
        params: { streamId: "stream-1", text: "hello" },
      });
      handlePluginEvent(backendEvent("docker/logs/end", { streamId: "stream-1", exitCode: 0 }));
      expect(handler).toHaveBeenLastCalledWith({
        method: "docker/logs/end",
        params: { streamId: "stream-1", exitCode: 0 },
      });
      // 快照类 method 不走这条路由（语义保留）。
      handlePluginEvent(backendEvent("docker/logs", { streamId: "stream-1" }));
      expect(handler).toHaveBeenCalledTimes(2);
    } finally {
      dispose();
    }
  });
});

describe("usePluginEvents upload ack routing", () => {
  it("releases cumulative acks and rejects every waiter on upload/error", () => {
    const waiters: UploadAckWaiters = new Map();
    const { handlePluginEvent } = buildHarness({ uploadAckWaiters: waiters });
    const firstResolve = vi.fn();
    const firstReject = vi.fn();
    const secondReject = vi.fn();
    enqueueUploadAck(waiters, "t1", {
      nextOffset: 1,
      resolve: firstResolve,
      reject: firstReject,
      timer: window.setTimeout(() => undefined, 10_000),
    });

    // ack 携带累计 offset：2 同时证明 1 已落盘。
    handlePluginEvent(backendEvent("sftp/upload/ack", { taskId: "t1", nextOffset: 2 }));
    expect(firstResolve).toHaveBeenCalledTimes(1);
    expect(waiters.size).toBe(0);

    enqueueUploadAck(waiters, "t1", {
      nextOffset: 5,
      resolve: vi.fn(),
      reject: secondReject,
      timer: window.setTimeout(() => undefined, 10_000),
    });
    handlePluginEvent(backendEvent("sftp/upload/error", { taskId: "t1", error: "rejected" }));
    expect(secondReject).toHaveBeenCalledWith(expect.objectContaining({ code: "upload-append-failed" }));
    expect(waiters.size).toBe(0);
  });
});

describe("usePluginEvents remote clipboard routing", () => {
  const rdpSession = () =>
    ref<{ sessionId: string; host: string; port: number } | null>({
      sessionId: "rdp-1",
      host: "10.0.0.8",
      port: 3389,
    });

  it("reassembles RDP clipboard chunks and forwards them to the session", () => {
    const onRdpClipboard = vi.fn();
    const { handlePluginEvent } = buildHarness({ rdpSession: rdpSession(), onRdpClipboard });
    handlePluginEvent(
      backendEvent("rdp/clipboard", { sessionId: "rdp-1", text: "ab", chunkIndex: 0, chunkTotal: 2 }),
    );
    expect(onRdpClipboard).not.toHaveBeenCalled();
    handlePluginEvent(
      backendEvent("rdp/clipboard", { sessionId: "rdp-1", text: "cd", chunkIndex: 1, chunkTotal: 2 }),
    );
    expect(onRdpClipboard).toHaveBeenCalledWith("abcd");
  });

  it("forwards RDP file clipboard events with the method name", () => {
    const onRdpClipboardFiles = vi.fn();
    const { handlePluginEvent } = buildHarness({ rdpSession: rdpSession(), onRdpClipboardFiles });
    handlePluginEvent(
      backendEvent("rdp/clipboard-files/ready", {
        sessionId: "rdp-1",
        archiveName: "clip.zip",
        archiveSize: 42,
        fileCount: 2,
      }),
    );
    expect(onRdpClipboardFiles).toHaveBeenCalledWith(
      "rdp/clipboard-files/ready",
      expect.objectContaining({ sessionId: "rdp-1", archiveName: "clip.zip" }),
    );
  });
});

describe("usePluginEvents VNC clipboard sharing", () => {
  it("forwards a remote clipboard update to the VNC session handler", () => {
    const onVncClipboard = vi.fn();
    const vncSession = ref<{ sessionId: string; host: string; port: number } | null>({
      sessionId: "vnc-1",
      host: "10.0.0.7",
      port: 5900,
    });
    const { handlePluginEvent } = buildHarness({ vncSession, vncState: ref("running"), onVncClipboard });

    handlePluginEvent(backendEvent("vnc/clipboard", { sessionId: "vnc-1", text: "远程剪贴板" }));
    expect(onVncClipboard).toHaveBeenCalledWith("远程剪贴板");
    handlePluginEvent(backendEvent("vnc/clipboard", { sessionId: "vnc-other", text: "other" }));
    expect(onVncClipboard).toHaveBeenCalledTimes(1);
  });
});
