import { getCurrentScope, onScopeDispose, ref, type Ref } from "vue";
import {
  clipboardReadAvailable,
  readClipboardText,
  writeClipboardText,
  type ClipboardDeps,
} from "../../../shared/lib/clipboardBridge";

export interface DesktopClipboardOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  sessionId: Ref<string | null>;
  /** Session phase gate: true only while the remote desktop is running and not disposed. */
  active: () => boolean;
  /** Push host text to the remote clipboard (rdp/set-clipboard or vnc/set-clipboard). */
  setRemoteText: (sessionId: string, text: string) => Promise<void>;
  /** Send Ctrl+V to the remote (implemented by the surface). Called exactly once after a successful sync. */
  sendPasteShortcut: () => void;
  /** i18n key for the remote→host notice; defaults to the RDP wording. */
  receivedNoticeKey?: string;
  /** i18n key shown once when the host denies clipboard reads. */
  accessDeniedNoticeKey?: string;
  deps?: () => ClipboardDeps;
  pollIntervalMs?: number;
  retryDelayMs?: number;
  denyRetryDelayMs?: number;
  now?: () => number;
}

export interface DesktopClipboardSync {
  /** Call when a session starts/reattaches: drops baselines and channel state. */
  reset(): void;
  /** Poll/read the host clipboard now (also used on focus/visibility). */
  syncNow(): Promise<void>;
  /** Remote clipboard -> host; call for rdp/clipboard and vnc/clipboard events. */
  receiveRemoteText(text: string): Promise<void>;
  /** Set the remote clipboard only (the user pastes inside the remote later). */
  setRemoteClipboard(text: string): Promise<boolean>;
  /** Ctrl/Cmd+V: sync host text (or native paste text) then send the remote paste shortcut once.
   *  Resolves true only when the shortcut was actually sent. */
  pasteToRemote(nativeText?: string): Promise<boolean>;
  /** The remote reported that its clipboard channel is (un)available. */
  setChannelAvailable(available: boolean): void;
  /** Start/stop the polling timer and window focus/visibility listeners. */
  start(): void;
  stop(): void;
  lastError: Ref<string>;
  syncEnabled: Ref<boolean>;
}

const DEFAULT_POLL_INTERVAL_MS = 1500;
const DEFAULT_RETRY_DELAY_MS = 15000;
const DEFAULT_DENY_RETRY_DELAY_MS = 60000;
const READ_FAILURE_LIMIT = 3;
const NOTICE_THROTTLE_MS = 8000;
const PASTE_SETTLE_MS = 80;
/** A host read this recent is reused for an explicit paste instead of burning the
 *  host bridge's 1 read/second quota (and adding a retry delay to Ctrl+V). */
const HOST_READ_FRESH_MS = 900;
const DEFAULT_RECEIVED_NOTICE_KEY = "rdp.clipboardReceived";
const DEFAULT_ACCESS_DENIED_KEY = "rdp.clipboardAccessDenied";

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function isRateLimit(cause: unknown): boolean {
  return /rate[\s_-]*limit/i.test(messageOf(cause));
}

/** The host bridge remembers a denied consent for its whole bridge lifetime:
 *  retrying quickly can never succeed, so back off and tell the user how to
 *  grant access again (reopen the workbench tab recreates the bridge). */
function isDenied(cause: unknown): boolean {
  return /denied|not declared permission|unavailable/i.test(messageOf(cause));
}

/** Shared host↔remote clipboard engine for RDP and VNC.
 *
 * Invariants:
 * - Baselines are only advanced after the remote RPC / host write resolves, so a
 *   failed sync is retried instead of being treated as delivered.
 * - Reading the host and writing the remote clipboard both update both baselines,
 *   which is what suppresses echo loops.
 * - Polling is gated by session state and document visibility only. The plugin
 *   iframe is focused only while the remote canvas has focus, so a focus gate
 *   would stop syncing exactly when the user copies from another macOS app.
 * - No permanent failure flag: read failures pause polling; denied consent pauses
 *   it longer and shows an actionable notice.
 * - An explicit paste always sends the remote shortcut, even when the host read
 *   fails, so the physical key is never swallowed.
 */
