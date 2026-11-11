import { computed, onScopeDispose, reactive, ref } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import { createRequestEpoch } from "../../../shared/lib/requestEpoch";
import {
  applySavedTransferForm,
  canUseDesktopCredentials,
  createTransferPasswordCache,
  emptyTransferPane,
  forgetSavedTransferPassword,
  isMissingStoredCredentials,
  loadTransferList,
  mapLocalTransferEntry,
  mapRemoteTransferEntry,
  normalizeRemoteTransferPath,
  persistTransferForm,
  remoteParentPath,
  transferErrorMessage,
  transferNumber,
  transferText,
  type TransferAuthentication,
  type TransferConnectForm,
  type TransferEntry,
  type TransferPaneState,
  type TransferProgress,
  type TransferSession,
} from "../lib/remoteFileTransfer";

export type {
  TransferAuthentication,
  TransferDirection,
  TransferEntry,
  TransferPaneState,
  TransferProgress,
  TransferSession,
  TransferStatus,
} from "../lib/remoteFileTransfer";

export interface RemoteFileTransferOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  workbenchId: () => string;
  connectionId: () => string;
  initialHost: () => string;
  initialUsername: () => string;
}

const CONNECT_TIMEOUT_MS = 90_000;

export function useRemoteFileTransfer(options: RemoteFileTransferOptions) {
  const session = ref<TransferSession | null>(null);
  const connecting = ref(false);
  const connectError = ref("");
  const local = reactive<TransferPaneState>(emptyTransferPane());
  const remote = reactive<TransferPaneState>(emptyTransferPane());
  const form = reactive<TransferConnectForm>({
    host: "",
    port: "22",
    username: "",
    authentication: "password",
    password: "",
    privateKeyPath: "",
    privateKeyPassphrase: "",
  });
  const progress = ref<TransferProgress | null>(null);
  const lastDownloaded = ref<string[]>([]);
  const clipboardSupported = ref(false);
  const pendingLocalPaths = ref<string[]>([]);
  const initialized = ref(false);
  const passwords = createTransferPasswordCache();
  let activeTaskId = "";
  let unsubscribe: (() => void) | null = null;
  let prefilledConnection = "";
  let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  // Only the newest list request may publish (see loadTransferList in lib).
  const localEpoch = createRequestEpoch();
  const remoteEpoch = createRequestEpoch();
  const reportListError = (cause: unknown) => options.showError(cause, "terminal");

  const busy = computed(() => progress.value?.status === "running");
  const canUpload = computed(
    () => !!session.value && !busy.value && (local.selected.length > 0 || pendingLocalPaths.value.length > 0),
  );
  const canDownload = computed(() => !!session.value && !busy.value && remote.selected.length > 0);

  applySavedTransferForm(form);

  function prefill() {
    const host = options.initialHost();
    if (!host || host === prefilledConnection) return;
    prefilledConnection = host;
    // Keep a previously used custom SSH host/user: the saved encrypted
    // password is keyed by them, and overwriting would force a re-entry.
    if (!form.host.trim()) form.host = host;
    const username = options.initialUsername();
    if (!form.username.trim() && username) form.username = username;
  }

  function clearSelection(pane: TransferPaneState) {
    pane.selected.splice(0, pane.selected.length);
  }

  /** Reconnect silently from the encrypted saved password, this webview's
   *  cache, saved key/agent settings, or the desktop connection's credentials. */
  function autoConnect(useDesktopCredentials = false) {
    if (session.value || connecting.value) return;
    prefill();
    initialize();
    if (!passwords.apply(form, true)) return;
    void connect({
      auto: true,
      useDesktopCredentials:
        useDesktopCredentials && canUseDesktopCredentials(form, options.connectionId(), options.initialHost()),
    });
  }

  function toggleSelection(pane: TransferPaneState, entry: TransferEntry) {
    const index = pane.selected.indexOf(entry.path);
    if (index >= 0) pane.selected.splice(index, 1);
    else pane.selected.push(entry.path);
  }

  function isSelected(pane: TransferPaneState, entry: TransferEntry): boolean {
    return pane.selected.includes(entry.path);
  }

  async function loadLocal(path?: string) {
    await loadTransferList(local, localEpoch,
      () => rpc<Record<string, unknown>>("filetransfer/local/list", path ? { path } : {}),
      (result) => {
        local.path = transferText(result.path);
        local.parent = typeof result.parent === "string" ? result.parent : null;
        local.home = typeof result.home === "string" ? result.home : null;
        local.draft = local.path;
        local.entries = Array.isArray(result.entries)
          ? (result.entries as Array<Record<string, unknown>>).map(mapLocalTransferEntry)
          : [];
      }, reportListError);
  }

  async function loadRemote(path?: string) {
    const current = session.value;
    if (!current) return;
    const target = normalizeRemoteTransferPath(path || remote.path || current.home);
    await loadTransferList(remote, remoteEpoch,
      () => rpc<{ entries?: Array<Record<string, unknown>> }>("sftp/list", {
        sessionId: current.sessionId,
        path: target,
      }),
      (result) => {
        remote.path = target;
        remote.parent = remoteParentPath(target);
        remote.draft = target;
        remote.entries = (result.entries ?? []).map(mapRemoteTransferEntry);
      }, reportListError);
  }

  async function refreshClipboardSupport() {
    try {
      const result = await rpc<{ supported?: boolean }>("filetransfer/clipboard/files");
      clipboardSupported.value = result.supported === true;
    } catch {
      clipboardSupported.value = false;
    }
  }

  function stopHeartbeat() {
    if (heartbeatTimer !== null) {
      clearInterval(heartbeatTimer);
      heartbeatTimer = null;
    }
  }

  function handleSessionLost(expectedSessionId: string) {
    if (!session.value || session.value.sessionId !== expectedSessionId) return;
    stopHeartbeat();
    session.value = null;
    activeTaskId = "";
    progress.value = null;
    lastDownloaded.value = [];
    Object.assign(remote, emptyTransferPane());
    options.showNotice(options.t("remoteFiles.sessionLost"));
  }

  function startHeartbeat() {
    stopHeartbeat();
    heartbeatTimer = setInterval(() => {
      const current = session.value;
      if (!current) {
        stopHeartbeat();
        return;
      }
      void rpc<{ alive?: boolean }>("filetransfer/ping", { sessionId: current.sessionId })
        .then((result) => {
          if (result.alive === false) handleSessionLost(current.sessionId);
        })
        // Transient bridge errors are not proof the session died; the next
        // heartbeat (or the next operation) surfaces a real loss.
        .catch(() => undefined);
    }, 30_000);
  }

  function handleProgress(raw: Record<string, unknown>) {
    // Background clipboard fetches must not hijack the window's aggregate bar.
    if (raw.background === true) return;
    const taskId = transferText(raw.taskId);
    if (!taskId) return;
    if (!session.value || transferText(raw.sessionId) !== session.value.sessionId) return;
    if (activeTaskId && activeTaskId !== taskId) return;
    activeTaskId = taskId;
    const direction = raw.direction === "download" ? "download" : "upload";
    const status = raw.status;
    const normalizedStatus: TransferProgress["status"] =
      status === "done" || status === "error" || status === "canceled" ? status : "running";
    const previous = progress.value;
    const filesDone = transferNumber(raw.filesDone);
    const filesTotal = transferNumber(raw.filesTotal);
    const bytesTotal = transferNumber(raw.bytesTotal);
    // The backend zeroes counters on failure/cancel; keep the last known
    // context so the footer does not lose "3/5" when a push fails.
    const preserve =
      (normalizedStatus === "error" || normalizedStatus === "canceled") &&
      filesDone === 0 &&
      filesTotal === 0 &&
      bytesTotal === 0;
    progress.value = {
      taskId,
      direction,
      status: normalizedStatus,
      file: preserve && previous ? previous.file : transferText(raw.file),
      filesDone: preserve && previous ? previous.filesDone : filesDone,
      filesTotal: preserve && previous ? previous.filesTotal : filesTotal,
      bytesDone: preserve && previous ? previous.bytesDone : transferNumber(raw.bytesDone),
      bytesTotal: preserve && previous ? previous.bytesTotal : bytesTotal,
      percent: preserve && previous ? previous.percent : transferNumber(raw.percent),
      ...(typeof raw.error === "string" && raw.error ? { error: raw.error } : {}),
    };
  }

  function handleDone(raw: Record<string, unknown>) {
    const taskId = transferText(raw.taskId);
    if (!taskId || (activeTaskId && activeTaskId !== taskId)) return;
    if (!session.value || transferText(raw.sessionId) !== session.value.sessionId) return;
    activeTaskId = "";
    const direction = progress.value?.direction;
    const filesDone = transferNumber(raw.filesDone, progress.value?.filesDone ?? 0);
    const skipped = transferNumber(raw.skipped);
    const failed = transferNumber(raw.failed);
    if (progress.value) {
      progress.value = { ...progress.value, status: "done", percent: 100 };
    }
    if (direction === "download") {
      lastDownloaded.value = Array.isArray(raw.localPaths)
        ? (raw.localPaths as unknown[]).filter((path): path is string => typeof path === "string")
        : [];
      clearSelection(remote);
      options.showNotice(
        failed > 0
          ? options.t("remoteFiles.downloadedPartial", { count: filesDone, failed })
          : options.t("remoteFiles.downloaded", { count: filesDone }),
      );
      void loadLocal(local.path || undefined);
    } else {
      clearSelection(local);
      pendingLocalPaths.value = [];
      options.showNotice(
        skipped > 0
          ? options.t("remoteFiles.uploadedSkipped", { count: filesDone, skipped })
          : options.t("remoteFiles.uploaded", { count: filesDone }),
      );
      void loadRemote(remote.path);
    }
  }

  function subscribe() {
    const api = window.dbxPlugin;
    if (!api?.onEvent) return;
    unsubscribe = api.onEvent((event) => {
      if (!("method" in event)) return;
      if (event.method === "filetransfer/progress") {
        handleProgress((event.params ?? {}) as Record<string, unknown>);
      } else if (event.method === "filetransfer/done") {
        handleDone((event.params ?? {}) as Record<string, unknown>);
      }
    });
  }

  function initialize() {
    if (initialized.value) return;
    initialized.value = true;
    subscribe();
    void refreshClipboardSupport();
    if (!local.path) void loadLocal();
  }

  function open() {
    autoConnect(true);
  }

  async function connect(
    connectOptions: { auto?: boolean; useDesktopCredentials?: boolean } = {},
  ) {
    if (connecting.value) return;
    connectError.value = "";
    const host = form.host.trim();
    const username = form.username.trim();
    if (!host) {
      connectError.value = options.t("remoteFiles.hostRequired");
      return;
    }
    if (!username) {
      connectError.value = options.t("remoteFiles.usernameRequired");
      return;
    }
    if (form.authentication === "password" && !form.password && !connectOptions.auto) {
      connectError.value = options.t("remoteFiles.passwordRequired");
      return;
    }
    if (
      (form.authentication === "private-key" || form.authentication === "private-key-password") &&
      !form.privateKeyPath.trim()
    ) {
      connectError.value = options.t("remoteFiles.keyRequired");
      return;
    }
    const port = Number.parseInt(form.port, 10);
    connecting.value = true;
    try {
      const result = await rpc<{
        sessionId: string;
        home: string;
        host?: string;
        username?: string;
      }>(
        "filetransfer/connect",
        {
          workbenchId: options.workbenchId(),
          connectionId: options.connectionId() || undefined,
          useDesktopCredentials:
            connectOptions.useDesktopCredentials ??
            canUseDesktopCredentials(form, options.connectionId(), options.initialHost()),
          host,
          port: Number.isInteger(port) && port > 0 && port <= 65535 ? port : 22,
          username,
          authentication: form.authentication,
          password: form.password,
          privateKeyPath: form.privateKeyPath.trim(),
          privateKeyPassphrase: form.privateKeyPassphrase,
        },
        { timeoutMs: CONNECT_TIMEOUT_MS },
      );
      session.value = {
        sessionId: result.sessionId,
        home: result.home || "/",
        host: result.host || host,
        username: result.username || username,
      };
      persistTransferForm(form);
      passwords.remember(form);      startHeartbeat();
      await loadRemote(result.home || "/");
      if (!local.path) await loadLocal();
    } catch (cause) {
      passwords.forget(form);
      // A cached password that stopped working must not be retried silently.
      if (connectOptions.auto) form.password = "";
      const message = transferErrorMessage(cause);
      // Having no stored desktop credentials is expected on first use.
      connectError.value =
        connectOptions.auto && isMissingStoredCredentials(message) ? "" : message;
    } finally {
      connecting.value = false;
    }
  }

  async function forgetSavedPassword() {
    try {
      await forgetSavedTransferPassword(form);
      passwords.forget(form);
      form.password = "";
      connectError.value = "";
      options.showNotice(options.t("remoteFiles.passwordForgotten"));
    } catch (cause) {
      options.showError(cause, "terminal");
    }
  }

  async function disconnect() {
    if (busy.value) void cancelTransfer();
    stopHeartbeat();
    const current = session.value;
    session.value = null;
    activeTaskId = "";
    progress.value = null;
    lastDownloaded.value = [];
    pendingLocalPaths.value = [];
    Object.assign(remote, emptyTransferPane());
    connectError.value = "";
    if (!current) return;
    await rpc("filetransfer/disconnect", { sessionId: current.sessionId }).catch(() => undefined);
  }

  async function startUpload(paths: string[]) {
    const current = session.value;
    if (!current || !remote.path || paths.length === 0) return;
    const taskId = `local-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    activeTaskId = "";
    progress.value = {
      taskId,
      direction: "upload",
      status: "running",
      file: "",
      filesDone: 0,
      filesTotal: paths.length,
      bytesDone: 0,
      bytesTotal: 0,
      percent: 0,
    };
    try {
      const result = await rpc<{ taskId: string }>("filetransfer/upload/start", {
        sessionId: current.sessionId,
        localPaths: paths,
        remoteDir: remote.path,
        conflict: "rename",
      });
      if (!activeTaskId) activeTaskId = result.taskId;
      if (progress.value) progress.value.taskId = activeTaskId;
    } catch (cause) {
      activeTaskId = "";
      progress.value = null;
      options.showError(cause, "terminal");
    }
  }

  async function startDownload(paths: string[]) {
    const current = session.value;
    if (!current || !local.path || paths.length === 0) return;
    const taskId = `remote-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    activeTaskId = "";
    progress.value = {
      taskId,
      direction: "download",
      status: "running",
      file: "",
      filesDone: 0,
      filesTotal: paths.length,
      bytesDone: 0,
      bytesTotal: 0,
      percent: 0,
    };
    try {
      const result = await rpc<{ taskId: string }>("filetransfer/download/start", {
        sessionId: current.sessionId,
        remotePaths: paths,
        localDir: local.path,
        conflict: "rename",
      });
      if (!activeTaskId) activeTaskId = result.taskId;
      if (progress.value) progress.value.taskId = activeTaskId;
    } catch (cause) {
      activeTaskId = "";
      progress.value = null;
      options.showError(cause, "terminal");
    }
  }

  function upload() {
    void startUpload(pendingLocalPaths.value.length ? pendingLocalPaths.value : local.selected.slice());
  }

  function download() {
    void startDownload(remote.selected.slice());
  }

  async function cancelTransfer() {
    if (!activeTaskId || !session.value) return;
    await rpc("filetransfer/cancel", {
      taskId: activeTaskId,
      sessionId: session.value.sessionId,
    }).catch(() => undefined);
  }

  async function writeDownloadedToClipboard() {
    if (!lastDownloaded.value.length) return;
    try {
      await rpc("filetransfer/clipboard/write", { paths: lastDownloaded.value });
      options.showNotice(options.t("remoteFiles.clipboardWritten", { count: lastDownloaded.value.length }));
    } catch (cause) {
      options.showError(cause, "terminal");
    }
  }

  function revealDownloaded() {
    const first = lastDownloaded.value[0];
    if (!first) return;
    void rpc("local/reveal", { path: first }).catch((cause) => options.showError(cause, "terminal"));
  }

  /** VNC/RDP paste with a Finder file list: the caller opens the window with
   * those paths queued for upload. Returns [] when the clipboard has no files
   * or the platform is unsupported. */
  async function detectClipboardFiles(): Promise<string[]> {
    try {
      const result = await rpc<{ supported?: boolean; files?: unknown[] }>(
        "filetransfer/clipboard/files",
      );
      clipboardSupported.value = result.supported === true;
      return (result.files ?? []).filter((path): path is string => typeof path === "string");
    } catch {
      return [];
    }
  }

  function queueLocalPaths(paths: string[]) {
    const unique = Array.from(new Set(paths.filter((path) => typeof path === "string" && path)));
    pendingLocalPaths.value = unique;
  }

  function clearPendingLocalPaths() {
    pendingLocalPaths.value = [];
  }

  function enter(pane: TransferPaneState, entry: TransferEntry) {
    if (entry.kind !== "directory") return;
    if (pane === local) void loadLocal(entry.path);
    else void loadRemote(entry.path);
  }

  function goUp(pane: TransferPaneState) {
    if (!pane.parent) return;
    if (pane === local) void loadLocal(pane.parent);
    else void loadRemote(pane.parent);
  }

  function navigate(pane: TransferPaneState, path: string) {
    const target = path.trim();
    if (!target) return;
    if (pane === local) void loadLocal(target);
    else void loadRemote(target);
  }
  onScopeDispose(() => {
    unsubscribe?.();
    unsubscribe = null;
    stopHeartbeat();
    const current = session.value;
    session.value = null;
    if (current) {
      void rpc("filetransfer/disconnect", { sessionId: current.sessionId }).catch(() => undefined);
    }
  });

  return {
    session,
    connecting,
    connectError,
    local,
    remote,
    form,
    progress,
    lastDownloaded,
    clipboardSupported,
    pendingLocalPaths,
    busy,
    canUpload,
    canDownload,
    open,
    autoConnect,
    connect,
    forgetSavedPassword,
    disconnect,
    loadLocal,
    loadRemote,
    refreshClipboardSupport,
    upload,
    download,
    cancelTransfer,
    writeDownloadedToClipboard,
    revealDownloaded,
    detectClipboardFiles,
    queueLocalPaths,
    clearPendingLocalPaths,
    enter,
    goUp,
    navigate,
    toggleSelection,
    isSelected,
  };
}

export type RemoteFileTransferController = ReturnType<typeof useRemoteFileTransfer>;
