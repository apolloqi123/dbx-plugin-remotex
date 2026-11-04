// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, ref, type Ref } from "vue";
import { useDesktopClipboard, type DesktopClipboardSync } from "./useDesktopClipboard";

/** useDesktopClipboard 隔离单测：effectScope 提供组件作用域（stop() 随
 *  scope.stop() 清理定时器/监听器），宿主剪贴板与远端 RPC 全部走桩，
 * 时间用 vitest 假定时器（含 Date）推进轮询与退避。 */

const cleanups: Array<() => void> = [];

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function setVisibility(value: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { value, configurable: true });
}

interface Harness {
  api: DesktopClipboardSync;
  sessionId: Ref<string | null>;
  bridge: {
    readText: ReturnType<typeof vi.fn>;
    writeText: ReturnType<typeof vi.fn>;
  };
  setRemoteText: ReturnType<typeof vi.fn>;
  sendPasteShortcut: ReturnType<typeof vi.fn>;
  showError: ReturnType<typeof vi.fn>;
  showNotice: ReturnType<typeof vi.fn>;
  setHost(text: string): void;
  getHost(): string;
  setActive(value: boolean): void;
}

function createHarness(options: { useDefaultDeps?: boolean } = {}): Harness {
  let hostText = "";
  let active = true;
  const sessionId = ref<string | null>("session-1");
  const bridge = {
    readText: vi.fn(async () => hostText),
    writeText: vi.fn(async (text: string) => {
      hostText = text;
    }),
  };
  const setRemoteText = vi.fn(async (_sessionId: string, _text: string) => undefined);
  const sendPasteShortcut = vi.fn();
  const showError = vi.fn();
  const showNotice = vi.fn();

  if (options.useDefaultDeps) {
    (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = { clipboard: bridge };
    cleanups.push(() => {
      delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
    });
  }

  const scope = effectScope();
  const api = scope.run(() =>
    useDesktopClipboard({
      t: (key: string) => key,
      showError,
      showNotice,
      sessionId,
      active: () => active,
      setRemoteText,
      sendPasteShortcut,
      ...(options.useDefaultDeps
        ? {}
        : { deps: () => ({ bridge, nativeClipboard: null, execCommand: () => false }) }),
    }),
  );
  if (!api) throw new Error("useDesktopClipboard did not initialize");
  cleanups.push(() => scope.stop());

  return {
    api,
    sessionId,
    bridge,
    setRemoteText,
    sendPasteShortcut,
    showError,
    showNotice,
    setHost: (text: string) => {
      hostText = text;
    },
    getHost: () => hostText,
    setActive: (value: boolean) => {
      active = value;
    },
  };
}

describe("useDesktopClipboard host -> remote", () => {
  it("pushes the current host clipboard on the first sync and suppresses the echo", async () => {
    const harness = createHarness();
    harness.setHost("pre-connect text");
    harness.api.reset();

    await harness.api.syncNow();

    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "pre-connect text");

    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledTimes(1);
  });

  it("reads through the default host bridge when no deps are injected", async () => {
    const harness = createHarness({ useDefaultDeps: true });
    harness.setHost("bridge text");
    harness.api.reset();

    await harness.api.syncNow();

    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "bridge text");
  });

  it("pushes a later host change once and suppresses the echo", async () => {
    const harness = createHarness();
    harness.setHost("initial");
    harness.api.reset();
    await harness.api.syncNow();

    harness.setHost("first change");
    await harness.api.syncNow();
    await harness.api.syncNow();

    expect(harness.setRemoteText).toHaveBeenCalledTimes(2);
    expect(harness.setRemoteText).toHaveBeenLastCalledWith("session-1", "first change");
  });

  it("polls while the document is visible and the session is active, without a canvas focus gate", async () => {
    const harness = createHarness();
    harness.api.reset();
    harness.api.start();

    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);

    // A host-level focus change must not stop background syncing: the iframe
    // document is unfocused whenever the user copies from another macOS app.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(2);

    setVisibility("hidden");
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(2);

    setVisibility("visible");
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(3);

    harness.setActive(false);
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(3);

    harness.setActive(true);
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(4);

    harness.sessionId.value = null;
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(4);

    harness.sessionId.value = "session-1";
    harness.api.syncEnabled.value = false;
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(4);
  });

  it("stops polling when the remote clipboard channel is unavailable", async () => {
    const harness = createHarness();
    harness.api.reset();
    harness.api.start();

    harness.api.setChannelAvailable(false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(harness.bridge.readText).not.toHaveBeenCalled();

    harness.api.setChannelAvailable(true);
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
  });

  it("does not advance baselines when the remote staging RPC fails, so the next poll retries", async () => {
    const harness = createHarness();
    harness.setHost("host text");
    harness.api.reset();
    harness.setRemoteText.mockRejectedValueOnce(new Error("rpc down"));

    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledTimes(1);
    expect(harness.showError).toHaveBeenCalledTimes(1);

    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledTimes(2);
    expect(harness.setRemoteText).toHaveBeenLastCalledWith("session-1", "host text");
  });

  it("keeps a manual remote clipboard until the host clipboard actually changes", async () => {
    const harness = createHarness();
    harness.setHost("host-old");
    harness.api.reset();
    await harness.api.syncNow();
    harness.setRemoteText.mockClear();

    await harness.api.setRemoteClipboard("toolbar text");
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "toolbar text");

    harness.setRemoteText.mockClear();
    await harness.api.syncNow();
    expect(harness.setRemoteText).not.toHaveBeenCalled();

    harness.setHost("host-new");
    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "host-new");
  });

  it("syncs on window focus without requiring the canvas to hold focus", async () => {
    const harness = createHarness();
    harness.api.reset();
    await harness.api.syncNow();
    harness.setHost("focused change");
    harness.api.start();

    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(0);

    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "focused change");
  });
});

