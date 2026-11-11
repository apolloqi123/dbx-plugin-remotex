import { loadLastConnectParams, persistLastConnectParams } from "../../connections/lib/connectLastParams";
import { rpc } from "../../../shared/lib/rpc";
import type { RequestEpoch } from "../../../shared/lib/requestEpoch";

export type TransferDirection = "upload" | "download";
export type TransferStatus = "running" | "done" | "error" | "canceled";
export type TransferAuthentication = "password" | "private-key" | "private-key-password" | "agent" | "none";

export interface TransferProgress {
  taskId: string;
  direction: TransferDirection;
  status: TransferStatus;
  file: string;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  percent: number;
  error?: string;
}

export interface TransferEntry {
  name: string;
  path: string;
  kind: "directory" | "file" | "symlink" | "other";
  size: number;
  modifiedAt: number | null;
  hidden: boolean;
  symlink: boolean;
}

export interface TransferPaneState {
  path: string;
  parent: string | null;
  home: string | null;
  entries: TransferEntry[];
  selected: string[];
  draft: string;
  loading: boolean;
  error: string;
}

export interface TransferSession {
  sessionId: string;
  home: string;
  host: string;
  username: string;
}

export interface TransferConnectForm {
  host: string;
  port: string;
  username: string;
  authentication: TransferAuthentication;
  password: string;
  privateKeyPath: string;
  privateKeyPassphrase: string;
}

export function emptyTransferPane(): TransferPaneState {
  return {
    path: "",
    parent: null,
    home: null,
    entries: [],
    selected: [],
    draft: "",
    loading: false,
    error: "",
  };
}

export function transferErrorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export function transferNumber(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function transferText(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

/** Per-controller only: the file transfer window must not persist SSH
 *  passwords. Keeping them here lets a reopened window auto-connect without a
 *  prompt, and dropping the cache with the workbench prevents cross-tab reuse. */
export function createTransferPasswordCache() {
  const passwords = new Map<string, string>();
  const key = (form: TransferConnectForm) =>
    [form.host.trim(), form.port.trim(), form.username.trim(), form.authentication].join("\u0000");
  return {
    remember(form: TransferConnectForm) {
      if (form.authentication === "password" && form.password) {
        passwords.set(key(form), form.password);
      }
    },
    forget(form: TransferConnectForm) {
      passwords.delete(key(form));
    },
    /** Fills a cached password (if any) and reports whether auto-connect can
     *  run. `allowConnectionCredentials` lets the backend retry with the
     *  RDP/VNC connection's stored password (same host/user) when no SSH
     *  password is cached yet. */
    apply(form: TransferConnectForm, allowConnectionCredentials = false): boolean {
      if (form.authentication === "password") {
        if (!form.password) {
          form.password = passwords.get(key(form)) ?? "";
        }
        if (!form.password && allowConnectionCredentials) {
          return !!form.host.trim() && !!form.username.trim();
        }
      }
      return transferAutoConnectReady(form);
    },
  };
}

export function transferAutoConnectReady(form: TransferConnectForm): boolean {
  if (!form.host.trim() || !form.username.trim()) return false;
  if (form.authentication === "password") return !!form.password;
  if (form.authentication === "private-key" || form.authentication === "private-key-password") {
    return !!form.privateKeyPath.trim();
  }
  return true;
}

/** Only the connection's own host may reuse its stored RDP/VNC password for
 *  the SSH companion; an edited host must go through the form. */
export function canUseDesktopCredentials(
  form: TransferConnectForm,
  connectionId: string,
  initialHost: string,
): boolean {
  return !!connectionId && form.host.trim() === initialHost.trim();
}

export function isMissingStoredCredentials(message: string): boolean {
  return /No stored SSH credentials/i.test(message);
}

const LAST_CONNECT_KEY = "remote-files-connect-last";

/** Restores the non-secret half of the transfer form (never a password). */
export function applySavedTransferForm(form: TransferConnectForm): void {
  const last = loadLastConnectParams<Record<string, unknown>>(LAST_CONNECT_KEY);
  if (typeof last.host === "string") form.host = last.host;
  if (typeof last.port === "string") form.port = last.port;
  if (typeof last.username === "string") form.username = last.username;
  if (
    last.authentication === "password" ||
    last.authentication === "private-key" ||
    last.authentication === "private-key-password" ||
    last.authentication === "agent" ||
    last.authentication === "none"
  ) {
    form.authentication = last.authentication;
  }
  if (typeof last.privateKeyPath === "string") form.privateKeyPath = last.privateKeyPath;
}

export async function forgetSavedTransferPassword(form: TransferConnectForm): Promise<void> {
  await rpc("filetransfer/credentials/forget", {
    host: form.host.trim(),
    port: Number.parseInt(form.port, 10) || 22,
    username: form.username.trim(),
  });
}

export function persistTransferForm(form: TransferConnectForm): void {
  persistLastConnectParams(LAST_CONNECT_KEY, {
    host: form.host,
    port: form.port,
    username: form.username,
    authentication: form.authentication,
    privateKeyPath: form.privateKeyPath,
  });
}

export function pathFromSftpUri(uri: string): string {
  return uri.replace(/^sftp:/, "") || "/";
}

export function normalizeRemoteTransferPath(path: string): string {
  let value = path.trim() || "/";
  if (!value.startsWith("/")) value = `/${value}`;
  value = value.replace(/\/{2,}/g, "/");
  return value === "/" ? value : value.replace(/\/+$/, "");
}

export function remoteParentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed || trimmed === "/") return null;
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
}

export function mapLocalTransferEntry(raw: Record<string, unknown>): TransferEntry {
  const kind = transferText(raw.kind);
  return {
    name: transferText(raw.name),
    path: transferText(raw.path),
    kind: kind === "directory" || kind === "file" || kind === "symlink" ? kind : "other",
    size: transferNumber(raw.size),
    modifiedAt: typeof raw.modifiedAt === "number" ? raw.modifiedAt : null,
    hidden: raw.hidden === true,
    symlink: raw.symlink === true,
  };
}

export function mapRemoteTransferEntry(raw: Record<string, unknown>): TransferEntry {
  const name = transferText(raw.name);
  const kind = transferText(raw.kind);
  return {
    name,
    path: pathFromSftpUri(transferText(raw.uri)),
    kind: kind === "directory" || kind === "file" || kind === "symlink" ? kind : "other",
    size: transferNumber(raw.size),
    modifiedAt: typeof raw.modifiedAt === "number" ? raw.modifiedAt : null,
    hidden: name.startsWith("."),
    symlink: kind === "symlink",
  };
}

/** Runs one list request under a monotonic epoch guard: a stale reply must not
 *  overwrite the directory, clear the spinner, or surface its error. */
export async function loadTransferList<T>(
  pane: TransferPaneState,
  epoch: RequestEpoch,
  fetchList: () => Promise<T>,
  apply: (result: T) => void,
  reportError: (cause: unknown) => void,
): Promise<void> {
  const id = epoch.next();
  pane.loading = true;
  pane.error = "";
  try {
    const result = await fetchList();
    if (!epoch.isCurrent(id)) return;
    apply(result);
    pane.selected.splice(0, pane.selected.length);
  } catch (cause) {
    if (!epoch.isCurrent(id)) return;
    pane.error = transferErrorMessage(cause);
    reportError(cause);
  } finally {
    if (epoch.isCurrent(id)) pane.loading = false;
  }
}
