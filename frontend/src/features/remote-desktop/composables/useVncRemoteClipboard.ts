import { getCurrentScope, onScopeDispose, ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";

export interface VncRemoteClipboardOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  /** Active transfer session, or null when the window is disconnected. */
  sessionId: () => string | null;
  /** VNC session that owns `vnc/clipboard-files` events. */
  vncSessionId?: () => string | null;
  /** VNC desktop is running (RFB text sync alone cannot carry files). */
  active: () => boolean;
  /** A manual transfer is in flight; do not compete with it. */
  busy: () => boolean;
  pollIntervalMs?: number;
  probeDelayMs?: number;
}

const DEFAULT_POLL_MS = 3_000;
const DEFAULT_PROBE_DELAY_MS = 200;
/** A remote file copy only becomes a host-side download when the user leaves
 *  the VNC window shortly afterwards (copy there, paste here). Staying longer
 *  means they intend to paste inside the remote desktop. */
const PENDING_TTL_MS = 15_000;

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/** Makes VNC "copy file in the desktop → Cmd+V on the Mac" work.
 *
 * RFB has no file clipboard: a remote file copy updates only the X selection
 * on the VNC host. While a transfer session is connected we read that
 * selection through the SSH companion (xclip/xsel). We deliberately do not
 * download on copy: a remote-only copy/paste must not replace the Mac
 * clipboard or move bytes. The pending URI list is fetched only when the VNC
 * window loses focus within a short window, which is the "paste on the Mac"
 * gesture; a slow poll catches right-click/menu copies. */