describe("useDesktopClipboard remote -> host", () => {
  it("writes remote text to the host and suppresses the poller echo", async () => {
    const harness = createHarness();
    harness.setHost("local text");
    harness.api.reset();
    await harness.api.syncNow();

    await harness.api.receiveRemoteText("remote text");

    expect(harness.bridge.writeText).toHaveBeenCalledWith("remote text");
    expect(harness.getHost()).toBe("remote text");
    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardReceived");

    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledTimes(1);
  });

  it("ignores duplicate and empty remote text", async () => {
    const harness = createHarness();

    await harness.api.receiveRemoteText("");
    await harness.api.receiveRemoteText("remote text");
    await harness.api.receiveRemoteText("remote text");

    expect(harness.bridge.writeText).toHaveBeenCalledTimes(1);
  });

  it("surfaces a host write failure and retries it on the next sync", async () => {
    const harness = createHarness();
    harness.api.reset();
    await harness.api.syncNow();
    harness.bridge.writeText.mockRejectedValueOnce(new Error("write denied"));

    await harness.api.receiveRemoteText("remote text");

    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.showError.mock.calls[0][0]).toBeInstanceOf(Error);
    expect(harness.api.lastError.value).toBe("clipboard write unavailable");
    expect(harness.getHost()).toBe("");

    await harness.api.syncNow();

    expect(harness.bridge.writeText).toHaveBeenCalledTimes(2);
    expect(harness.getHost()).toBe("remote text");
  });
});

describe("useDesktopClipboard failure handling", () => {
  it("pauses after three consecutive read failures and resumes after the retry delay", async () => {
    const harness = createHarness();
    harness.bridge.readText.mockRejectedValue(new Error("read failed"));
    harness.api.reset();
    harness.api.start();

    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(3);
    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.api.lastError.value).toBe("read failed");

    // Paused for retryDelayMs: the next tick must not read.
    await vi.advanceTimersByTimeAsync(1500);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(3);

    await vi.advanceTimersByTimeAsync(15000);
    expect(harness.bridge.readText.mock.calls.length).toBeGreaterThan(3);
  });

  it("does not count rate-limit read errors toward the failure backoff", async () => {
    const harness = createHarness();
    harness.bridge.readText.mockRejectedValue(new Error("rate limit exceeded"));
    harness.api.reset();
    harness.api.start();

    for (let i = 0; i < 6; i += 1) {
      await vi.advanceTimersByTimeAsync(1500);
    }

    expect(harness.bridge.readText).toHaveBeenCalledTimes(6);
    expect(harness.showError).not.toHaveBeenCalled();
  });

  it("shows the actionable notice when the host denies clipboard access", async () => {
    const harness = createHarness();
    harness.bridge.readText.mockRejectedValue(
      new Error("Clipboard read was denied for this plugin session"),
    );
    harness.api.reset();
    harness.api.start();

    await vi.advanceTimersByTimeAsync(1500);

    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardAccessDenied");
    expect(harness.showError).not.toHaveBeenCalled();

    // Denied consent cannot change during a bridge lifetime: back off for a
    // minute instead of retrying every tick.
    await vi.advanceTimersByTimeAsync(10000);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
  });
});

