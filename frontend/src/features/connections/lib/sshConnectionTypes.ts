import type { Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { ConnectLog } from "./connectLog";
import type { SftpSoftError } from "../../sftp/composables/listing/useSftpNavigation";
import type { ModifiedPrompt, WatchRegistry } from "../../sftp/lib/editor/watchEdits";

export interface ReplayResult {
  frameCount: number;
  firstAvailableSequence: number;
  tailSequence: number;
  complete: boolean;
}

export interface SessionInfo {
  sessionId: string;
  connectionId: string;
  workbenchId?: string;
  connected: boolean;
  sequence: number;
  chunkSize: number;
  directoryTrackingSupported?: boolean;
  replay?: ReplayResult;
}

/** host-key 挑战应答所需的最小载荷（完整弹窗状态留在 App.vue）。 */
export interface HostKeyChallenge {
  challengeId: string;
  operationId: string;
}

/** 重连梯子（useSessionReconnect 持有，在本组合式函数之后装配）。 */
export interface SshReconnectState {
  reconnectPending: Ref<boolean>;
  reconnectAttempt: Ref<number>;
  reconnectNextAt: Ref<number>;
  reconnectDelayMs: Ref<number>;
  reconnectTimer: Ref<number>;
  reconnectWasPending: Ref<boolean>;
}

/** 连接成功后的 SFTP 侧效应（useSftpNavigation / useSftpTreeWorkspace 持有，
 *  均在本组合式函数之后装配）。 */
export interface SshSftpHooks {
  loadDirectory: (path: string, fromTerminal?: boolean, quiet?: boolean) => Promise<void>;
  refreshSftpHomePath: () => Promise<void>;
  setDirectoryTracking: (enabled: boolean) => Promise<void>;
  initializeSftpTree: () => void;
  resetSftpTree: () => void;
}

/** SSH 输出管线仍由 App.vue 持有（Task 16 再迁出）：会话启停只经这组读写口
 *  触及 sequence/pending/resync 计数，避免与本地/串口帧流互染。 */
export interface SshFrameStreams {
  setActiveTerminalSessionId: (sessionId: string) => void;
  getLastSequence: () => number;
  setLastSequence: (sequence: number) => void;
  pendingTerminalFrames: Map<number, { stream: number; data: Uint8Array }>;
  setReplayNoProgress: (value: number) => void;
}

export interface SshConnectionOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  connectLog: ConnectLog;
  connectCancelled: Ref<boolean>;
  connectSucceeded: Ref<boolean>;
  hostKeyPrompt: Ref<HostKeyChallenge | undefined>;
  rememberHostKey: Ref<boolean>;
  terminalState: Ref<"connecting" | "connected" | "disconnected" | "error">;
  terminalError: Ref<string>;
  session: Ref<SessionInfo | undefined>;
  connectionId: Readonly<Ref<string>>;
  workbenchId: Readonly<Ref<string>>;
  connectionProtocol: Readonly<Ref<"ssh" | "telnet" | "vnc" | "serial" | "rdp">>;
  storedConnectionProtocol: Ref<string>;
  directoryTrackingSupported: Ref<boolean | undefined>;
  sftpListingLoaded: Ref<boolean>;
  panelSurface: Readonly<Ref<boolean>>;
  hostContext: Ref<Record<string, unknown>>;
  followDirectory: Readonly<Ref<boolean>>;
  sftpAutoConnect: Readonly<Ref<boolean>>;
  currentPath: Readonly<Ref<string>>;
  terminalInputQueue: { reset: () => void };
  isDisposed: () => boolean;
  getTerminal: () => Terminal | undefined;
  createTerminal: () => void;
  fitTerminalDimensions: () => { cols: number; rows: number };
  scheduleFit: () => void;
  resetCommandMarker: () => void;
  initialState: () => { sessionId?: string };
  persistState: () => void;
  writeWorkbenchState: () => Promise<void>;
  loadRemoteShellHistory: (sessionId: string) => void | Promise<void>;
  restoreTransfers: () => void | Promise<void>;
  cancelZmodemUpload: () => void;
  teardownTrzsz: () => void;
  startTelnetFromConnection: () => Promise<boolean>;
  telnetDialogOpen: Ref<boolean>;
  startVncFromConnection: () => Promise<boolean>;
  vncDialogOpen: Ref<boolean>;
  startSerialFromConnection: () => Promise<boolean>;
  serialDialogOpen: Ref<boolean>;
  routeRdpConnection: () => Promise<void>;
  startLocalTerminal: (
    shellOverride?: string,
    connection?: { cwd?: string; env?: Array<{ name: string; value: string }> },
  ) => Promise<void>;
  // 装配依赖（声明/装配在本组合式函数之后）：一律 getter 惰性接入；函数内
  // 按原名解构，迁出代码保持逐行原样。
  getReconnectState: () => SshReconnectState;
  getAgentTerminalMode: () => { clearAgentPrompts: () => void; agentRunning: Ref<unknown> };
  getSftpHooks: () => SshSftpHooks;
  getSftpSoftError: () => Ref<SftpSoftError | null>;
  getExternalEditState: () => {
    activeExternalWatches: Ref<WatchRegistry>;
    watchModifiedQueue: Ref<ModifiedPrompt[]>;
  };
  rescanHighlightViewport: () => void;
  probeLocalCapabilities: () => void;
  frameStreams: SshFrameStreams;
}