export function useDesktopClipboard(options: DesktopClipboardOptions): DesktopClipboardSync {
  const {
    t,
    showError,
    showNotice,
    sessionId,
    active,
    setRemoteText,
    sendPasteShortcut,
    receivedNoticeKey = DEFAULT_RECEIVED_NOTICE_KEY,
    accessDeniedNoticeKey = DEFAULT_ACCESS_DENIED_KEY,
    now = () => Date.now(),
  } = options;
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  const retryDelayMs = options.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS;
  const denyRetryDelayMs = options.denyRetryDelayMs ?? DEFAULT_DENY_RETRY_DELAY_MS;

  const lastError = ref("");
  const syncEnabled = ref(true);

  let lastHostText = "";
  let lastRemoteText = "";
  /** The most recent text actually read from the host (paste reuses this even
   *  when pushing it to the remote failed). */
  let lastReadText = "";
  let lastHostReadAt = 0;
  let pendingRemoteText: string | null = null;
  let readFailures = 0;
  let pausedUntil = 0;
  let hostEpoch = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  let started = false;
  let syncInFlight = false;
  let remoteWriteInFlight = false;
  let pasteInFlight = false;
  let channelAvailable = true;
  const noticeAt = new Map<string, number>();

  function resolveDeps(): ClipboardDeps {
    if (options.deps) return options.deps();
    return {
      bridge: typeof window !== "undefined" ? window.dbxPlugin?.clipboard ?? null : null,
      nativeClipboard: typeof navigator !== "undefined" ? navigator.clipboard ?? null : null,
    };
  }

  /** The host bridge is authoritative when present: never bypass its consent or
   *  rate limit by falling through to the Web Clipboard API. */
  async function readHostText(): Promise<string> {
    const deps = resolveDeps();
    if (deps.bridge && typeof deps.bridge.readText === "function") {
      return deps.bridge.readText();
    }
    if (!clipboardReadAvailable(deps)) throw new Error("clipboard read unavailable");
    return readClipboardText(deps);
  }

  function throttled(kind: string, notify: () => void) {
    const last = noticeAt.get(kind);
    const time = now();
    if (last !== undefined && time - last < NOTICE_THROTTLE_MS) return;
    noticeAt.set(kind, time);
    notify();
  }

  function canPoll(): boolean {
    // Do not burn the host's 1/s quota while an explicit paste is reading it.
    if (pasteInFlight) return false;
    if (!syncEnabled.value || !channelAvailable || !active() || !sessionId.value) return false;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return false;
    return true;
  }

  function noteReadFailure(cause: unknown) {
    lastError.value = messageOf(cause);
    if (isDenied(cause)) {
      pausedUntil = now() + denyRetryDelayMs;
      throttled("host-read-denied", () => showNotice(t(accessDeniedNoticeKey)));
      return;
    }
    if (isRateLimit(cause)) return;
    readFailures += 1;
    if (readFailures < READ_FAILURE_LIMIT) return;
    readFailures = 0;
    pausedUntil = now() + retryDelayMs;
    throttled("host-read-failed", () => showError(cause, "terminal"));
  }

  async function flushPendingRemoteText(): Promise<void> {
    const text = pendingRemoteText;
    if (!text || remoteWriteInFlight) return;
    remoteWriteInFlight = true;
    try {
      // Event-driven writes must not steal focus through the textarea fallback:
      // that would blur the remote canvas and release held keys. Without any
      // host bridge it is the only write path left, so keep it there.
      const deps = resolveDeps();
      await writeClipboardText(text, deps, { execCommandFallback: !deps.bridge });
      lastRemoteText = text;
      lastHostText = text;
      lastReadText = text;
      lastHostReadAt = now();
      if (pendingRemoteText === text) pendingRemoteText = null;
      lastError.value = "";
      throttled("remote-received", () => showNotice(t(receivedNoticeKey)));
    } catch (cause) {
      lastError.value = messageOf(cause);
      throttled("host-write-failed", () => showError(cause, "terminal"));
    } finally {
      remoteWriteInFlight = false;
    }
  }

  function reset() {
    lastHostText = "";
    lastRemoteText = "";
    lastReadText = "";
    lastHostReadAt = 0;
    pendingRemoteText = null;
    readFailures = 0;
    pausedUntil = 0;
    channelAvailable = true;
    lastError.value = "";
    noticeAt.clear();
    hostEpoch += 1;
  }

  function setChannelAvailable(available: boolean) {
    if (channelAvailable === available) return;
    channelAvailable = available;
  }

  async function syncNow(): Promise<void> {
    if (!canPoll() || syncInFlight || now() < pausedUntil) return;
    const sid = sessionId.value;
    if (!sid) return;
    syncInFlight = true;
    const epoch = hostEpoch;
    try {
      await flushPendingRemoteText();
      if (!canPoll() || sessionId.value !== sid) return;
      let text: string;
      try {
        text = await readHostText();
      } catch (cause) {
        noteReadFailure(cause);
        return;
      }
      if (!canPoll() || sessionId.value !== sid || epoch !== hostEpoch) return;
      readFailures = 0;
      lastError.value = "";
      lastReadText = text;
      lastHostReadAt = now();
      if (!text) {
        lastHostText = "";
        return;
      }
      if (text === lastHostText || text === lastRemoteText) return;
      try {
        await setRemoteText(sid, text);
        if (epoch === hostEpoch) {
          lastHostText = text;
          lastRemoteText = text;
        }
      } catch (cause) {
        lastError.value = messageOf(cause);
        throttled("remote-set-failed", () => showError(cause, "terminal"));
      }
    } finally {
      syncInFlight = false;
    }
  }

  async function receiveRemoteText(text: string): Promise<void> {
    if (!text) return;
    if (text === lastRemoteText && pendingRemoteText === null) return;
    pendingRemoteText = text;
    hostEpoch += 1;
    await flushPendingRemoteText();
  }

  async function setRemoteClipboard(text: string): Promise<boolean> {
    const sid = sessionId.value;
    if (!sid || !active() || !text) return false;
    if (text === lastRemoteText) return true;
    try {
      await setRemoteText(sid, text);
      // The host clipboard still holds its previous text: keep the host
      // baseline and the read cache untouched so the next poll does not
      // overwrite this manual remote clipboard until the host really changes.
      lastRemoteText = text;
      return true;
    } catch (cause) {
      lastError.value = messageOf(cause);
      throttled("remote-set-failed", () => showError(cause, "terminal"));
      return false;
    }
  }

  /** Explicit paste is a user gesture: if the background poll consumed the
   *  host bridge's 1/s read slot, wait one interval and retry once instead of
   *  leaving Ctrl+V a no-op. */
  async function readHostTextForPaste(): Promise<string> {
    try {
      return await readHostText();
    } catch (cause) {
      if (!isRateLimit(cause)) throw cause;
      await new Promise<void>((resolve) => setTimeout(resolve, 1100));
      return readHostText();
    }
  }

  async function pasteToRemote(nativeText?: string): Promise<boolean> {
    const sid = sessionId.value;
    if (!sid || !active() || pasteInFlight) return false;
    pasteInFlight = true;
    try {
      return await performPaste(sid, nativeText);
    } finally {
      pasteInFlight = false;
    }
  }

  async function performPaste(sid: string, nativeText?: string): Promise<boolean> {
    let text = nativeText;
    if (text === undefined) {
      if (lastHostReadAt > 0 && now() - lastHostReadAt < HOST_READ_FRESH_MS) {
        text = lastReadText;
      } else {
        try {
          text = await readHostTextForPaste();
          readFailures = 0;
          pausedUntil = 0;
          lastReadText = text;
          lastHostReadAt = now();
        } catch (cause) {
          noteReadFailure(cause);
        }
      }
    }
    if (sessionId.value !== sid) return false;
    let staged = true;
    if (text && text !== lastRemoteText) {
      try {
        await setRemoteText(sid, text);
      } catch (cause) {
        // Never advance the baselines on a failed stage: the next poll/paste
        // must retry instead of treating the text as already delivered.
        staged = false;
        lastError.value = messageOf(cause);
        throttled("paste-set-failed", () => showError(cause, "terminal"));
      }
    }
    if (text && staged) {
      lastHostText = text;
      lastRemoteText = text;
      lastReadText = text;
    }
    // Let the sidecar advertise/stage the clipboard update before the remote reads it.
    if (text && staged) await new Promise<void>((resolve) => setTimeout(resolve, PASTE_SETTLE_MS));
    // Always forward the user's paste: the remote may still hold the previously
    // staged text, and swallowing the key is worse than pasting stale content.
    sendPasteShortcut();
    return true;
  }

  function tick() {
    if (!canPoll() || now() < pausedUntil) return;
    void syncNow();
  }

  function onWindowFocus() {
    void syncNow();
  }

  function onVisibilityChange() {
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    void syncNow();
  }

  function start() {
    if (started) return;
    started = true;
    timer = setInterval(tick, pollIntervalMs);
    if (typeof window !== "undefined") {
      window.addEventListener("focus", onWindowFocus);
    }
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisibilityChange);
    }
  }

  function stop() {
    if (timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
    if (typeof window !== "undefined") {
      window.removeEventListener("focus", onWindowFocus);
    }
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", onVisibilityChange);
    }
    started = false;
  }

  if (getCurrentScope()) onScopeDispose(stop);

  return {
    reset,
    syncNow,
    receiveRemoteText,
    setRemoteClipboard,
    pasteToRemote,
    setChannelAvailable,
    start,
    stop,
    lastError,
    syncEnabled,
  };
}