describe("useDesktopClipboard paste", () => {
  it("syncs native paste text and sends the shortcut exactly once", async () => {
    const harness = createHarness();

    const pending = harness.api.pasteToRemote("native text");
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    expect(harness.bridge.readText).not.toHaveBeenCalled();
    expect(harness.setRemoteText).toHaveBeenCalledTimes(1);
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "native text");
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("syncs bridge text and sends the shortcut exactly once", async () => {
    const harness = createHarness();
    harness.setHost("bridge text");
    harness.api.reset();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "bridge text");
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("reuses a fresh host read instead of burning the host read quota", async () => {
    const harness = createHarness();
    harness.setHost("fresh text");
    harness.api.reset();
    await harness.api.syncNow();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("does not let a manual remote write masquerade as host clipboard text", async () => {
    const harness = createHarness();
    harness.setHost("host text");
    harness.api.reset();
    await harness.api.syncNow();
    await harness.api.setRemoteClipboard("toolbar text");
    harness.setRemoteText.mockClear();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    // Ctrl+V always pastes the host clipboard, never the previous manual text.
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "host text");
  });

  it("reuses the latest host read for paste even when its push failed", async () => {
    const harness = createHarness();
    harness.setHost("fresh text");
    harness.api.reset();
    harness.setRemoteText.mockRejectedValueOnce(new Error("rpc down"));
    await harness.api.syncNow();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    const pasted = await pending;

    expect(pasted).toBe(true);
    expect(harness.bridge.readText).toHaveBeenCalledTimes(1);
    expect(harness.setRemoteText).toHaveBeenLastCalledWith("session-1", "fresh text");
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("sends the shortcut once even when the host clipboard is empty", async () => {
    const harness = createHarness();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    expect(harness.setRemoteText).not.toHaveBeenCalled();
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("retries once when an explicit paste hits the host read rate limit", async () => {
    const harness = createHarness();
    harness.bridge.readText
      .mockRejectedValueOnce(new Error("Clipboard read rate limit exceeded; retry in a moment"))
      .mockResolvedValueOnce("rate-limited then pasted");
    harness.api.reset();

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(1100);
    await vi.advanceTimersByTimeAsync(80);
    await pending;

    expect(harness.bridge.readText).toHaveBeenCalledTimes(2);
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "rate-limited then pasted");
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
    expect(harness.showError).not.toHaveBeenCalled();
  });

  it("still forwards the paste when the host read fails (the key is never swallowed)", async () => {
    const harness = createHarness();
    harness.bridge.readText.mockRejectedValue(new Error("read denied"));

    const pasted = await harness.api.pasteToRemote();

    expect(pasted).toBe(true);
    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardAccessDenied");
    expect(harness.setRemoteText).not.toHaveBeenCalled();
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);
  });

  it("still forwards the paste when the remote sync fails", async () => {
    const harness = createHarness();
    harness.setHost("bridge text");
    harness.setRemoteText.mockRejectedValueOnce(new Error("rpc down"));

    const pending = harness.api.pasteToRemote();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(80);
    const pasted = await pending;

    expect(pasted).toBe(true);
    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.sendPasteShortcut).toHaveBeenCalledTimes(1);

    // The failed stage did not advance the baselines, so the next poll retries.
    harness.setRemoteText.mockClear();
    await harness.api.syncNow();
    expect(harness.setRemoteText).toHaveBeenCalledWith("session-1", "bridge text");
  });
});