export function useVncRemoteClipboard(options: VncRemoteClipboardOptions) {
  const supported = ref<boolean | null>(null);
  const fetching = ref(false);
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_MS;
  const probeDelayMs = options.probeDelayMs ?? DEFAULT_PROBE_DELAY_MS;
  let timer: ReturnType<typeof setInterval> | undefined;
  let probeTimer: ReturnType<typeof setTimeout> | undefined;
  let unsubscribe: (() => void) | null = null;
  let checking = false;
  let pendingRequest = "";
  let pendingPaths: string[] = [];
  let pendingAt = 0;
  let lastSignature = "";
  let warnedUnsupported = false;

  function sessionId(): string | null {
    if (!options.active()) return null;
    return options.sessionId();
  }

  function vncSessionId(): string {
    return options.vncSessionId?.() ?? "";
  }

  function clearPending() {
    pendingPaths = [];
    pendingAt = 0;
  }

  /** Records a new remote file list without downloading it. */
  function remember(paths: string[]): boolean {
    if (!paths.length) return false;
    const signature = paths.join("\n");
    if (signature === lastSignature) return false;
    lastSignature = signature;
    pendingPaths = paths;
    pendingAt = Date.now();
    return true;
  }

  function matchesVnc(params: Record<string, unknown>): boolean {
    const expected = vncSessionId();
    return !expected || text(params.sessionId) === expected;
  }

  function subscribe() {
    const api = window.dbxPlugin;
    if (!api?.onEvent) return;
    unsubscribe = api.onEvent((event) => {
      if (!("method" in event)) return;
      const params = (event.params ?? {}) as Record<string, unknown>;
      // Some VNC servers forward a file copy as its URI list. Keep the paths
      // for the blur fetch; no xclip probe (or remote tool) is required.
      if (event.method === "vnc/clipboard-files") {
        if (!options.active() || !matchesVnc(params)) return;
        const paths = Array.isArray(params.paths)
          ? (params.paths as unknown[]).filter((path): path is string => typeof path === "string" && !!path)
          : [];
        remember(paths);
        return;
      }
      // A text copy replaced the remote selection: a previously remembered
      // file list must not override that text on the Mac.
      if (event.method === "vnc/clipboard") {
        if (options.active() && matchesVnc(params)) clearPending();
        return;
      }
      if (event.method !== "filetransfer/clipboard/ready" && event.method !== "filetransfer/clipboard/error") return;
      if (!pendingRequest || text(params.requestId) !== pendingRequest) return;
      const sid = sessionId();
      if (sid && text(params.sessionId) !== sid) return;
      pendingRequest = "";
      fetching.value = false;
      if (event.method === "filetransfer/clipboard/error") {
        lastSignature = "";
        options.showError(new Error(text(params.error) || "Remote clipboard download failed"), "terminal");
        return;
      }
      const localPaths = Array.isArray(params.localPaths)
        ? (params.localPaths as unknown[]).filter((path): path is string => typeof path === "string")
        : [];
      const failed = count(params.failed);
      const clipboardError = text(params.clipboardError);
      if (clipboardError) {
        options.showError(new Error(clipboardError), "terminal");
        return;
      }
      const copied = params.clipboard === true && localPaths.length > 0;
      if (copied) {
        options.showNotice(
          options.t(
            failed > 0 ? "remoteFiles.remoteCopyReadyPartial" : "remoteFiles.remoteCopyReady",
            { count: localPaths.length, failed },
          ),
        );
      } else {
        options.showNotice(
          options.t("remoteFiles.remoteCopySaved", {
            count: localPaths.length,
            dir: text(params.dir),
          }),
        );
      }
    });
  }

  async function fetchFiles(sid: string, paths: string[]) {
    const requestId = `clipboard-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    pendingRequest = requestId;
    fetching.value = true;
    try {
      await rpc("filetransfer/clipboard/fetch", {
        sessionId: sid,
        remotePaths: paths,
        requestId,
      });
      options.showNotice(options.t("remoteFiles.remoteCopyDetected", { count: paths.length }));
    } catch (cause) {
      pendingRequest = "";
      fetching.value = false;
      lastSignature = "";
      options.showError(cause, "terminal");
    }
  }

  /** Downloads a remembered file list if the user left the VNC window in time. */
  async function flushPending() {
    if (!pendingPaths.length) return;
    if (Date.now() - pendingAt > PENDING_TTL_MS) {
      clearPending();
      return;
    }
    if (fetching.value || options.busy()) return;
    const sid = options.sessionId();
    if (!sid) return;
    const paths = pendingPaths;
    clearPending();
    await fetchFiles(sid, paths);
  }

  /** Returns true when the remote clipboard tool answered (even with no
   *  files); false when the host cannot expose file URIs at all. */
  async function check(): Promise<boolean> {
    const sid = sessionId();
    if (!sid || checking || fetching.value || options.busy()) return true;
    checking = true;
    try {
      const result = await rpc<{ supported?: boolean; paths?: unknown[] }>(
        "filetransfer/clipboard/remote",
        { sessionId: sid },
      );
      supported.value = result.supported === true;
      if (!supported.value) return false;
      const paths = (result.paths ?? []).filter((path): path is string => typeof path === "string" && !!path);
      remember(paths);
      return true;
    } catch {
      // A dropped/restarting transfer session is not a clipboard verdict.
      return true;
    } finally {
      checking = false;
    }
  }

  /** Copy-shortcut probe: discover the remote file list, never download yet. */
  function probe() {
    if (!sessionId()) return;
    if (probeTimer !== undefined) clearTimeout(probeTimer);
    probeTimer = setTimeout(() => {
      probeTimer = undefined;
      void check().then((ok) => {
        if (ok || warnedUnsupported) return;
        warnedUnsupported = true;
        options.showNotice(options.t("remoteFiles.remoteClipboardUnavailable"));
      });
    }, probeDelayMs);
  }

  function onWindowBlur() {
    if (!options.active()) return;
    // The copy-shortcut probe usually resolved while the user was still in the
    // canvas; start the download immediately instead of paying another probe.
    if (pendingPaths.length) {
      void flushPending();
      return;
    }
    void check().then(() => flushPending());
  }

  function start() {
    if (timer !== undefined) return;
    timer = setInterval(() => void check(), pollIntervalMs);
    if (typeof window !== "undefined") window.addEventListener("blur", onWindowBlur);
  }

  function stop() {
    if (timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
    if (probeTimer !== undefined) {
      clearTimeout(probeTimer);
      probeTimer = undefined;
    }
    if (typeof window !== "undefined") window.removeEventListener("blur", onWindowBlur);
  }

  subscribe();
  start();

  if (getCurrentScope()) {
    onScopeDispose(() => {
      stop();
      unsubscribe?.();
      unsubscribe = null;
    });
  }

  return { supported, fetching, probe, check, start, stop };
}

export type VncRemoteClipboardController = ReturnType<typeof useVncRemoteClipboard>;
