<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { ImageAddon } from "@xterm/addon-image";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Disc,
  Film,
  Pause,
  Play,
  ArrowLeftRight,
  ArrowUpDown,
  Bot,
  Braces,
  ClipboardPaste,
  Scan,
  StretchHorizontal,
  Proportions,
  SlidersHorizontal,
  Copy,
  Eraser,
  ExternalLink,
  File as FileIcon,
  FileText,
  FileUp,
  Plus,
  FolderOpen,
  Gauge,
  ImagePlay,
  History,
  Info,
  ListChecks,
  Loader2,
  Network,
  Palette,
  PanelRightClose,
  Pencil,
  PlugZap,
  RefreshCw,
  Save,
  Search,
  Settings,
  ShieldCheck,
  Siren,
  Square,
  SquarePlus,
  ListPlus,
  SquareTerminal,
  Terminal as TerminalIcon,
  TextSelect,
  Trash2,
  TriangleAlert,
  X,
  Zap,
} from "@lucide/vue";
import {
  detectTrzszAnnounceFromBytes,
  resolveTerminalInputRoute,
} from "../features/terminal/lib/terminalTrzsz";
import { Osc7DirectoryParser } from "../features/terminal/lib/terminalDirectoryTracking";
import { PromptNewlineTransformer } from "../features/terminal/lib/terminalPromptNewline";
import { handleOsc52ClipboardWrite } from "../features/terminal/lib/terminalOsc";
import { useConfirmDialogHost } from "../shared/lib/confirmDialog";
import { subscribeHostEnvironment } from "../../../shared/frontend/hostThemeRuntime";
import {
  cwdFromUserVar,
  parseOsc1337SetUserVar,
  parseOsc777Notify,
  parseOsc9Notification,
  shouldOsc7FollowOverrideUserVarCwd,
} from "../features/terminal/lib/terminalOscChannels";
import {
  createTerminalCopyCache,
  resolveTerminalPasteText,
  isApplePlatform,
  normalizeDropTargetDir,
  resolveDropTargetDir,
  transferBusyPaneClass,
} from "../features/terminal/lib/terminalInteraction";
import { createTerminalWriteThrottle, type TerminalWriteThrottle } from "../features/terminal/lib/terminalWriteThrottle";
import { createOutputGate } from "../features/terminal/lib/terminalBackpressure";
import { createTerminalInputQueue, terminalInputChannel, TerminalInputTimeoutError } from "../features/terminal/lib/terminalInputQueue";
import { SERIAL_STREAM_STDIN, isKnownStreamTag, supportsBinaryInput } from "../features/serial/lib/serialTerminalFrames";
import { describeReconnectRestoredNotice, isConnectionInactiveError, isSessionGoneError, shouldReattachTerminal, terminalReconnectDelay, TERMINAL_RECONNECT_DELAYS } from "../features/terminal/lib/terminalReconnect";
import { TERMINAL_TRANSPORT_PROBE_TIMEOUT_MS, useTerminalTransportWatch } from "../features/terminal/composables/useTerminalTransportWatch";
import { useTerminalAiSearch } from "../features/terminal/composables/useTerminalAiSearch";
import { classifyConnectError, connectErrorKey } from "../features/connections/lib/connectError";
import { decideConnectRetry, isDuplicatedTransportUnavailableError } from "../features/connections/lib/connectRetry";
import { reopenHostConnection } from "../features/connections/lib/hostReopen";
import { workbenchCloseTargets } from "./lib/workbenchCloseSessions";
import {
  createSessionTransportReuseState,
  fallbackToFreshTransport,
  markSessionTransportOpenSucceeded,
  sessionTransportOpenParams,
} from "../features/connections/lib/sessionTransportReuse";
import { createConnectLog } from "../features/connections/lib/connectLog";
import { pickModalFocusTarget } from "../shared/lib/modalFocus";
import { buildPasteConfirmation, type PasteConfirmation } from "../features/terminal/lib/dangerousCommands";
import { readClipboardText, writeClipboardText, type ClipboardDeps } from "../shared/lib/clipboardBridge";
import { DOCKER_SPLIT_RATIO_DEFAULT, paneDirectionClass, paneSplitCssVar, resolveWorkbenchPaneLayout, sanitizePaneDirection, terminalFlexBasis } from "../shared/lib/paneLayout";
import { isCountdownActive } from "../features/recording/lib/recordingCountdown";
import { type SftpTypeFilter } from "../features/sftp/lib/sftpFileFilters";
import { SFTP_BOOKMARK_LABEL_MAX_LENGTH } from "../features/sftp/lib/sftpBookmarks";
import {
  extractMcpSettingsMirror,
  loadMcpSettingsMirror,
  mcpSettingsMirrorEquals,
  persistMcpSettingsMirror,
} from "../features/settings/lib/settingsModel";
import type { SftpCompressMode } from "../shared/lib/preferencesMirror";
import { EnterLineTracker, acceptCommandWithContinuation, canCaptureEnterLine, echoConfirmsLine, extractDesyncedCommand, extractEchoedCommand } from "../features/terminal/lib/terminalEnterCapture";
import { applyLineEditControlChar } from "../features/terminal/lib/terminalLineModel";
import { loadGhostTabAccept, matchesGhostFullAccept, matchesGhostWordAccept, resolveCtrlFRoute, saveGhostTabAccept } from "../features/completion/lib/ghostAcceptKey";
import { readWorkbenchUiMirror, writeWorkbenchUiMirror } from "./lib/workbenchUiMirror";
import { createPersistThrottle } from "./lib/persistThrottle";
import { createPreviewAsyncComponent } from "../shared/lib/asyncPreview";
import { settingsPreferences } from "../features/settings/lib/settingsPreferences";
// 终端交互提示守卫（issue #150 及其反馈扩展）：远端询问输入/要求选择（MFA/
// 验证码/密码待输入、选择菜单、y/n 确认）时，↑/↓ 留给远端交互，不唤起历史
// 面板；键入应答不开建议/ghost 浮层。
import { cursorAbsoluteRow } from "../features/terminal/lib/terminalAnchor";
import { evaluateGhost } from "../features/terminal/lib/terminalGhostSuggest";
import { ARBITER_PRIORITIES, resolveKeyOwners, type ArbiterClaim, type ClaimKey, type KeyOwners, type OverlayId } from "../shared/lib/overlayArbiter";
// AI 助手（Warp AI 对齐批）：宿主 AI 通道封装与失败修复已收口
// composables/useAiAssistant；`#` 命令搜索状态机在 useTerminalAiSearch。
// 结构化补全（FIG wave-1 最终架构）：唯一结构化补全来源 = fig 引擎
// （vendored amazon-q parser + 全量语料，经冻结接缝 FigCompletionSource 注入
// CompletionController）；legacy 补全目录已退役，无命中即
// pass-through（菜单关、Tab 交 shell），不造假候选。
import type { CompletionItem } from "../features/completion/lib/completion/core/types";
import { type SftpNameEncoding } from "../features/sftp/lib/sftpName";
import { batchUploadPercent, runTransfers, type TransferDuplicatePolicy, type TransferTask,
} from "../features/transfers/lib/transferQueue";
import { registerWatch } from "../features/sftp/lib/watchEdits";
import { formatLatency, formatAuthMethodLabel, normalizeConnectionPort, normalizeConnectionText, type KnownAuthMethod } from "../features/connections/lib/connectionInfo";
import { readPluginMode, readPluginShell, resolveWorkbenchId } from "../shared/lib/pluginContext";
import { clampFontSize } from "../features/terminal/lib/terminalZoom";
import { loadLastConnectParams } from "../features/connections/lib/connectLastParams";
import { pluginStore } from "../shared/lib/pluginStore";
import { loadTerminalFontOverride, resolveTerminalFont, type TerminalFontOverride } from "../features/terminal/lib/terminalFont";
import type { DownloadConflictPolicy } from "../features/sftp/lib/downloadPrefs";
import { commandMarkerTooltip, formatCommandDuration, Osc633CommandParser, runningCommandElapsedMs, type Osc633StreamUpdates } from "../features/terminal/lib/terminalCommandMarkers";
import { batchProgressPercent, type BatchProgressState } from "../features/transfers/lib/sftpBatchProgress";
import { describeWorkbenchSessionStatus, type WorkbenchSessionStatus } from "../features/connections/lib/sessionStatus";
import { sanitizeCommandOutput } from "../features/terminal/lib/terminalOutputText";
import { normalizeTerminalInputBytes } from "../features/terminal/lib/terminalInput";
import { type TerminalSyncOutput } from "../features/terminal/lib/terminalModeQueries";
import { installMacWebkitInputFallback } from "../features/terminal/lib/terminalWebkitInput";
import { formatBytes, formatRate } from "../shared/lib/format";
import { resolveAppearance } from "../shared/lib/appearance";
import { isDbxPluginTheme, onHostThemeChange, themeToAppearance } from "../shared/lib/hostTheme";
import { AGENT_MODES } from "../features/agent/lib/agentTerminal";
import {
  highlightFillStyle,
  highlightTextStyle,
  matchesInLine,
  shouldRebuildHighlightRow,
  toAbsoluteRowRange,
} from "../features/terminal/lib/keywordHighlight";
// 动作链接（P1-2，默认关闭）+ 行号/时间戳 gutter（P1-3，默认关闭）。
import { createActionLinkProvider } from "../features/terminal/lib/actionLinksAddon";
import {
  matchActionLinks,
  sanitizeActionLinksSettings,
  type ActionLinkMatch,
  type ActionLinkMatcherToggles,
  type ActionLinksSettings,
} from "../features/terminal/lib/actionLinksMatcher";
// 命中是文本下标，装饰要的是单元格列号：宽字符行（CJK）不换算就会整段左移。
import { lineColumnTable, matchToColumns } from "../features/terminal/lib/terminalLineColumns";
import {
  computeGutterRows,
  getRenderCellHeight,
  sanitizeGutterSettings,
  trimTimestampMap,
  GUTTER_TIMESTAMP_RETENTION_ROWS,
  type GutterRow,
  type GutterSettings,
} from "../features/terminal/lib/terminalGutter";
import { transferPausable } from "../features/transfers/lib/transferResume";
import { auditKindLabel, auditKindOptions, auditOutcomeLabel } from "../features/security/lib/auditLog";
import { useAuditLogViewer } from "../features/security/composables/useAuditLogViewer";
import { useSessionReconnect } from "../features/connections/composables/useSessionReconnect";
import { usePluginEvents } from "./composables/usePluginEvents";
import { useDockMetrics } from "./composables/useDockMetrics";
import { useEditorOpenWith } from "./composables/useEditorOpenWith";
import { useEditorSplitPane } from "./composables/useEditorSplitPane";
import { useAiAssistant } from "./composables/useAiAssistant";
import { useNotices } from "./composables/useNotices";
import { useOnlineSearch } from "./composables/useOnlineSearch";
import { usePromptHints } from "./composables/usePromptHints";
import { useSftpAttributes } from "./composables/useSftpAttributes";
import { useSftpBookmarks } from "./composables/useSftpBookmarks";
import { useSftpRowSelection } from "./composables/useSftpRowSelection";
import { useSerialUpload } from "./composables/useSerialUpload";
import { useSftpTreeWorkspace } from "./composables/useSftpTreeWorkspace";
import { joinRemote, normalizeRemotePath, parentPath, pathFromUri } from "./composables/sftpRemotePaths";
import { useWallpaper } from "./composables/useWallpaper";
import { useWorkbenchPrefs } from "./composables/useWorkbenchPrefs";
import { useWorkbenchPanels } from "./composables/useWorkbenchPanels";
import { useLocalShellPreferences } from "./composables/useLocalShellPreferences";
import { useShellTabs } from "./composables/useShellTabs";
import { useTerminalMenus } from "./composables/useTerminalMenus";
import { useQuickCommandPanel } from "./composables/useQuickCommandPanel";
import { useSftpFolderActions } from "./composables/useSftpFolderActions";
import { useSftpChmod, PERM_COLUMNS, PERM_ROLES, permBit } from "./composables/useSftpChmod";
import { useWorkbenchViewPrefs } from "./composables/useWorkbenchViewPrefs";
import { useWorkbenchSpawnTabs } from "./composables/useWorkbenchSpawnTabs";
import { useWorkbenchToolbar } from "./composables/useWorkbenchToolbar";
import { useTerminalSearch } from "./composables/useTerminalSearch";
import { useTerminalTooltip } from "./composables/useTerminalTooltip";
import { useRdpSession } from "../features/remote-desktop/composables/useRdpSession";
import { useRdpWorkbench } from "../features/remote-desktop/composables/useRdpWorkbench";
import { useVncSession } from "../features/remote-desktop/composables/useVncSession";
import { useConnectCard } from "../features/connections/composables/useConnectCard";
import { useProtocolFrameBuffers } from "../shared/composables/useProtocolFrameBuffers";
import { useUploadChain } from "../features/sftp/composables/useUploadChain";
import { useFilePreview } from "../features/sftp/composables/useFilePreview";
import { useSftpDownload } from "../features/sftp/composables/useSftpDownload";
import { useSftpDelete } from "../features/sftp/composables/useSftpDelete";
import { useSftpListLayout } from "../features/sftp/composables/useSftpListLayout";
import { useSftpSelectionClipboard } from "../features/sftp/composables/useSftpSelectionClipboard";
import { useSftpNavigation } from "../features/sftp/composables/useSftpNavigation";
import OtpPanel from "../features/security/components/OtpPanel.vue";
import ImportWizard from "../features/connections/components/ImportWizard.vue";
import { useTerminalAppearance } from "../features/terminal/composables/useTerminalAppearance";
import { useTerminalBell } from "../features/terminal/composables/useTerminalBell";
import { useTerminalFontZoom } from "../features/terminal/composables/useTerminalFontZoom";
import { useAppearanceCrossSurfaceSync } from "../shared/composables/useAppearanceCrossSurfaceSync";
import { useTerminalMouse } from "../features/terminal/composables/useTerminalMouse";
import { useTransferQueue } from "../features/transfers/composables/useTransferQueue";
import { useCommandSuggestions } from "../features/completion/composables/useCommandSuggestions";
import { useGhostSuggest } from "../features/completion/composables/useGhostSuggest";
import { useQuickSelect } from "../features/terminal/composables/useQuickSelect";
import { useHistoryPanel } from "../features/terminal/composables/useHistoryPanel";
import { useCommandHistory } from "../features/terminal/composables/useCommandHistory";
import { useSftpRename } from "../features/sftp/composables/useSftpRename";
import { useEditorTabs } from "../features/sftp/composables/useEditorTabs";
import EditorPane from "../features/sftp/components/EditorPane.vue";
import TerminalLayoutTree from "../features/terminal/components/TerminalLayoutTree.vue";
import {
  findTerminalPane,
  hasTerminalSplit,
  shouldFollowTabCwd,
} from "../features/terminal/lib/terminalLayout";
import { useSftpCreateDirectory } from "../features/sftp/composables/useSftpCreateDirectory";
import { useConnectionInfo } from "../features/connections/composables/useConnectionInfo";
import { useSftpArchive } from "../features/sftp/composables/useSftpArchive";
import { useSftpPathHistory } from "../features/sftp/composables/useSftpPathHistory";
import { useSymlink } from "../features/sftp/composables/useSymlink";
import { useZmodem } from "../features/terminal/composables/useZmodem";
import { useCommandDialog } from "../features/terminal/composables/useCommandDialog";
import { useExternalEdits } from "../features/sftp/composables/useExternalEdits";
import { useBatchSend } from "../features/terminal/composables/useBatchSend";
import { useMetricsPanel, type ServerMetrics } from "../features/metrics/composables/useMetricsPanel";
import { useTrzsz } from "../features/terminal/composables/useTrzsz";
import { useRecording } from "../features/recording/composables/useRecording";
import { useHighlightRules } from "../features/terminal/composables/useHighlightRules";
import { useQuickCommands } from "../features/quick-commands/composables/useQuickCommands";
import { useAgentTerminalMode } from "../features/agent/composables/useAgentTerminalMode";
import { useAlertTriage } from "../features/security/composables/useAlertTriage";
import { useTransferHistory } from "../features/transfers/composables/useTransferHistory";
import { resolveSftpPaneOpen, resolveDirectoryFollow, shouldLazyLoadSftpListing, sanitizeWorkbenchLayoutPrefs, type SshWorkbenchPaneOrder } from "./lib/workbenchLayout";
import { pickLiveSessionForReattach, pickProtocolSessionForReattach, type SessionSummary } from "../features/connections/lib/sessionRestore";
import { toolbarTintStyle } from "../shared/lib/toolbarTint";
import { createGhostClickGuard } from "../features/completion/lib/ghostClickGuard";
import {
  COLUMN_MIN_WIDTHS,
  COLUMN_WIDTH_MAX,
  DEFAULT_COLUMN_WIDTHS,
  DEFAULT_VISIBLE_COLUMNS,
  NAME_COLUMN_MAX,
  NAME_COLUMN_MIN,
  sanitizeVisibleColumns,
  type SftpColumn,
} from "../features/sftp/lib/sftpEntries";
import {
  folderUploadPercent,
} from "../features/sftp/lib/folderUpload";
import { createCrossSurfaceStorageReader, subscribeStorageEvents } from "../shared/lib/appearanceSync";
import { attachWebglRenderer, syncWebglRenderer, type WebglRecoveryOptions, type WebglRendererLike } from "../features/terminal/lib/terminalWebgl";
import {
  activeProfileId,
  loadTerminalAppearance,
  terminalOptionPatch,
  type TerminalAppearanceState,
} from "../features/terminal/lib/terminalAppearance";
import {
  terminalBehaviorOptionPatch,
  transformPasteText,
} from "../features/terminal/lib/terminalBehavior";
import {
  keyComboFromEvent,
  matchTerminalHotkey,
} from "../features/terminal/lib/terminalHotkeys";
import { bridgeBinaryBytes } from "../../../shared/frontend/binaryEvent";
import {
  applyFileTreeChildren,
  createFileTreeRoot,
  findFileTreeNode,
  flattenVisibleTreeRows,
  markCollapsedTreeNodesStale,
  type SftpFileTreeNode,
} from "../features/sftp/lib/sftpFileTree";
import {
  filterSelectableSftpEntries,
} from "../features/sftp/lib/sftpContextMenu";
import { type SftpSanitizedEntry } from "../features/sftp/lib/sftpEntries";
import { mergeSftpDetail, type SftpDetailInfo } from "../features/sftp/lib/sftpDetails";
import { workbenchMessage } from "../shared/lib/i18n";
import { randomUUID } from "../shared/lib/uuid";
// 预览面板按需加载（含 CodeMirror + language-data，数百 KB）：定义异步组件
// 使模块只在首次打开预览弹窗时求值，工作台启动不再解析编辑器/语言包。
// loading/error 兜底：动态 import 失败时渲染说明 + 重试，不再留空白正文
// （文案闭包按当前 locale 现取；t 在 setup 后段定义，渲染时已初始化）。
const previewAsyncLabels = () => ({
  loading: t("loading"),
  failed: t("previewDialog.loadFailed"),
  retry: t("previewDialog.retry"),
});
const TextPreview = createPreviewAsyncComponent(
  () => import("../features/sftp/components/TextPreview.vue"),
  previewAsyncLabels,
);
const JsonPreviewPanel = createPreviewAsyncComponent(
  () => import("../features/sftp/components/JsonPreviewPanel.vue"),
  previewAsyncLabels,
);
import TerminalSearchPanel from "../features/terminal/components/TerminalSearchPanel.vue";
import TerminalQuickSelectPanel from "../features/terminal/components/TerminalQuickSelectPanel.vue";
import TerminalHistoryPanel from "../features/terminal/components/TerminalHistoryPanel.vue";
import TerminalPromptHints from "../features/terminal/components/TerminalPromptHints.vue";
import {
  commandHistoryHotkeyPassthrough,
} from "../features/terminal/lib/historyPanel";
import TerminalGutter from "../features/terminal/components/TerminalGutter.vue";
import CommandSuggestions from "../features/completion/components/CommandSuggestions.vue";
import CompletionMenu from "../features/completion/components/CompletionMenu.vue";
import ConnectingCard from "../features/connections/components/ConnectingCard.vue";
import ServerMetricsPanel, { type ServerMetricsView, type MetricsProcessSortKey } from "../features/metrics/components/ServerMetricsPanel.vue";
import FolderPickerDialog from "../features/sftp/components/FolderPickerDialog.vue";
import DockerPanel from "../features/docker/components/DockerPanel.vue";
import DockerWhaleLogo from "../features/docker/components/DockerWhaleLogo.vue";
import DockMetricsStrip from "./components/DockMetricsStrip.vue";
import ConnectionInfoPopover from "./components/ConnectionInfoPopover.vue";
import BatchBar from "./components/BatchBar.vue";
import TransferPopover from "./components/TransferPopover.vue";
import WorkbenchSftpDialogs from "./components/WorkbenchSftpDialogs.vue";
import WorkbenchSessionDialogs from "./components/WorkbenchSessionDialogs.vue";
import SftpWorkspacePanel from "./components/SftpWorkspacePanel.vue";
import { Switch } from "../shared/ui/switch/index";
import type { EditorConfig } from "../features/sftp/lib/editorRules";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../shared/ui/select/index";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "../shared/ui/context-menu/index";
import { Popover, PopoverAnchor, PopoverContent } from "../shared/ui/popover/index";
import { Dialog, DialogContent, DialogTitle } from "../shared/ui/dialog/index";
import SettingsDialog from "../features/settings/components/SettingsDialog.vue";
import TerminalContextMenu from "../features/terminal/components/TerminalContextMenu.vue";
import PortForwardDialog from "../features/forwarding/components/PortForwardDialog.vue";
import TelnetConnectDialog, { type TelnetConnectOptions } from "../features/telnet/components/TelnetConnectDialog.vue";
import SerialConnectDialog, { type SerialConnectOptions } from "../features/serial/components/SerialConnectDialog.vue";
// 串口文件上传（NyaTerm 对齐 P0-3）：弹窗 + overlay 状态机在 lib/serialUpload，
// 状态/送数/取消在 composables/useSerialUpload；模板复位仍需初始态构造器。
import SerialUploadDialog from "../features/serial/components/SerialUploadDialog.vue";
import { initialSerialUploadState } from "../features/serial/lib/serialUpload";
// VNC 会话（nyaterm-parity P2 2d）：连接表单 + 画布表面，帧通道在
// handleBinary 的 vnc/frame/{id} 分支接入。
import VncConnectDialog, { type VncConnectOptions } from "../features/remote-desktop/components/VncConnectDialog.vue";
import VncSurface from "../features/remote-desktop/components/VncSurface.vue";
// RDP 会话（nyaterm-parity P3-4）：画布与 VNC 同构（同一 44 字节 patch 头，
// 解码复用 vncFrame），输入走扫描码/unicode 双通道，证书确认走
// connection/challenge kind=rdp-certificate 分支。
import RdpConnectDialog, { type RdpConnectOptions } from "../features/remote-desktop/components/RdpConnectDialog.vue";
import RdpSurface from "../features/remote-desktop/components/RdpSurface.vue";
import {
  initialRdpSessionState,
  rdpErrorKindKey,
  type RdpSessionStateView,
} from "../features/remote-desktop/lib/rdpFrame";
import { ToastAction, ToastClose, ToastProvider, ToastRoot, ToastViewport } from "../shared/ui/toast/index";

interface SessionInfo {
  sessionId: string;
  connectionId: string;
  workbenchId?: string;
  connected: boolean;
  sequence: number;
  chunkSize: number;
  directoryTrackingSupported?: boolean;
  replay?: ReplayResult;
}

interface ReplayResult {
  frameCount: number;
  firstAvailableSequence: number;
  tailSequence: number;
  complete: boolean;
}

interface SftpEntry {
  name: string;
  uri: string;
  kind: "file" | "directory" | "symlink" | "other";
  size?: number;
  modifiedAt?: number;
  permissions?: string;
  contentType?: string;
  /** 属主用户/属组（includeOwner 时由 sidecar 返回；缺失显示 "-"）。 */
  owner?: string;
  group?: string;
  /** M14-B：显示名不可忠实还原（wire 名含 U+FFFD）；传输仍走 uri。 */
  lossy?: boolean;
}

// 服务器内复制/剪切/粘贴的剪贴板：仅当前连接内有效。
interface SftpClipboard {
  mode: "copy" | "cut";
  paths: string[];
  connectionId: string;
}

interface HostKeyPrompt {
  challengeId: string;
  operationId: string;
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
}


type RightPanelView = "sftp" | "metrics" | "docker";

interface WorkbenchState {
  sessionId?: string;
  terminalSequence?: number;
  sftpPath?: string;
  followDirectory?: boolean;
  sudoMode?: boolean;
  splitRatio?: number;
  paneOrder?: SshWorkbenchPaneOrder;
  sftpPaneOpen?: boolean;
  visibleColumns?: SftpColumn[];
  sftpColumnWidths?: Partial<Record<SftpColumn, number>>;
  sftpNameWidth?: number | null;
  /** 列配置版本标记：六列默认（issue #35）上线后的一次性迁移，老偏好重置为全开。 */
  columnsV2?: boolean;
  /** RDP 会话内用户选择的桌面尺寸（P0-2）：恢复 tab 时优先于连接表单 rdp_resolution。 */
  rdpWidth?: number;
  rdpHeight?: number;
  /** SFTP 隐藏文件显隐（issue #178）：此前完全未持久化。 */
  sftpShowHidden?: boolean;
  /** SFTP 列表排序规则（issue #178）：此前完全未持久化。 */
  sftpSort?: { column: SftpSortColumn; direction: "asc" | "desc" };
}

interface RuntimeEndpoint {
  host?: string;
  port?: number;
}

interface ConnectionSummary {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  color?: string;
  readOnly?: boolean;
  /** 宿主连接表单的 protocol 字段（缺省 ssh）：非 SSH 连接在 openSession 里路由到各自会话。 */
  protocol?: "ssh" | "telnet" | "vnc" | "serial" | "rdp";
  /** 串口连接配置（M32-B，manifest serial_* 字段；缺失时回落连接表单）。 */
  serialPort?: string;
  serialBaud?: number;
  serialDataBits?: "7" | "8";
  serialParity?: "none" | "even" | "odd";
  serialStopBits?: "1" | "2";
  serialBackspace?: "del" | "ctrl_h";
  /** RDP 连接配置（M32-B，manifest rdp_* 字段）。 */
  rdpDomain?: string;
  rdpWidth?: number;
  rdpHeight?: number;
  rdpCertificatePolicy?: "prompt" | "strict" | "accept-temporarily";
  rdpClipboard?: boolean;
}

interface DownloadInfo {
  taskId: string;
  fileName: string;
  size: number;
  chunkSize: number;
  // 断点续传：start 带 offset 时回显的恢复起点。
  resumeOffset?: number;
  // 目录下载（tree/start）扩展：扫描得到的整树规模。
  fileCount?: number;
  dirCount?: number;
  skippedCount?: number;
}

interface ExecResult {
  output: string;
  exitCode: number;
}

interface DiskUsage {
  filesystem: string;
  mount: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  percentUsed: number;
}

// 列类型移到 lib/sftpEntries（issue #34：owner/group 属主/属组列，默认关）。
type SftpSortColumn = "name" | "size" | "modified";

// 粘贴防护：内容含换行或达到该字符数时先确认（对齐 tiny-rdm TerminalPane 阈值）。
const PASTE_CONFIRM_CHAR_THRESHOLD = 200;
// SFTP 路径历史：每连接最多保留 10 条，存 pluginStore（宿主 host.storage；对齐 tiny-rdm pathHistory）。
// Upper bound for out-of-order terminal frames held while waiting for the
// missing sequence; the replay path re-delivers anything dropped beyond it.
const TERMINAL_PENDING_FRAME_LIMIT = 1024;
// 命令历史 / 终端字号：pluginStore 持久化（敏感命令不入持久层；快速命令已迁 sidecar，见 QUICK_COMMANDS_KEY）。
const COMMAND_HISTORY_KEY = "ssh-command-history";
// 执行时间并行映射（Warp 式 history 面板的右侧相对时间）：command → 最近一次
// 执行时刻。不改 commandHistory（string[]）的形状——suggestion/ghost/命令弹窗
// 多处消费零波及；时间只有面板用。
const COMMAND_HISTORY_TIMES_KEY = "ssh-command-history-times";
const COMMAND_HISTORY_META_KEY = "ssh-command-history-meta";
// 终端字号/字体族键移入 lib/terminalFont.ts（issue #31 字体单独设置）统一管理。
// 跨重启的布局/视图全局偏好（issue #178）：单键 JSON，记录最近一次的
// paneOrder / splitRatio / SFTP 面板开合 / 列显隐与列宽 / 显示隐藏文件 / 排序。
// writeWorkbenchState 写穿（persistState 防抖统一触发），restoreUiState 启动
// 水合为新工作台初始值；workbenchState 会话恢复字段存在时仍优先覆盖。
const WORKBENCH_LAYOUT_KEY = "ssh-workbench-layout";
// 是否默认下载到「下载保存目录」（默认开）；关闭则每次下载打开目录选择窗口。
// 文件已存在时的处理策略：rename（自动重命名，默认）/ ask（询问我）/ overwrite（覆盖）。
// 上传并发（P1-5，1..10，默认 3）与重复目标策略（rename 默认）：sidecar
// 压缩传输（M33，gzip 混合方案）：策略三态（auto=智能综合判断含弱 CPU
// 门槛，默认 / on=始终尝试 / off=关闭）+ 生效阈值 MiB（默认 64，0=不限
// 下限）；sidecar preferences 同步，键名与后端白名单一致。
// 命令输入建议（P1-1）：开关 + 查询长度上下限；同一偏好链路持久化。
// 下载偏好的内存权威态：setup 早期（downloadUseDefaultDraft 初始化）就会被读，
// 必须声明在所有读取点之前（存储语义见下方 loadDownloadDir 一带的注释）。
const downloadDirState = ref("");
const downloadUseDefaultState = ref(true);
const downloadConflictState = ref<DownloadConflictPolicy>("rename");
// 上传并发/重复策略与命令建议的内存权威态（hydratePrefs 时被 sidecar 值覆盖）。
// M14-B 三键：会话级并发深度（1-8，默认 3）、老旧服务器兼容模式（默认关）、
// 文件名显示编码（auto/latin-1，默认 auto）。权威态在此，sidecar preferences 同步。
// 下载限速权威态（KiB/s，0=不限速）；sidecar preferences 同步。
const sftpCompatModeState = ref(false);
const sftpNameEncodingState = ref<SftpNameEncoding>("auto");
// 压缩传输（M33）：策略默认 auto、阈值默认 64 MiB（0=不限下限，上限 65536）。
const transferCompressModeState = ref<SftpCompressMode>("auto");
const transferCompressThresholdState = ref(64);
const suggestionsEnabledState = ref(true);
const suggestionMinCharsState = ref(1);
const suggestionMaxCharsState = ref(64);
// ghost 接受键（Warp 多键模型）：→ / Ctrl+F / Ctrl+E 整段、Ctrl+→ 逐词恒可用
// （lib/ghostAcceptKey.ts 匹配矩阵）；唯一可配置项是「Tab 接受建议」开关
// （Warp Tab key behavior），pluginStore 权威 + 内存态供按键分支与键帽回显。
// issue #108a 起 Ctrl+F 只在 ghost 建议可见待接受时作接受键，无 ghost 归终端
// 搜索（iTerm2 同位，resolveCtrlFRoute 判定）；→ / Ctrl+E 行为不变。
const ghostTabAccept = ref(loadGhostTabAccept());
// 键位胶囊（Warp accept-autosuggestion-hint 同位）的下拉菜单与展示标签。
const ghostKeycapMenuOpen = ref(false);
const ghostAcceptKeyLabel = computed(() => (ghostTabAccept.value ? "Tab" : "→"));
function setGhostTabAccept(value: boolean) {
  ghostTabAccept.value = value;
  saveGhostTabAccept(value);
}

// 通知/错误横幅状态收口在 composables/useNotices；t 为后段初始化的惰性引用。
const {
  terminalError,
  sftpError,
  sftpErrorRetry,
  sftpErrorOpen,
  sftpErrorKey,
  notice,
  noticeActions,
  noticeOpen,
  noticeKey,
  showNotice,
  onNoticeOpenChange,
  onSftpErrorOpenChange,
  showError,
} = useNotices({ t: (key) => workbenchMessage(locale.value, key) });

// 主终端搜索面板状态/动作收口在 composables/useTerminalSearch；addon 与菜单在
// 后段初始化，getter/回调延迟读取。
const {
  searchOpen,
  searchSeedQuery,
  searchSeedOptions,
  searchMatchState,
  searchResultIndex,
  searchResultCount,
  openTerminalSearch,
  closeTerminalSearch,
  clearTerminalSearch,
  resetSearchResults,
  handleSearchResults,
  runTerminalSearch,
} = useTerminalSearch({
  terminal: () => terminal,
  searchAddon: () => searchAddon,
  closeMenu: () => {
    terminalMenuOpen.value = false;
  },
});

// 存储迁移批 3：传输/下载/建议偏好消毒器已下沉 lib/preferencesMirror
// （镜像 schema 同处、可单测），App 统一从那里导入。
// —— AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）——
// 设置权威值/修复条/执行期输出采集/推荐位收口在 composables/useAiAssistant；
// 终端与后声明偏好经 getter 惰性读取。`#` 命令搜索状态机留在 App（模板直接
// 消费其解构态）。
const {
  aiSettingsState,
  aiBridgeReady,
  aiFixBar,
  aiBridgeOk,
  updateAiSettings,
  resetAiFixConsent,
  aiContextExtras,
  aiFixFromBar,
  aiAssistFromTerminal,
  aiFixBarStyle,
  maybeOfferAiFix,
  captureOutputChunk,
  beginAiCommand,
  resetAiFixForSession,
  clearFixRecommendation,
} = useAiAssistant({
  t: (key, values) => t(key, values),
  showNotice,
  getLocale: () => locale.value,
  getConnectionId: () => connectionId.value,
  getTerminalCwd: () => terminalCwd.value,
  isLocalMode: () => isLocalMode.value,
  getLocalShellPref: () => localShellPref.value,
  getLocalShells: () => localShells.value,
  isApplePlatform: () => applePlatform,
  sniffShell: () => sniffTerminalShell(),
  terminal: () => terminal,
  commandMarker: () => commandMarker,
  suggestionViewportHeight: () => suggestionViewport.value.height,
  readSuggestionAnchor: () => readTerminalSuggestionAnchor(),
  replaceLine: (command) => {
    replaceTerminalLineWith(command, false);
    terminal?.focus();
  },
});
// `#` 命令搜索状态机/等待冻结/锚点/直连生成与分屏窗格共用同一 composable。
const aiSearch = useTerminalAiSearch({
  terminal: () => terminal,
  getHost: () => terminalHost.value,
  t: (key, values) => t(key, values),
  getLine: () => pendingTerminalInput,
  isEnabled: () => aiBridgeOk() && aiSettingsState.value.search,
  gates: () => ({
    alternateActive: terminal?.buffer.active.type === "alternate",
    commandRunning: commandRunning.value,
    transferBusy: terminalTransferBusy.value,
    suggestionOpen: suggestionOpen.value,
    completionOpen: completionOpen.value,
    historyPanelOpen: historyPanelOpen.value,
    quickSelectOpen: quickSelectOpen.value,
    searchOpen: searchOpen.value,
  }),
  replaceLine: (command) => {
    replaceTerminalLineWith(command, false);
    terminal?.focus();
  },
  notify: showNotice,
  context: aiContextExtras,
  onActivate: dismissPromptHints,
});
const {
  state: aiSearchState,
  pending: aiSearchPending,
  pendingQuery: aiSearchPendingQuery,
  anchor: aiSearchAnchor,
  barEl: aiSearchBarEl,
  barStyle: aiSearchBarStyle,
} = aiSearch;

// Apple 平台判定（Cmd 为主修饰键）：既有的全选语义与新增的快捷键默认键位都要用，
// 因此在此单点声明，供后面的偏好初始值与终端选项复用。
const applePlatform = isApplePlatform();
// decoration 引擎护栏：全局在档 decoration 上限（超限停止本帧注册）。
const HIGHLIGHT_DECORATION_LIMIT = 400;
// rAF 节流目标：≤30fps（约 33ms 一帧）。
const HIGHLIGHT_SCAN_MIN_INTERVAL_MS = 33;

const terminalHost = ref<HTMLElement>();
const sftpPane = ref<HTMLElement>();
// 目录跟随的"目标目录置顶"需要命令式滚动到树内指定路径。
const sftpTreePane = ref<{ scrollPathToTop(path: string): Promise<boolean> } | null>(null);
/** SftpWorkspacePanel 把内部树实例回填到上面的 ref，保持目录跟随即有语义。 */
function bindSftpWorkspaceTreePane(api: { scrollPathToTop(path: string): Promise<boolean> } | null) {
  sftpTreePane.value = api;
}
/** 路径输入框随工具栏移入子组件：回填 composable 的 ref，进入编辑态仍能聚焦。 */
function bindSftpPathBarInput(el: HTMLInputElement | null) {
  pathBarInputEl.value = el;
}
const paneContainer = ref<HTMLElement>();
const zmodemInput = ref<HTMLInputElement>();
const hostContext = ref<Record<string, unknown>>({});
let transportReuseState = createSessionTransportReuseState({});
// Bottom dock panel surface (surface=panel, host §8.3): hide the workbench identity block so the panel
// and focus the terminal itself; multi-open/shell switching goes through the panel "+" menu (bridge openWorkbench opens another panel).
// Declared early: the batch bar / sftp pane initializers below must know the surface at setup time.
// 宿主表面词表（开放集合，未知回退）："tab" 主工作台多标签 / "dock" 底部栏
// （2026-09-30 起，旧值 "panel" 兼容期保留）/ "sidebar-left|right" / "window" 预留。
const panelSurface = computed(() => hostContext.value.surface === "dock" || hostContext.value.surface === "panel");
// 宿主未下发 appearance 前的兜底：DBX `.dark` 规范令牌。
const appearance = ref(resolveAppearance());
const terminalState = ref<"connecting" | "connected" | "disconnected" | "error">("connecting");
// 连接卡片：用户取消（在途 open 无法中止，仅切换 UI 态并在 promise 落地后回收
// 孤儿会话）、Show logs 展开态与连接尝试日志（环形 200 条，跨尝试保留历史）。
const connectCancelled = ref(false);
// 连接成功后立即揭示终端；success 标记只用于非阻塞的视觉状态。
const connectSucceeded = ref(false);
// 手动重连撞上 "Connection is not active"（sidecar 注册表丢凭据）：卡片保持
// connecting 态并提示用户在 DBX 侧边栏重开连接，插件在有界窗口内自动轮等待宿主重放凭据。
const inactiveWaiting = ref(false);
const connectLogsOpen = ref(false);
const connectLog = createConnectLog();
const session = ref<SessionInfo>();
const currentPath = ref("/");
const entries = ref<SftpEntry[]>([]);
const selectedPath = ref("");
const loadingFiles = ref(false);
const hostKeyPrompt = ref<HostKeyPrompt>();
const rememberHostKey = ref(true);
const splitRatio = ref(58);
// Docker 底部分屏的独立比例：默认终端 67% / 容器 33%，每次打开 Docker 重置。
const dockerSplitRatio = ref(DOCKER_SPLIT_RATIO_DEFAULT);
const paneOrder = ref<SshWorkbenchPaneOrder>("terminal-left");
// 工作台视图/终端行为偏好（布局轴向、SFTP 默认/自动连接、目录跟随、终端行为
// 与快捷键）：收口在 composables/useWorkbenchViewPrefs。
const {
  paneDirection,
  sftpPaneDefaultOpen,
  sftpAutoConnect,
  terminalBehavior,
  termSelectCopy,
  terminalHotkeys,
  followDirectory,
  toggleSftpPaneDefaultOpen,
  loadSftpPaneDefaultOpen,
  toggleSftpAutoConnect,
  loadDirectoryFollowPref,
  persistDirectoryFollowPref,
  setPaneDirection,
  updateTerminalBehavior,
  updateTerminalHotkeys,
} = useWorkbenchViewPrefs({
  applePlatform,
  terminal: () => terminal,
  showNotice: (message) => showNotice(message),
  t: (key, values) => t(key, values),
  scheduleFit: () => scheduleFit(),
});
// 右侧栏可见性：SFTP / 指标 / 容器三视图互斥，sftpPaneOpen 表示右侧栏
// 整体开合（沿用既有 workbenchState 持久化），rightPanelView 决定当前内容。
// 指标/容器视图不跨重启恢复，避免启动即触发轮询。
const sftpPaneOpen = ref(loadSftpPaneDefaultOpen());
const rightPanelView = ref<RightPanelView>("sftp");
// 容器视图：关闭/切换后即卸载停轮询。Docker 打开期间分栏固定为上下。
const dockerPanelOpen = computed(() => sftpPaneOpen.value && rightPanelView.value === "docker");
const sftpViewActive = computed(() => rightPanelView.value === "sftp");
// 终端拖放的“面板抢落点”门控：只有 SFTP 树真的可见时才把拖入交给面板；
// 指标/容器视图不应阻止终端接收拖拽。
const sftpViewVisible = computed(() => sftpPaneOpen.value && sftpViewActive.value);
// 本会话是否已成功加载过 SFTP 目录：自动连接关闭时，手动打开面板的瞬间
// 据此惰性补载（每会话一次，断开重置）。
const sftpListingLoaded = ref(false);
// sftp/home 探测结果：quick tab 置顶展示（获取失败时该项隐藏）。
const sftpHomePath = ref("");
// 沙箱宿主读不到系统剪贴板：右键粘贴的降级链靠这份插件视图内的复制副本。
const terminalCopyCache = createTerminalCopyCache();
// 终端 shell 最近一次上报的 cwd（OSC 7 / OSC 633 Cwd，无论跟随开关是否打开
// 都记录）：终端拖拽上传的「当前目录」落点解析靠它，避免误用 SFTP 面板的
// 浏览目录（初始值 "/"，拼根路径会被服务器以权限拒绝）。
const terminalCwd = ref("");
const directoryTrackingSupported = ref<boolean | undefined>();
const visibleColumns = ref<SftpColumn[]>([...DEFAULT_VISIBLE_COLUMNS]);
/** 每列当前宽度（px）。 */
const sftpColumnWidths = reactive<Record<SftpColumn, number>>({ ...DEFAULT_COLUMN_WIDTHS });
/** 名称列宽度（px）；null = 未拖过，弹性填充剩余空间。 */
const sftpNameWidth = ref<number | null>(null);
/** 正在拖拽的列（含 name）；null 表示没在拖。 */
const sort = ref<{ column: SftpSortColumn; direction: "asc" | "desc" }>({ column: "name", direction: "asc" });
// syncPrefs 实体在下方 useWorkbenchPrefs 解构后才接线；useTransferQueue 在
// setup 早期按引用捕获它，用提升的薄包装保持既有接线时机（调用都在 setup 后）。
let syncPrefsImpl: (() => Promise<void>) | undefined;
function syncPrefs() {
  return syncPrefsImpl?.() ?? Promise.resolve();
}
// 传输队列核心：任务账本/进度/暂停取消/偏好收口在 composables/useTransferQueue。
const {
  TRANSFER_CONCURRENCY_KEY,
  TRANSFER_MAX_ACTIVE_KEY,
  TRANSFER_DOWNLOAD_LIMIT_KEY,
  TRANSFER_DUPLICATE_KEY,
  transferTasks,
  pausedTaskIds,
  transferPanelOpen,
  transferSpeeds,
  transferHistoryMenu,
  transferConcurrencyState,
  transferMaxActiveState,
  transferDownloadLimitState,
  transferDuplicateState,
  cancelledTransferTasks,
  uploadAckWaiters,
  downloadChunkWaiters,
  updateTransfer,
  transferList,
  activeTransfers,
  restoreTransfers,
  reconcileActiveTransfers,
  refreshTransferPanel,
  loadTransferConcurrency,
  persistTransferConcurrency,
  loadTransferDuplicatePolicy,
  persistTransferDuplicatePolicy,
  loadTransferMaxActive,
  persistTransferMaxActive,
  loadTransferDownloadLimit,
  persistTransferDownloadLimit,
  uploadSource,
  waitWhilePaused,
  toggleTransferPause,
  cancelTransfer,
  transferPercent,
  transferBarValue,
  transferShownBytes,
  transferBytesTitle,
} = useTransferQueue({
  t: (key, values) => t(key, values),
  showError,
  session, currentPath, panelSurface,
  resolveUploadDuplicateName: (name, targetDir) => resolveUploadDuplicateName(name, targetDir),
  joinRemote, writeU64, syncPrefs,
  refreshTransferHistory: () => refreshTransferHistory(),
  refreshResumableUploads: () => refreshResumableUploads(),
  // UploadSudo：sudo 模式 + 可写连接时上传走 sudo 车道（sudo/upload/*）。
  // 只读连接不切——sudo 族对只读 fail closed，普通下载/上传可读目标不受影响。
  sudoUpload: () => sudoMode.value && canWrite.value,
});
const columnsOpen = ref(false);
const sudoMode = ref(false);

const deleteTarget = ref<SftpEntry>();
const deleteSubmitting = ref(false);
const dragActive = ref(false);
// 宿主桥接管 OS drop 时没有目标路径：SftpTreePane 把当前悬停的树行目录
// 上抛到这里，作为 host onDrop 的上传落点。
const sftpDragDropTarget = ref("");
// Terminal-local drag overlay: true only while files are dragged over the
// terminal pane and the drop can actually be accepted (writable session).
const terminalDragActive = ref(false);
// 窗口级兜底（issue #197）：拖拽离开 webview 窗口或被 Esc 取消时，最后一次
// dragleave 的 relatedTarget 为空——无论高亮来自哪条链路（面板 DOM 事件或宿主
// 桥 onDragState）都复位，SFTP 面板不再停留在「上传」悬浮层。子元素间移动的
// leave relatedTarget 非空，不会误触发。
function resetDragHighlightOnWindowLeave(event: DragEvent) {
  if (event.relatedTarget) return;
  dragActive.value = false;
  terminalDragActive.value = false;
}
// 右键菜单/工具条弹层的受控 open 与负载、打开动作收口在
// composables/useTerminalMenus；「关闭全部兄弟弹层」的族清单仍在下方
// closeToolbarPopovers（结构守卫按该函数体扫描收口覆盖）。后续才声明的
// ref（快速命令/会话/连接信息/RDP/配色快切/批量）一律经 getter/setter 惰性接入。
const {
  terminalMenuOpen,
  fileMenu,
  fileMenuProtected,
  menuSelectionSnapshot,
  showTerminalMenu,
  showFileMenu,
  showTransferHistoryMenu,
  onDocumentClickCloseMenus,
  copySelectedPaths,
  toggleQuickMenu,
  toggleSessionMenu,
  toggleConnectionInfo,
  toggleColumnsMenu,
  toggleRdpResolutionMenu,
  toggleRdpPasteMenu,
  togglePanelPaletteMenu,
  toggleBatchQuickPopover,
  toggleTransferPanel,
} = useTerminalMenus<SftpEntry>({
  terminal: () => terminal,
  terminalBehavior: () => terminalBehavior.value,
  isRdp: () => isRdpMode.value || isRdpConnection.value,
  pasteTerminal: () => { void pasteTerminal(); },
  copyTerminalSelection: () => { void copyTerminalSelection(); },
  transferHistoryMenu,
  closeToolbarPopovers: () => closeToolbarPopovers(),
  currentPath,
  selectedPath,
  getSelectedUris: () => selectedUris.value,
  setSelectedUris: (uris) => { selectedUris.value = uris; },
  setLastClickedUri: (uri) => { lastClickedUri.value = uri; },
  getSelectedEntryUris: () => selectedEntries.value.map((item) => item.uri),
  isLoadedDirectoryUri: (uri) => isLoadedDirectoryUri(uri),
  isProtectedDirectory: (entry) => isProtectedDirectory(entry),
  copyTextToClipboard: (value, noticeKey, values) => copyTextToClipboard(value, noticeKey, values),
  quickMenu: {
    isOpen: () => quickMenuOpen.value,
    setOpen: (open) => { quickMenuOpen.value = open; },
  },
  resetQuickMenu: () => {
    quickSearch.value = "";
    quickExpandedId.value = null;
  },
  sessionMenu: {
    isOpen: () => sessionMenuOpen.value,
    setOpen: (open) => { sessionMenuOpen.value = open; },
  },
  closeSessionShellViews: () => {
    sessionMenuShellOpen.value = false;
    sessionMenuShellPickOpen.value = false;
  },
  connectionInfo: {
    isOpen: () => connectionInfoOpen.value,
    setOpen: (open) => { connectionInfoOpen.value = open; },
  },
  refreshConnectionInfo: () => {
    void measureLatency();
    void refreshConnectionAuthMethod();
    // 发行版徽标数据源是 metrics 快照：未拉过时补拉一次（一次 exec，约 0.4s），
    // 否则从未开过指标浮层的会话在连接信息里永远看不到徽标。
    if (!metrics.value) void refreshMetrics();
  },
  columnsOpen,
  rdpResolution: {
    isOpen: () => rdpResolutionOpen.value,
    setOpen: (open) => { rdpResolutionOpen.value = open; },
  },
  rdpPaste: {
    isOpen: () => rdpPasteOpen.value,
    setOpen: (open) => { rdpPasteOpen.value = open; },
  },
  panelPalette: {
    isOpen: () => panelPaletteOpen.value,
    setOpen: (open) => { panelPaletteOpen.value = open; },
  },
  batchQuick: {
    isOpen: () => batchQuickOpen.value,
    setOpen: (open) => { batchQuickOpen.value = open; },
  },
  transferPanelOpen,
  closeMetrics: () => closeMetrics(),
});
// 文件夹行右键菜单里的新建/上传落点：目录右键命中的路径，操作完成后清空。
const mkdirTargetDir = ref<string>();
const newFileTargetDir = ref<string>();
const commandRunning = ref(false);
// 命令历史（按连接作用域分桶）：采集/持久化/远端 shell 历史导入收口在
// composables/useCommandHistory；watch(historyScope) 仍在 setup 尾部（见下）。
const {
  historyScope,
  commandHistoryLimit,
  applyCommandHistoryLimit,
  commandHistory,
  commandHistoryTimes,
  commandHistoryMeta,
  commandHistoryIndex,
  commandHistoryBackup,
  persistCommandHistory,
  persistCommandHistoryTimes,
  pushTerminalCommandHistory,
  recordCommandExecutionMeta,
  loadRemoteShellHistory,
} = useCommandHistory({
  getConnectionId: () => connectionId.value,
  getSerialSessionId: () => serialSession.value?.sessionId,
});
// 快速命令：用户自定义片段（≤20 条），全局存储在插件数据目录（sidecar），
// 所有连接/工作台共享；状态与 PTY 写入语义收口在 composables/useQuickCommands。
const {
  quickCommands,
  quickSaving,
  quickImportBusy,
  hydrateQuickCommands,
  deleteQuickCommand,
  sendQuickCommand,
  pasteQuickCommand,
  sendSudoRefresh,
  saveQuickCommand,
  renameQuickCommandGroupById,
  dissolveQuickCommandGroupById,
  importQuickCommands,
} = useQuickCommands({
  t: (key, values) => t(key, values),
  showNotice,
  showError,
  session,
  commandRunning,
  terminalTransferBusy: () => terminalTransferBusy.value,
  terminal: () => terminal,
  trackPendingInput,
  sendTerminalBytes,
  // 惰性求值：hideGhostSuggestion 由下方 useGhostSuggest 解构（issue #196，
  // 快捷指令程序化写入前消除行内 ghost，防残影重锚到底部光标行）。
  hideGhostSuggestion: () => hideGhostSuggestion(),
});
// 快速命令弹层视图态（搜索/展开/分组折叠）：收口在 composables/useQuickCommandPanel。
const {
  quickMenuOpen,
  quickSearch,
  quickExpandedId,
  filteredQuickCommands,
  groupedQuickCommands,
  quickHasRealGroups,
  quickCollapsedGroups,
  quickCommandGroupSummary,
  toggleQuickGroupCollapse,
  quickGroupName,
  toggleQuickExpand,
} = useQuickCommandPanel({
  quickCommands,
  t: (key, values) => t(key, values),
});
// 命令建议 + 结构化补全（FIG wave-1）：浮层/引擎/调度中枢收口在
// composables/useCommandSuggestions。
const {
  suggestionOpen,
  suggestionItems,
  suggestionActiveIndex,
  suggestionAnchor,
  suggestionQuery,
  suggestionViewport,
  completionOpen,
  completionItems,
  completionActiveIndex,
  completionAnchor,
  completionLoading,
  closeSuggestions,
  handleSuggestionKey,
  handleCompletionKey,
  refreshSuggestionsAfterInput,
  refreshCompletionMenu,
  openCompletionsManually,
  sniffTerminalShell,
  replaceTerminalLineWith,
  fillSuggestion,
  acceptCompletionRow,
  syncSuggestionAnchorsOnSettle,
  readTerminalCellFrame,
  readTerminalSuggestionAnchor,
  completionController,
  lastTerminalCommand,
  resetSuggestionsForSession,
  ignoreSuggestion,
} = useCommandSuggestions({
  terminal: () => terminal,
  getTerminalHost: () => terminalHost.value,
  sendTerminalBytes,
  getPendingTerminalInput: () => pendingTerminalInput,
  setPendingTerminalInput: (value) => {
    pendingTerminalInput = value;
    // 同 trackPendingInput：整行替换（回填/接受）也改变行空态。
    promptHintsLineEmpty.value = value === "";
  },
  commandRunning,
  isTerminalTransferBusy: () => terminalTransferBusy.value,
  terminalCwd, commandHistory, quickCommands,
  suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState,
  session,
  getSessionId: () => session.value?.sessionId ?? localSession.value?.sessionId ?? serialSession.value?.sessionId ?? "",
  getLocalSessionId: () => localSession.value?.sessionId,
  persistCommandHistory,
  isHistoryPanelOpen: () => historyPanelOpen.value,
  // 历史面板接线在下方 useHistoryPanel：惰性转发，规避声明顺序。
  updateHistoryPanelFilter: (syncLine?: boolean) => updateHistoryPanelFilter(syncLine),
  pushTerminalCommandHistory,
  syncHistoryPanelAnchor: () => { historyPanelAnchor.value = readTerminalSuggestionAnchor(); },
  // 数据分工（#138 批 2）：ghost 开启时前缀延伸命中让位行内 ghost，模糊
  // 浮层不弹。ghostEnabled 在下方 useGhostSuggest 才创建，惰性求值闭包
  // 仅在 onData 期调用，无初始化顺序问题。
  inlineGhostActive: () => ghostEnabled.value,
  // 终端交互提示待答（issue #150 及其反馈扩展）：询问输入/选择处不开建议/补全。
  isInteractivePromptPending: () => interactivePromptGatePending(),
});

// —— 快速命令数据面（M32-A3）：RPC 全部留在 App，编辑器/导入视图在
// QuickCommandsSection（设置·终端），经 SettingsDialog 上抛意图。 ——

// 批量发送命令条（Electerm quick-command bar 风格）：常驻贴在终端底部，回车
// 生效只读门禁（来自 ssh/sessions/list 行的 readOnly：表单 read_only ∥
// 宿主标准 read_only）。后端门禁为权威来源，前端据此禁用写操作。
const connectionReadOnly = ref(false);
const settingsOpen = ref(false);
// reka Select 不接受空串 option value（空串 = 未选中占位）；空值选项用哨兵值双向映射。
const SELECT_EMPTY_SENTINEL = "__empty__";
const auditOpen = ref(false);
const profilesOpen = ref(false);
const diskUsage = ref<DiskUsage>();
// Warp 式 history 面板（↑ 唤起）：状态/过滤/键位消费收口在
// composables/useHistoryPanel；这里做数据与浮层互斥接线。
const {
  historyPanelOpen,
  historyPanelEnabled,
  setHistoryPanelEnabled,
  historyPanelFocusSearch,
  historyPanelQuery,
  historyPanelEntries,
  historyPanelActiveIndex,
  historyPanelAnchor,
  historyPanelGateOpen,
  openHistoryPanel,
  closeHistoryPanel,
  updateHistoryPanelFilter,
  selectHistoryEntry,
  handleHistoryPanelKey,
  handleHistoryPanelPanelKey,
} = useHistoryPanel({
  terminal: () => terminal,
  commandHistory: () => commandHistory.value,
  commandHistoryTimes: () => commandHistoryTimes.value,
  commandHistoryMeta: () => commandHistoryMeta.value,
  getPendingTerminalInput: () => pendingTerminalInput,
  getSessionId: () => session.value?.sessionId,
  isLocalMode: () => isLocalMode.value,
  commandRunning: () => commandRunning.value,
  terminalTransferBusy: () => terminalTransferBusy.value,
  completionOpen: () => completionOpen.value,
  suggestionOpen: () => suggestionOpen.value,
  quickSelectOpen: () => quickSelectOpen.value,
  searchOpen: () => searchOpen.value,
  interactivePromptPending: () => interactivePromptGatePending(),
  terminalMenuOpen,
  loadRemoteShellHistory,
  replaceTerminalLineWith,
  readEchoTextBeforeCursor,
  readTerminalSuggestionAnchor,
  hideGhostSuggestion: () => hideGhostSuggestion(),
  closeSuggestions,
  dismissPromptHints,
});
const pasteConfirm = ref<PasteConfirmation>();
// 终端拖入文件的落点询问：null 表示取消；"cwd" 用解析后的 shell/SFTP 当前
// 目录（resolveDropTargetDir：终端 cwd 跟随 → SFTP home → 面板当前目录），
// 弹窗展示解析结果；{ dir } 是用户输入的目标目录（文件原名落其下）。
const terminalFontSize = ref(appearance.value.terminal.fontSize);
// 终端字体单独设置（issue #31）：字体族/字号的用户覆盖，null 字段 = 跟随宿主。
// setup 期读取安全：loadTerminalFontOverride 经 pluginStore（内部全 guarded，
// opaque origin 不抛错），见 lib/terminalFont.ts 说明。
const terminalFontOverride = ref<TerminalFontOverride>(loadTerminalFontOverride());
// 终端外观偏好（对标 Tabby 的 Settings → Appearance）：配色方案两槽（随宿主
// 亮暗自动切换）、底色策略、字体间距、光标形态与多套主题快照。默认态为
// schemeSource: "host"，即不启用任何方案、行为与既有版本完全一致。
//
// 字体字段（font）不在此持久化状态里作为真相：权威态是 terminalFontOverride
// （Ctrl+滚轮缩放也写它）。读取时用 terminalAppearanceState 覆盖合成，避免
// 「缩放改过字号后主题仍高亮」这类双真相漂移。
const terminalAppearance = ref<TerminalAppearanceState>(loadTerminalAppearance());
const terminalAppearanceState = computed<TerminalAppearanceState>(() => ({
  ...terminalAppearance.value,
  font: { family: terminalFontOverride.value.fontFamily, size: terminalFontOverride.value.fontSize },
}));
// 当前配置命中的主题 id（null = 已改动，不再等于任何预设/我的主题）。
const activeAppearanceThemeId = computed(() => activeProfileId(terminalAppearanceState.value));
// 设置弹窗（独立组件 SettingsDialog）：实例 ref 用于 Esc 内联分层消费与
// 下载草稿回填；下载偏好权威态在本组件，经适配器交给组件读写。
const settingsDialog = ref<InstanceType<typeof SettingsDialog>>();
// 下载/传输/命令建议/SFTP 兼容/压缩传输偏好与 sidecar preferences 水合收口在
// composables/useWorkbenchPrefs；本组状态在早期声明（设置弹窗适配器直接引用
// load/persist 函数），后段偏好域经 accessor 惰性接入。
const {
  loadDownloadDir,
  persistDownloadDir,
  loadDownloadUseDefaultDir,
  persistDownloadUseDefaultDir,
  loadDownloadConflictPolicy,
  persistDownloadConflictPolicy,
  loadSftpCompatMode,
  persistSftpCompatMode,
  loadSftpNameEncoding,
  persistSftpNameEncoding,
  loadTransferCompressMode,
  persistTransferCompressMode,
  loadTransferCompressThreshold,
  persistTransferCompressThreshold,
  loadSuggestionsEnabled,
  persistSuggestionsEnabled,
  loadSuggestionMinChars,
  persistSuggestionMinChars,
  loadSuggestionMaxChars,
  persistSuggestionMaxChars,
  syncPrefs: syncPrefsNow,
  hydratePrefs,
  downloadPrompt,
  resolveDownloadPrompt,
  askDownloadTarget,
  applyChosenDirAsDefault,
  folderPickerTarget,
  onFolderPicked,
  downloadConflictPrompt,
  askDownloadConflict,
  resolveDownloadConflict,
  resolveDownloadConflictFor,
} = useWorkbenchPrefs({
  downloadDirState, downloadUseDefaultState, downloadConflictState,
  transferConcurrencyState, transferMaxActiveState, transferDownloadLimitState, transferDuplicateState,
  sftpCompatModeState, sftpNameEncodingState, transferCompressModeState, transferCompressThresholdState,
  suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState,
  transferKeys: {
    concurrency: TRANSFER_CONCURRENCY_KEY,
    maxActive: TRANSFER_MAX_ACTIVE_KEY,
    downloadLimit: TRANSFER_DOWNLOAD_LIMIT_KEY,
    duplicate: TRANSFER_DUPLICATE_KEY,
  },
  getLocalShell: () => localShellPref.value,
  setLocalShell: (value) => { localShellPref.value = value; },
  getLocalShellIntegration: () => localShellIntegrationPref.value,
  setLocalShellIntegration: (value) => { localShellIntegrationPref.value = value; },
  getLocalShellCwd: () => localShellCwdPref.value,
  setLocalShellCwd: (value) => { localShellCwdPref.value = value; },
  setLocalShellEnv: (value) => { localShellEnvPref.value = value; },
  setActionLinksSettings: (value) => { actionLinksSettings.value = value; },
  setGutterSettings: (value) => { gutterSettings.value = value; },
  setWallpaperEnabled: (value) => { wallpaperEnabled.value = value; },
  setWallpaperOpacity: (value) => { wallpaperOpacity.value = value; },
  loadWallpaperImage: () => loadWallpaperImage(),
  setCtxSearchEngines: (text) => { ctxSearchEnginesText.value = text; },
  settingsDialog,
  getLocalDownloadDir: () => localDownloadDir.value,
});
syncPrefsImpl = syncPrefsNow;
const downloadPrefsAdapter = {
  loadDir: loadDownloadDir,
  loadUseDefault: loadDownloadUseDefaultDir,
  loadConflict: loadDownloadConflictPolicy,
  persistDir: persistDownloadDir,
  persistUseDefault: persistDownloadUseDefaultDir,
  persistConflict: persistDownloadConflictPolicy,
};
// 传输并发/重复策略与命令建议的读写适配器（权威态在本组件，同下载偏好）。
const transferPrefsAdapter = {
  loadConcurrency: loadTransferConcurrency,
  loadDuplicatePolicy: loadTransferDuplicatePolicy,
  persistConcurrency: persistTransferConcurrency,
  persistDuplicatePolicy: persistTransferDuplicatePolicy,
  loadMaxActive: loadTransferMaxActive,
  persistMaxActive: persistTransferMaxActive,
  loadDownloadLimit: loadTransferDownloadLimit,
  persistDownloadLimit: persistTransferDownloadLimit,
  loadCompatMode: loadSftpCompatMode,
  persistCompatMode: persistSftpCompatMode,
  loadNameEncoding: loadSftpNameEncoding,
  persistNameEncoding: persistSftpNameEncoding,
  loadCompressMode: loadTransferCompressMode,
  persistCompressMode: persistTransferCompressMode,
  loadCompressThreshold: loadTransferCompressThreshold,
  persistCompressThreshold: persistTransferCompressThreshold,
};
const suggestionPrefsAdapter = {
  loadEnabled: loadSuggestionsEnabled,
  loadMinChars: loadSuggestionMinChars,
  loadMaxChars: loadSuggestionMaxChars,
  persistEnabled: persistSuggestionsEnabled,
  persistMinChars: persistSuggestionMinChars,
  persistMaxChars: persistSuggestionMaxChars,
};
// 终端 WebGL 渲染加速：偏好权威 = settingsPreferences store（与设置弹窗
// 共享同一 ref + 唯一落盘点），默认开；WebGL 不可用（headless/无 context）
// 时静默回退 DOM 渲染。只有主终端长期挂 renderer；回放弹窗保持 DOM 渲染，
// GIF 导出在导出期间给离屏终端临时挂载（取像素依赖 canvas），导出完随终端
// dispose 释放 context。
const webglEnabled = settingsPreferences.webglEnabled;
// 背景图（图源/开关/透明度 + 生效期强制 DOM 渲染器）收口在
// composables/useWallpaper；WebGL renderer 引用仍由本域持有并经 getter 注入。
const {
  wallpaperEnabled,
  wallpaperOpacity,
  wallpaperDataUrl,
  wallpaperSessionOnly,
  wallpaperActive,
  loadWallpaperImage,
  setWallpaperImage,
  clearWallpaperImage,
  updateWallpaperEnabled,
  updateWallpaperOpacity,
} = useWallpaper({
  webglEnabled,
  getTerminal: () => terminal,
  getWebglRenderer: () => webglRenderer.value,
  setWebglRenderer: (renderer) => { webglRenderer.value = renderer; },
  createWebglAddon: () => new WebglAddon(),
  webglRecoveryOptions,
  persistPrefs: (patch) => persistTerminalFeaturePrefs(patch),
});
const webglRenderer = ref<WebglRendererLike | null>(null);
// GPU 重置/驱动切换后有限次重建 renderer（Tabby 同款策略）：成功经
// onRecovered 回填引用，偏好已关闭则放弃重建，预算耗尽静默留在 DOM 渲染。
function webglRecoveryOptions(): WebglRecoveryOptions<WebglRendererLike> {
  return {
    onRecovered: (addon) => {
      webglRenderer.value = addon;
    },
    enabled: () => webglEnabled.value,
  };
}
// True while attachSession sits inside its bounded backoff loop; turns the
// status pill and overlay into the dedicated "reconnecting" phase.
const commandMarker = reactive({
  installed: false,
  active: false,
  command: "",
  exitCode: null as number | null,
  durationMs: null as number | null,
  cwd: "",
  // Start timestamp of the currently running command; drives the 1s tick that
  // keeps the marker strip duration live while a command is in flight.
  startedAt: null as number | null,
});
// Live elapsed milliseconds for the running marker (null when idle or finished).
const commandMarkerElapsed = ref<number | null>(null);
const sftpSearch = ref("");
const sftpTypeFilter = ref<SftpTypeFilter>("all");
/** 默认隐藏以 "." 开头的 Unix 隐藏文件、__pycache__ 等系统缓存条目；
 *  用户可通过过滤栏眼睛图标切换。 */
const sftpShowHidden = ref(false);
/** 显示/隐藏文件切换进全局布局偏好（issue #178），重启不再回落默认隐藏。 */
function toggleSftpShowHidden() {
  sftpShowHidden.value = !sftpShowHidden.value;
  persistState();
}
// 单栏文件树：文件夹与文件同树，目录懒加载。根固定连接根 "/"，可见行由
// 搜索/类型过滤在 flattenVisibleTreeRows 内完成；entries 仍是"当前目录"的
// 数据源，树节点在 loadDirectory 落地后同步子树。
const sftpTreeRoot = ref(createFileTreeRoot("/"));
const sftpTreeRows = computed(() =>
  flattenVisibleTreeRows(sftpTreeRoot.value, {
    search: sftpSearch.value,
    type: sftpTypeFilter.value === "directory" ? "directory" : sftpTypeFilter.value === "file" ? "file" : "all",
  }),
);
const sftpTreeEntries = computed(() => sftpTreeRows.value.map((row) => row.node.entry));
const sftpTreeTotalRows = computed(() => flattenVisibleTreeRows(sftpTreeRoot.value, {}));
// 已加载树节点全集（含被搜索/类型过滤隐藏或折叠在后代里的条目）：批量计数、
// 删除/下载目标都以它为准，避免过滤/折叠把已选中的远端条目静默丢掉。
function collectLoadedTreeEntries(node: SftpFileTreeNode, out: SftpSanitizedEntry[]): void {
  out.push(node.entry);
  for (const child of node.children) collectLoadedTreeEntries(child, out);
}
const loadedTreeEntries = computed(() => {
  const out: SftpSanitizedEntry[] = [];
  collectLoadedTreeEntries(sftpTreeRoot.value, out);
  return out;
});
// 路径弹层快捷路径（侧栏移除后仍保留在路径历史 popover 内）。
const SFTP_QUICK_PATHS = ["/", "/home", "/tmp", "/etc", "/var", "/root"];
// 行选中态（多选集合/保护目录拦截/键盘语义）收口在 composables/useSftpRowSelection；
// 选中/剪贴板动作本体仍由下方 useSftpSelectionClipboard 提供（惰性注入）。
const {
  selectedUris,
  selectedUriSet,
  lastClickedUri,
  remoteBasename,
  isProtectedDirectory,
  loadedDirectoryUris,
  isLoadedDirectoryUri,
  selectTreeEntry,
  onFileRowKeydown,
} = useSftpRowSelection<SftpEntry>({
  currentPath, loadedTreeEntries, selectedPath, deleteTarget, pathFromUri,
  canWrite: () => canWrite.value,
  selectFile: (entry, event) => selectFile(entry, event),
  openEditorTab: (entry) => openEditorTab(entry),
  beginRename: (entry) => beginRename(entry),
});
const sftpClipboard = ref<SftpClipboard>();
const pasteBusy = ref(false);
const newFileDialog = ref(false);
const newFileDraft = ref("");
const newFileSubmitting = ref(false);
// OTP 验证码 / 会话导入工具弹窗（SFTP 路径工具栏入口）。
const otpPanelOpen = ref(false);
const importPanelOpen = ref(false);
const batchDeleteOpen = ref(false);
const batchDeleteSubmitting = ref(false);
// Aggregated progress for multi-item batch operations (delete / archive);
// null while no batch is in flight.
const batchProgress = ref<BatchProgressState | null>(null);

let terminal: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let searchAddon: SearchAddon | undefined;
let osc52Disposable: { dispose(): void } | undefined;
// WT-2：OSC 通知/SetUserVar 白名单通道（OSC 9 / 777 / 1337）的 disposable：
// createTerminal 挂一次，终端销毁统一释放。解析与防御上限在 lib/terminalOscChannels.ts。
let oscFeedDisposables: { dispose(): void }[] = [];
// SetUserVar cwd 元数据（优先通道）的最新上报；null 表示通道未启用，OSC 7
// 提示符通道照旧跟随（回落语义不变）。裁决规则见 shouldOsc7FollowOverrideUserVarCwd。
let userVarCwdEvent: { path: string; at: number } | null = null;
// 主题色解析失败时颜色查询的兜底应答（深色系常规值，仅在宿主下发非法颜色时触达）。
let terminalPasteHandler: ((event: ClipboardEvent) => void) | undefined;
let terminalWheelHandler: ((event: WheelEvent) => void) | undefined;
// 点击定位光标（iTerm2 风格）：按下位置记忆 + 松开时判定“原地点击”。
let terminalMouseDownHandler: ((event: MouseEvent) => void) | undefined;
let terminalMouseUpHandler: ((event: MouseEvent) => void) | undefined;
let pasteConfirmResolver: ((accepted: boolean) => void) | undefined;
// 通用应用内确认弹窗（SSH-H1）：原语在 lib/confirmDialog（SettingsDialog/
// QuickCommandsSection 等子组件共用），此处只挂宿主渲染；模板/关框/Esc/
// teardown 统一经 resolvePendingConfirmDialog 结算。
const { pendingConfirmDialog, resolveConfirmDialog: resolvePendingConfirmDialog } = useConfirmDialogHost();
let resizeObserver: ResizeObserver | undefined;
let disposeInput: { dispose(): void } | undefined;
let disposeWebkitInputFallback: (() => void) | undefined;
let disposeSelectionCopy: { dispose(): void } | undefined;
let disposeTerminalBell: { dispose(): void } | undefined;
/** 响铃视觉提示的短暂高亮（xterm 6.x 无 bellStyle，须自行实现）。 */
let unsubscribeEvent: (() => void) | undefined;
let unsubscribeBinary: (() => void) | undefined;
// 宿主 fileTransfer 桥拖放事件（宿主 1.1 optional）注销句柄：OS 级拖入上传
// 与终端/SFTP 面板共享同一道门禁，见 handleHostFileDrop。
let unsubscribeFileDrag: (() => void) | undefined;
let unsubscribeFileDrop: (() => void) | undefined;
// X-P2/P3/P4 收敛：宿主环境订阅聚合句柄（shared/frontend/hostThemeRuntime）。
let unsubscribeEnvironment: (() => void) | undefined;
let resizeTimer = 0;
let disposed = false;
const OPEN_RETRY_MAX = 3;
let openRetryAttempt = 0;
let lastSequence = 0;
let replayInFlight = false;
// Consecutive replays that returned complete without filling the detected
// sequence hole. A sidecar that keeps reporting complete on a gap that never
// closes cannot self-heal by retrying — after a few attempts the drain must
// resync past the hole instead of spinning the replay loop forever.
let replayNoProgress = 0;
let commandMarkerTimer = 0;
let pendingTerminalInput = "";
let activeTerminalSessionId = "";
const pendingTerminalFrames = new Map<number, { stream: number; data: Uint8Array }>();
const directoryParser = new Osc7DirectoryParser();
// 提示符换行修复（t8y2/dbx#10750）：sidecar 给每个交互 shell 会话武装了
// 按提示符发 OSC 7 的钩子，这里把帧当作"提示符即将绘制"的信号——可见输出
// 不在行首就在帧前补 CRLF。字节级透传，无钩子流恒等直通。
const promptNewline = new PromptNewlineTransformer();
// OSC 633 shell-integration markers (pure frontend parse; no-op streams pass through).
const commandMarkerParser = new Osc633CommandParser();

// —— 本地终端（sidecar 所在机器的交互式登录 shell）——
// 与 SSH 会话共用同一条渲染管线（乱序重组/补发/633 命令标记/输出节流），
// 同一时刻只展示一个会话：进入本地模式前先经确认关闭 SSH 会话。会话帧用
// 独立的 sequence/pending 状态，避免与 SSH 流互染。
const localSession = ref<{ sessionId: string; shell: string } | null>(null);
const localState = ref<"starting" | "running" | "exited">("exited");
const localExitCode = ref<number | null>(null);
const localOpenConfirmOpen = ref(false);
const isLocalMode = computed(() => localSession.value !== null);
// A4 restored shell (spec §7.6/§8.4): a restored local tab never auto-starts a shell; the exit
// overlay as the shell state waiting for an explicit start; cleared once a shell actually comes up (startLocalTerminal succeeds)
// cleared. localUiMode = "local-terminal UI state" (running session or restored shell); SSH-only toolbar actions gate on it,
// SSH-only toolbar actions and display branches gate on it, decoupled from session existence.
const localShellRestored = ref(false);
// Telnet 会话（P2-3）：与 SSH/本地终端同款互斥展示。并入 localUiMode 后，
// 所有 SSH-only 工具栏分支对 Telnet 自动隐藏；Telnet 专属分支按 isTelnetMode
// 优先接在既有 localSession 分支前面。
const telnetSession = ref<{ sessionId: string; host: string; port: number } | null>(null);
const telnetDialogOpen = ref(false);
const telnetState = ref<"idle" | "connecting" | "running" | "closed">("idle");
const telnetError = ref("");
const isTelnetMode = computed(() => telnetSession.value !== null);
const telnetTarget = computed(() => (telnetSession.value ? `${telnetSession.value.host}:${telnetSession.value.port}` : ""));
// 串口会话（P3 + B1 增强）：与 SSH/本地/Telnet 同款互斥展示，并入
// localUiMode。出帧由读线程单线程递增 sequence；键盘输入主路径走
// `serial/terminal/in/{id}` 二进制写通道（B1，Stdin=3 标签），JSON
// `serial/write` 保留为兼容/降级路径；resize 无协议概念（设计稿 §4）。
const serialSession = ref<{ sessionId: string; port: string; baudRate: number } | null>(null);
const serialDialogOpen = ref(false);
const serialState = ref<"idle" | "running" | "closed">("idle");
const serialError = ref("");
// B1 解码契约：输出帧遇到未知流标签（> Stdin=3）一律静默丢弃并计数。
// B1 能力开关：true = 键盘走二进制写通道。初始值取 serial/start 的
// binaryInput 能力字段（未声明 = 旧 sidecar → JSON 兼容路径）；通道报错
// （未知方法/会话消失）时 send 回调一次性降级 JSON。
const serialBinaryInput = ref(true);
const isSerialMode = computed(() => serialSession.value !== null);
const serialTarget = computed(() => (serialSession.value ? `${serialSession.value.port}@${serialSession.value.baudRate}` : ""));
// 串口文件上传 overlay 状态：进度由 sidecar 的 serial/upload/progress 事件
// 驱动；running 期间吞掉键入（协议控制字符窗口）且禁止并发第二次 upload。
// 状态机/送数/取消收口在 composables/useSerialUpload（会话关闭经
// cancelForSessionClose 静默复位）。
const {
  serialUpload,
  serialUploadDialogOpen,
  serialUploadBusy,
  serialUploadOverlayVisible,
  serialUploadPercentValue,
  serialUploadStatusLabel,
  startSerialUpload,
  cancelSerialUpload,
  cancelForSessionClose: cancelSerialUploadForSessionClose,
} = useSerialUpload({
  t: (key, values) => t(key, values),
  showError,
  getSessionId: () => serialSession.value?.sessionId,
});
// RDP 会话（nyaterm-parity P3-4）：与 SSH/本地/Telnet/串口/VNC 同款互斥展示，
// 并入 localUiMode。画面走 RdpSurface 画布（帧从 rdp/frame/{id} 解码），状态
// 经 lib/rdpFrame 的纯 reducer 折叠（connecting/connected/reconnecting/closed
// + errorKind），断线重连展示手动 rdp/reconnect 出口。
const rdpSession = ref<{ sessionId: string; host: string; port: number } | null>(null);
const rdpDialogOpen = ref(false);
// RDP 实验门控的权威在后端（rdp/start 前置检查 local/preferences 的
// rdp_experimental_enabled，直接 RPC 也无法绕过）；M32-A 移除工具条入口后
// 前端不再持有内存门副本。
const rdpState = ref<RdpSessionStateView>(initialRdpSessionState());
const rdpScaleMode = ref<RdpConnectOptions["scaleMode"]>("fit");
const rdpSurface = ref<InstanceType<typeof RdpSurface> | null>(null);
// 超大剪贴板的分片拼接缓冲（sidecar C2 修复：>8 MiB 的 JSON 事件会被 SDK
// 上限静默丢弃，后端按 chunkIndex/chunkTotal 分片发送；sessionId 隔离防串话）。
const rdpClipboardChunks = new Map<string, { parts: string[]; received: number }>();
const isRdpMode = computed(() => rdpSession.value !== null);
const rdpTarget = computed(() => (rdpSession.value ? `${rdpSession.value.host}:${rdpSession.value.port}` : ""));
const localUiMode = computed(() => isLocalMode.value || localShellRestored.value || isTelnetMode.value || isSerialMode.value || isVncMode.value || isRdpMode.value);
// —— 本地终端偏好/shell 发现/选择器：收口在 composables/useLocalShellPreferences ——
// （本地终端进程本身仍由本文件 start/close，牵动输出管线状态。）
const {
  localShellPref,
  localShellIntegrationPref,
  localShellCwdPref,
  localShellEnvPref,
  localShellEnvClean,
  localShells,
  localShellsLoading,
  selectedShellInjectable,
  localShellSurfaceOpen,
  dockConnections,
  pickLocalShell,
  openSessionShellPick,
  openLocalShellPrefs,
  openLocalShellSurfaceMenu,
  openConnectionSurface,
  openLocalShellSurface,
  setLocalShellPref,
  setLocalShellIntegrationPref,
  setLocalShellCwd,
  setLocalShellEnv,
  addLocalShellEnvRow,
  removeLocalShellEnvRow,
  canOpenLocalTab,
  openLocalTerminalTab,
} = useLocalShellPreferences({
  isLocalMode: () => isLocalMode.value,
  isLocalShellRestored: () => localShellRestored.value,
  restartLocalTerminal: (shellOverride) => { void restartLocalTerminal(shellOverride); },
  requestLocalTerminal: (shellOverride) => requestLocalTerminal(shellOverride),
  // 会话下拉状态在本段之后声明，惰性写入。
  closeSessionMenu: () => { sessionMenuOpen.value = false; },
  setShellPickOpen: (open) => { sessionMenuShellPickOpen.value = open; },
});
// shell 选择器菜单：打开 shell 选项视图时拉一次 local/shells/list。
// 会话下拉（新建/复制/命令会话/本地终端/Shell 选项）单入口状态；shell 视图
// 为同弹层内的视图切换（非独立 popover，Esc/外点由 reka 随弹层整体收口）。
const sessionMenuOpen = ref(false);
const sessionMenuShellOpen = ref(false);
// 「选择 Shell 打开」子视图：同弹层视图切换（dock「+」同款列表），语义是
// 一次性 override 打开，与设置子视图（radio = 默认偏好）刻意分开。
const sessionMenuShellPickOpen = ref(false);
// Dock 面板配色快切 + 工具条隐藏偏好：收口在 composables/useWorkbenchToolbar。
const {
  panelPaletteOpen,
  panelToolbarHidden,
  togglePanelToolbar,
  panelSchemeChoices,
  panelActiveSchemeId,
  pickPanelScheme,
} = useWorkbenchToolbar({
  appearanceColorScheme: () => appearance.value.colorScheme,
  terminalAppearance: () => terminalAppearance.value,
  // 惰性转发：updateTerminalAppearance 在本段之后才由 useTerminalAppearance 解构。
  updateAppearance: (patch) => updateTerminalAppearance(patch),
});
// 上次本地会话跟踪到的 cwd：重开时继承（VS Code 新终端继承工作区目录惯例）。
const localLastCwd = ref("");
// 本地终端最近命令（633;E 收集， newest-first，cap 20）：右键菜单"重跑"用。
// 注入关闭时无命令边界，本功能静默缺席。
const localRecentCommands = ref<string[]>([]);

// Large-output rendering throttle: coalesce consecutive PTY frames into one
// merged xterm write per animation frame (capped, order preserving). The sink
// reads `terminal` lazily so it also works across terminal recreation. The
// write completion callback is the gutter timestamp capture point (P1-3): it
// stamps the logical rows each merged batch actually produced.
//
// 大输出保护（IMPL_PLAN Task P2-7）：积压口径 = 已交给 xterm 但 write 回调尚未
// 触发（还没解析完）的字节数。≥128KiB 进入 strained：合并批次按 32KiB 分帧写、
// 挂起 gutter/关键词高亮/动作链接扫描；回落到 64KiB 以下自动恢复并补扫一次。
const outputGate = createOutputGate();
let outputInFlightBytes = 0;
function settleOutputChunk(chunk: Uint8Array) {
  outputInFlightBytes = Math.max(0, outputInFlightBytes - chunk.byteLength);
  // 写入完成回调同时是模式复评点；恢复时挂起的扫描由 onOutputGateRelease 补上。
  if (outputGate.feed(outputInFlightBytes) && outputGate.mode === "normal") onOutputGateRelease();
  stampGutterWrittenRows();
  syncSuggestionAnchorsOnSettle();
  syncGhostAnchorOnSettle();
}
function onOutputGateRelease() {
  showNotice(t("backpressure.released"));
  rescanHighlightViewport();
  if (actionLinksEnabled.value && terminal) scheduleActionLinkScan(0, terminal.rows - 1);
  scheduleGutterRecompute();
}
const terminalWriteThrottle: TerminalWriteThrottle = createTerminalWriteThrottle({
  sink: (data) => {
    if (!terminal) return;
    outputInFlightBytes += data.byteLength;
    if (outputGate.feed(outputInFlightBytes) && outputGate.mode === "strained") {
      showNotice(t("backpressure.engaged"));
      scheduleGutterRecompute();
    }
    if (outputGate.mode === "strained") {
      for (const frame of outputGate.write(data)) terminal.write(frame, () => settleOutputChunk(frame));
    } else {
      terminal.write(data, () => settleOutputChunk(data));
    }
  },
});
// DECSET 2026 同步渲染（WT-1，对标 WezTerm 帧合并）：`CSI ? 2026 h` 期间挂起
// 合帧通道的定时提交、写入持续入队，`CSI ? 2026 l` 把积压整批合并一次提交——
// vim 重绘 / cat 大文件在同步窗内的中间帧不再逐帧上屏，撕裂闪烁随之收敛。
// 积压受合帧通道 1MiB 字节上限约束；hold 另有 100ms 截止时限：PTY 读边界把
// 同步窗切成两帧时（codex 30fps 小帧流下必然发生），release 自身被 hold 挡在
// 队列里，不定时放行会一直积到字节帽才渲染——表现为终端长时间"卡死"。
// 防卡死优先于帧完整，异常驻留同步态时按 deadline 强制放行。
const terminalSyncOutput: TerminalSyncOutput = {
  begin: () => terminalWriteThrottle.setHold(true),
  end: () => terminalWriteThrottle.setHold(false),
};
// #33/#71 快速输入丢字母的分层计数：keys(onData 实际路由到 PTY 的按键)、
// sends(提交给宿主桥的帧)、acks(sidecar 确认收到的帧)、errors(桥拒绝)、
// swallowed(被 zmodem/trzsz 路由吞掉的按键)。宿主开启 localStorage 的
// dbx-term-diag=1 后每 2s 在控制台输出；始终挂在 window 上便于随时读取。
const terminalDiag = reactive({ keys: 0, sends: 0, acks: 0, errors: 0, swallowed: 0 });
const terminalDiagVisible = ref(false);
let terminalDiagTimer = 0;
if (typeof window !== "undefined") {
  (window as unknown as Record<string, unknown>).__dbxTerminalDiag = terminalDiag;
  let diagEnabled = false;
  try {
    diagEnabled = window.localStorage?.getItem("dbx-term-diag") === "1";
  } catch {
    // 沙箱策略禁止 localStorage 时诊断保持关闭。
  }
  if (diagEnabled) {
    // 句柄入 onBeforeUnmount 清理：webview 反复重建时残留的 interval 会一直
    // 打 console 并持有 reactive 对象。
    terminalDiagTimer = window.setInterval(() => {
      console.info("[term-diag]", JSON.stringify(terminalDiag));
    }, 2000);
  }
}

const terminalInputQueue = createTerminalInputQueue({
  // Only SSH sends use the reconnect watcher below. A stalled local PTY or
  // Telnet send must not be mislabeled as an exited local process or SSH loss.
  sendTimeoutMs: (sessionId) => (sessionId === session.value?.sessionId ? 8_000 : 0),
  send: (sessionId, payload) => {
    terminalDiag.sends += 1;
    // 通道按会话身份选择（telnet: 前缀 / 本地会话 / SSH，见
    // terminalInputChannel）：本地会话误发 ssh 通道会被 sidecar 的 SSH
    // 会话表拒收，表现为终端看似在线却打不进字。
    return window.dbxPlugin.sendBinary(terminalInputChannel(sessionId, localSession.value?.sessionId), payload);
  },
  onError: (cause) => {
    terminalDiag.errors += 1;
    terminalInputQueue.reset();
    // 本地会话的输入错误（进程退出/会话被回收）直接落退出态，不走 SSH 重连梯子。
    if (isLocalMode.value) {
      markLocalExited(null);
      return;
    }
    const sshSessionId = session.value?.sessionId;
    if (!isTelnetMode.value && sshSessionId && (cause instanceof TerminalInputTimeoutError || isSessionGoneError(cause))) {
      terminalTransportWatch.reportUnavailable(sshSessionId);
      return;
    }
    showError(cause, "terminal");
    // 会话被外部杀掉（宿主重推连接的 disconnect、sidecar 重启）时本 tab 无
    // 事件感知，终端看似活着实则打不进字。输入撞上死会话时按传输断开的
    // 同款有界梯子自动重连。错误串契约见 backend ssh.rs session()。
    if (terminalState.value === "connected" && !reconnectPending.value && isSessionGoneError(cause)) scheduleSessionReconnect();
  },
});

// 串口专用输入队列：与 SSH/Telnet 共用同一有序实现，但负载带 Stdin 流
// 标签（B1 TerminalFrame 形状），序列号独立计数；宿主对未知通道/方法报错
// 时一次性降级 JSON 兼容路径（serialBinaryInput）。
const serialInputQueue = createTerminalInputQueue({
  frameTag: SERIAL_STREAM_STDIN,
  send: (sessionId, payload) => {
    if (!sessionId.startsWith("serial:")) return;
    return window.dbxPlugin
      .sendBinary(`serial/terminal/in/${sessionId.slice("serial:".length)}`, payload)
      .catch((cause: unknown) => {
        serialBinaryInput.value = false;
        throw cause;
      });
  },
  onError: (cause) => showError(cause, "terminal"),
});

const locale = ref("zh-CN");
const t = (key: string, values: Record<string, string | number> = {}) => workbenchMessage(locale.value, key, values);
// 宿主用 iframe 的 document.title 命名工作台标签：标题跟随 locale，
// 中文环境显示「终端」而不是静态的英文插件名。
watch(locale, (value) => { document.title = workbenchMessage(value, "pluginName"); }, { immediate: true });
// html lang 同步跟随 locale：屏幕阅读器/拼写检查按它选语言，滞留 zh-CN 会让
// 其他语种读错音（i18n 走查实录：locale=ja 时翻译已切换而 lang 仍为 zh-CN）。
watch(locale, (value) => { document.documentElement.lang = value; }, { immediate: true });
const connectionId = computed(() => normalizeConnectionText(hostContext.value.connectionId));
// Host API 1.1 provides a stable workbenchId in the host context; on 1.0 a
// locally generated id keeps session scoping per workbench instance (A4 W1
// helper, spec §11: host-authoritative workbenchId; the fallback covers 1.0
// hosts that omit the injection — see lib/pluginContext.spec.ts).
const fallbackWorkbenchId = randomUUID();
const workbenchId = computed(() => resolveWorkbenchId(hostContext.value, fallbackWorkbenchId));
// RDP 会话生命周期与证书确认：收口在 composables/useRdpSession（rdpSession/
// rdpState/rdpSurface 等模板共享态留守 App.vue，经 options 注入）。
const {
  rdpCertPrompt, rdpCertRemember, rdpCertRemaining,
  startRdpSession, closeRdpSession, sendRdpInput, sendRdpClipboard,
  reconnectRdpSession, resolveRdpCertificate,
} = useRdpSession({
  t, showError, showNotice,
  rdpSession, rdpState, rdpSurface, rdpScaleMode, rdpClipboardChunks,
  workbenchId,
  connectionId,
  isDisposed: () => disposed,
  getTerminal: () => terminal,
});
// —— RDP 工作台专属逻辑（大文本粘贴 + 分辨率入口）：收口在 rdp/useRdpWorkbench，
// App.vue 只保留薄挂载点（模板绑定经解构同名透出）。 ——
const {
  rdpPasteOpen, rdpPasteText, rdpTypingBusy, rdpTypingSent, rdpTypingTotal,
  rdpWriteRemoteClipboard, rdpTypeAsKeystrokes,
  RDP_RESOLUTION_PRESETS, rdpResolutionOpen, rdpCustomWidth, rdpCustomHeight, rdpResizing,
  rdpDesktopSize, rdpPersistedSize, rdpResolutionLabel, isRdpResolutionActive,
  rdpWindowProportionPreset, applyRdpResolution, applyRdpCustomResolution,
} = useRdpWorkbench({
  t, showError, showNotice, persistState,
  isRdpMode, rdpSession, rdpSurface,
  sendRdpInput, sendRdpClipboard,
});

const restored = computed(() => hostContext.value.restored === true);
const runtimeEndpoint = computed<RuntimeEndpoint>(() => {
  const value = hostContext.value.runtime;
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const raw = value as Record<string, unknown>;
  return {
    host: normalizeConnectionText(raw.host),
    port: normalizeConnectionPort(raw.port),
  };
});
// VNC 会话生命周期与输入/帧入口：收口在 composables/useVncSession
// （vncSession/vncState/vncSurface 等模板共享态由组合式函数持有并解构透出）。
const {
  vncSession, vncDialogOpen, vncState, vncError, vncScaleMode, vncSurface,
  isVncMode, vncTarget,
  markVncClosed, startVncSession, closeVncSession, sendVncInput, sendVncClipboard,
  handleVncFrame, reattachVncSession,
} = useVncSession({
  t, showError, showNotice,
  connectionId,
  workbenchId,
  runtimeEndpoint,
  isDisposed: () => disposed,
  getTerminal: () => terminal,
});
const connection = computed<ConnectionSummary>(() => {
  const value = hostContext.value.connection;
  // 宿主下发的 connection 是配置对象（mockDbxHost 同契约）；对象之外一律视为缺失。
  if (!value || typeof value !== "object" || Array.isArray(value)) return {} as ConnectionSummary;
  const raw = value as Record<string, unknown>;
  // 宿主把插件表单状态放在嵌套的 external_config 里（生命周期推送同构：
  // model.rs RuntimeEndpoint 即从 connection.external_config 读 protocol）；
  // 顶层拍平与嵌套两级都读，顶层优先。
  const ext = raw.external_config && typeof raw.external_config === "object" && !Array.isArray(raw.external_config)
    ? (raw.external_config as Record<string, unknown>)
    : {};
  const field = (key: string): unknown => raw[key] ?? ext[key];
  // serial/rdp 连接配置（M32-B）：manifest select 存的是字符串值，逐项
  // 白名单归一，非法/缺失回落各 startXXXSession 的默认值。
  const resolution = typeof field("rdp_resolution") === "string" ? (field("rdp_resolution") as string).match(/^(\d{3,4})x(\d{3,4})$/) : null;
  const protocolRaw = typeof field("protocol") === "string" ? (field("protocol") as string) : undefined;
  return {
    name: normalizeConnectionText(raw.name),
    host: normalizeConnectionText(raw.host),
    port: normalizeConnectionPort(raw.port),
    username: normalizeConnectionText(raw.username),
    color: normalizeConnectionText(raw.color),
    readOnly: raw.readOnly === true,
    protocol: protocolRaw === "telnet" || protocolRaw === "vnc" || protocolRaw === "serial" || protocolRaw === "rdp" ? protocolRaw : "ssh",
    serialPort: normalizeConnectionText(field("serial_port")),
    serialBaud: [9600, 19200, 38400, 57600, 115200, 230400].includes(Number(field("serial_baud"))) ? Number(field("serial_baud")) : undefined,
    serialDataBits: field("serial_data_bits") === "7" ? "7" : field("serial_data_bits") === "8" ? "8" : undefined,
    serialParity: field("serial_parity") === "even" || field("serial_parity") === "odd" ? (field("serial_parity") as "even" | "odd") : field("serial_parity") === "none" ? ("none" as const) : undefined,
    serialStopBits: field("serial_stop_bits") === "1" ? "1" : field("serial_stop_bits") === "2" ? "2" : undefined,
    serialBackspace: field("serial_backspace") === "ctrl_h" ? "ctrl_h" : field("serial_backspace") === "del" ? "del" : undefined,
    rdpDomain: normalizeConnectionText(field("rdp_domain")),
    rdpWidth: resolution ? Number(resolution[1]) : undefined,
    rdpHeight: resolution ? Number(resolution[2]) : undefined,
    rdpCertificatePolicy: field("rdp_certificate_policy") === "strict"
      ? ("strict" as const)
      : field("rdp_certificate_policy") === "accept-temporarily"
        ? ("accept-temporarily" as const)
        : field("rdp_certificate_policy") === "prompt"
          ? ("prompt" as const)
          : undefined,
    rdpClipboard: field("rdp_clipboard") === undefined ? undefined : field("rdp_clipboard") === true || field("rdp_clipboard") === "true",
  };
});
// sidecar 查询到的权威协议（connection/protocol）：真实宿主推给 webview 的
// connection 上下文缺 external_config.protocol，openSession 路由时查询结果必须
// 写回这里，协议路由与连接级门控才有可靠的事实源——只用于当次路由而不落状态的
// 话，isRdpConnection 在真实宿主恒为 ssh 形态，SSH 专属入口全部漏出。
// 连接切换（宿主改推另一连接）即失效，避免把上一连接的协议带给下一个连接。
const storedConnectionProtocol = ref("");
watch(connectionId, () => {
  storedConnectionProtocol.value = "";
});
// 连接协议（对标 Tabby profile）：sidecar 暂存的权威协议 > 宿主连接表单的
// protocol 字段，缺省 ssh。非 SSH 连接由 openSession 路由到各自的会话视图，
// 不建立 SSH 会话。
const connectionProtocol = computed<"ssh" | "telnet" | "vnc" | "serial" | "rdp">(() => {
  const protocol = storedConnectionProtocol.value || connection.value.protocol;
  return protocol === "telnet" || protocol === "vnc" || protocol === "serial" || protocol === "rdp" ? protocol : "ssh";
});
// RDP 连接（宿主表单 protocol=rdp）：SSH/SFTP/主机运维整类能力在 RDP 连接上
// 无处生效，工具条、面板与覆盖层按连接协议整体隐藏（而非置灰）——与 localUiMode
// 的"local 模式隐藏 SSH 专属动作"惯例一致。区别于 isRdpMode（仅 RDP 会话激活
// 期），连接级门控让断开态下残留的 SSH 入口也不再出现。并集 isRdpMode 兜住
// 对话框直连路径（不经 openSession 查询，表单协议无从知晓）与会话已激活、
// 协议查询尚未返回的窗口期——这两段里 SSH 入口同样一处都不该出现。
const isRdpConnection = computed(() => connectionProtocol.value === "rdp" || isRdpMode.value);
const canWrite = computed(() => !connection.value.readOnly && !connectionReadOnly.value);
const selectedEntry = computed(() => entries.value.find((entry) => entry.uri === selectedPath.value));
const connected = computed(() => terminalState.value === "connected" && !!session.value);
// 终端分屏：布局树状态与窗格/页签操作收口在 composables/useShellTabs；
// 「页签上报 cwd」「窗格激活」两个目录跟随入口仍在本文件（结构测试锁定）。
const {
  terminalLayout,
  shellTabSessions,
  activeTerminalPaneId,
  shellTabCwd,
  activePaneCwd,
  openTerminalSplit,
  splitTerminalPane,
  closeTerminalSplit,
  closePane,
  createPaneTab,
  activatePaneTab,
  closePaneTab,
  onShellTabOpened,
  resetTerminalLayout,
} = useShellTabs({
  terminalCwd,
  canSplit: () => canSplitTerminal.value,
  canCreateTab: () => canOpenShellTab.value,
  scheduleFit,
  syncSftpToActivePane: () => syncSftpToActivePane(),
});
const canSplitTerminal = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);
const terminalHasSplit = computed(() => hasTerminalSplit(terminalLayout.value));
const splitTerminalFont = computed(() =>
  resolveTerminalFont(terminalFontOverride.value, {
    fontFamily: hostTerminalFontFamily(appearance.value),
    fontSize: appearance.value.terminal.fontSize,
  }),
);

/** 分屏页签上报 OSC 7：记录并只在真实目录变化时驱动跟随。 */
function onShellTabCwd(tabId: string, path: string) {
  const previous = shellTabCwd.get(tabId);
  shellTabCwd.set(tabId, path);
  // 首次上报是该页签的初始提示符；只有它本来就是活动窗格的活动页签（cwd
  // 迟到）才跟随。重复上报（回车/resize/提示符重绘）不重复导航。
  const activeTabId = findTerminalPane(terminalLayout.value, activeTerminalPaneId.value)?.activeTabId;
  if (!shouldFollowTabCwd(previous, path, activeTabId === tabId)) return;
  if (!followDirectory.value || !connected.value) return;
  void followSftpPath(path);
}

/** 任意窗格内按下/聚焦：记录最后交互窗格，并把 SFTP 同步到该窗格的目录。 */
function onPaneActivated(paneId: string) {
  if (!paneId || activeTerminalPaneId.value === paneId) return;
  activeTerminalPaneId.value = paneId;
  syncSftpToActivePane();
}

/**
 * 聚焦/切换窗格或页签时显示该窗格的活动目录。只在“真正切换了窗格/页签”时
 * 调用（同一窗格内重复点击终端不会触发），因此会重新置顶但不会打断用户在
 * 当前窗格里的连续操作。
 */
function syncSftpToActivePane() {
  if (!followDirectory.value || !connected.value) return;
  const cwd = activePaneCwd.value;
  if (!cwd) return;
  void followSftpPath(cwd);
}

const canOpenShellTab = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);
// 主会话断开/切换本地/RDP 时销毁所有分屏与额外标签，避免留下不可见的 PTY。
watch(canSplitTerminal, (allowed) => {
  if (!allowed) resetTerminalLayout();
});
// 指标视图依赖 SSH 会话；断开后收起右栏，避免保留一份不再刷新的快照。
watch(connected, (isConnected) => {
  if (!isConnected && rightPanelView.value === "metrics") closeRightPanel();
});
const sessionStatus = computed<WorkbenchSessionStatus | "local">(() => (localUiMode.value ? "local" : describeWorkbenchSessionStatus(terminalState.value, { reattaching: reconnectPending.value })));
// 本地模式徽标附带 shell 名（Local · Zsh），一眼可见当前在哪种 shell 里。
const sessionPillText = computed(() => {
  // 串口徽标显示 port@baud（Serial · /dev/ttyUSB0@115200），一眼可见线路参数。
  if (isSerialMode.value) return `${t("serial.pillPrefix")} · ${serialTarget.value}`;
  // Telnet 徽标显示明文目标（Telnet · host:port），提示这是非 SSH 连接。
  if (isTelnetMode.value) return telnetState.value === "connecting" ? t("telnet.connecting") : `${t("telnet.pillPrefix")} · ${telnetTarget.value}`;
  // VNC 徽标显示远端桌面目标（VNC · host:port）。
  if (isVncMode.value) return vncState.value === "connecting" ? t("vnc.connecting") : `${t("vnc.pillPrefix")} · ${vncTarget.value}`;
  // RDP 徽标：连接中/重连中（带退避进度）显示状态，其余显示 host:port。
  if (isRdpMode.value) {
    if (rdpState.value.state === "connecting") return t("rdp.connecting");
    if (rdpState.value.state === "reconnecting") {
      return rdpState.value.maxAttempts > 0
        ? t("rdp.reconnectingAttempt", { attempt: rdpState.value.attempt, max: rdpState.value.maxAttempts })
        : t("rdp.reconnecting");
    }
    return `${t("rdp.pillPrefix")} · ${rdpTarget.value}`;
  }
  if (!isLocalMode.value || !localSession.value) return t(`sessionStatus.${sessionStatus.value}`);
  const kind = localSession.value.shell.split(/[\\/]/).pop() || localSession.value.shell;
  return `${t("sessionStatus.local")} · ${kind}`;
});
// Connect-error friendlification: raw sidecar/russh error strings stay as the
// tooltip detail while the primary line renders a localized per-category hint
// (auth / refused / DNS / timeout / host key). Non-connect errors pass through.
const terminalErrorDetail = computed(() => terminalError.value);
const terminalErrorFriendly = computed(() => {
  const kind = classifyConnectError(terminalError.value);
  return kind ? t(connectErrorKey(kind)) : "";
});
// Reconnect countdown lifecycle: while the backoff loop is pending a 250ms
// tick recomputes the pure countdown; any exit from "reconnecting" stops it.
const commandOutputText = computed(() => (commandResult.value ? sanitizeCommandOutput(commandResult.value.output) : ""));
// Hover tooltip for the terminal command marker strip: full command, exit
// code, duration and working directory (localized, multi-line).
const commandMarkerDetails = computed(() => commandMarkerTooltip(
  {
    command: commandMarker.command,
    exitCode: commandMarker.exitCode,
    durationMs: commandMarker.durationMs,
    elapsedMs: commandMarkerElapsed.value,
    cwd: commandMarker.cwd,
  },
  {
    command: t("terminalCommand.tooltipCommand"),
    exitCode: t("terminalCommand.tooltipExitCode"),
    duration: t("terminalCommand.tooltipDuration"),
    directory: t("terminalCommand.tooltipDirectory"),
  },
));
const connectionIdentity = computed(() => {
  // Connectionless local-terminal tab (including the restored shell): there is no connection identity to show.
  if (localUiMode.value && !connectionId.value) {
    if (isSerialMode.value) return `${t("serial.pillPrefix")} ${serialTarget.value}`;
    if (isTelnetMode.value) return `${t("telnet.pillPrefix")} ${telnetTarget.value}`;
    if (isVncMode.value) return `${t("vnc.pillPrefix")} ${vncTarget.value}`;
    return t("localTerminal.active");
  }
  const host = connection.value.host || connection.value.name || connectionId.value || "–";
  const identity = connection.value.username ? `${connection.value.username}@${host}` : host;
  const port = connection.value.port && connection.value.port !== 22 ? `:${connection.value.port}` : "";
  return `${identity}${port}`;
});
// 认证方式的本地化标签：已知方法名走 i18n，未知值原样展示（只读信息）。
const connectionAuthMethodLabel = computed(() => formatAuthMethodLabel(connectionAuthMethod.value, (method) => {
  const labels: Record<KnownAuthMethod, string> = {
    password: t("authMethodPassword"),
    "private-key": t("authMethodPrivateKey"),
    "private-key-password": t("authMethodPrivateKeyPassword"),
    agent: t("authMethodAgent"),
    auto: t("authMethodAuto"),
    none: t("authMethodNone"),
  };
  return labels[method];
}));
// 连接色染色按主题分级（light 压低 alpha 保 muted 文字 AA 对比度，P2-4）。
const toolbarStyle = computed(() => toolbarTintStyle(connection.value.color, appearance.value.colorScheme));
// 工作台分栏：Docker 固定为终端在上、容器在下（默认终端 67% / 容器 33%），
// 不受左右/上下偏好与换位影响；SFTP/指标沿用用户轴向与 splitRatio。
const paneLayout = computed(() => resolveWorkbenchPaneLayout({
  dockerOpen: dockerPanelOpen.value,
  pref: paneDirection.value,
  viewportWidth: typeof window !== "undefined" ? window.innerWidth : undefined,
  splitRatio: splitRatio.value,
  dockerSplitRatio: dockerSplitRatio.value,
}));
// 终端让宽（纯计算在 lib/paneLayout）：右侧栏按当前生效比例让出份额。
const terminalBasis = computed(() => ({
  flexBasis: terminalFlexBasis({ sftpOpen: sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }),
}));
// 纵向分栏（issue #207）：「始终上下」/ 窄屏断点/ Docker 底栏的两栏高度改由
// 容器上的 --pane-split 变量消费当前生效比例，CSS 不再钉死。
// solo（右侧栏收起）不设变量，回落既有 solo 全屏规则。
const paneSplitStyle = computed(() => paneSplitCssVar({ sftpOpen: sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }));
const orderedPaneClass = computed(() => [
  paneOrder.value === "sftp-left" ? "panes panes--reversed" : "panes",
  paneDirectionClass(paneDirection.value),
  dockerPanelOpen.value ? "panes--docker-split" : "",
  sftpPaneOpen.value ? "" : "panes--solo",
].filter(Boolean).join(" "));
// Docker 每次打开回到 1/3；拖动只在本次打开期间生效（视图本身不跨重启恢复）。
watch(dockerPanelOpen, (open) => {
  if (open) dockerSplitRatio.value = DOCKER_SPLIT_RATIO_DEFAULT;
});
// 把手提示按轴向：纵向拖的是高度、横向拖的是宽度（与 onDividerKeydown 的
// 方向键映射一致）。
const dividerResizeHint = computed(() =>
  paneLayout.value.direction === "column" ? t("dividerResizeHintHeight") : t("dividerResizeHint"),
);
// 搜索防抖与过滤态：列表排序/列布局/虚拟滚动器保留供全局列设置与偏好兼容，
// 单栏树只消费搜索草稿、清空入口与"过滤是否生效"。
const {
  sftpSearchDraft,
  clearSftpSearch,
  sftpFiltersActive,
} = useSftpListLayout({
  entries, sort, visibleColumns, sftpColumnWidths, sftpNameWidth,
  sftpSearch, sftpTypeFilter, sftpShowHidden,
  session, sudoMode,
  loadDirectory: (path?: string) => loadDirectory(path),
  persistState,
});

// 文件传输占用统一语义：ZMODEM 或 trzsz 任一持有终端流即视为 busy。
const terminalTransferBusy = computed(() => zmodemBusy.value || trzszBusy.value);
// 多选可能跨不同已展开目录（也可能被过滤/折叠隐藏）：按整棵已加载树解析，
// 顺序与树内 DFS 一致；只有已从树上消失的路径才会被剔除。
// 兜底不变量：即使旧状态/边界路径把受保护目录塞进了 selectedUris，批量
// 删除/归档/下载也只消费过滤后的条目。
const selectedEntries = computed(() =>
  filterSelectableSftpEntries(
    loadedTreeEntries.value.filter((entry) => selectedUriSet.value.has(entry.uri)),
    currentPath.value,
    pathFromUri,
  ),
);

function initialState(): WorkbenchState {
  const value = hostContext.value.workbenchState;
  return value && typeof value === "object" ? (value as WorkbenchState) : {};
}

/** 水合跨重启的布局/视图全局偏好（issue #178）：作为新工作台的初始值——
 *  字段缺省保持硬编码默认，随后 workbenchState 的会话恢复字段存在时覆盖。 */
function applyWorkbenchLayoutPrefs() {
  let prefs = null;
  try {
    prefs = sanitizeWorkbenchLayoutPrefs(JSON.parse(pluginStore.getItem(WORKBENCH_LAYOUT_KEY) ?? "null"));
  } catch {
    prefs = null;
  }
  if (!prefs) return;
  if (prefs.paneOrder) paneOrder.value = prefs.paneOrder;
  if (prefs.paneDirection) paneDirection.value = sanitizePaneDirection(prefs.paneDirection);
  if (prefs.splitRatio !== undefined) splitRatio.value = prefs.splitRatio;
  // 面板开合记的是用户最近一次的手动切换；偏好里没有时维持设置页的
  // 「默认打开」偏好（sftpPaneOpen ref 的初始值即来自它）。
  if (prefs.sftpPaneOpen !== undefined) sftpPaneOpen.value = prefs.sftpPaneOpen;
  if (prefs.visibleColumns) visibleColumns.value = sanitizeVisibleColumns(prefs.visibleColumns);
  if (prefs.sftpColumnWidths) {
    const allCols: SftpColumn[] = ["size", "modified", "owner", "group", "permissions"];
    for (const col of allCols) {
      const num = prefs.sftpColumnWidths[col];
      if (typeof num === "number") sftpColumnWidths[col] = Math.max(COLUMN_MIN_WIDTHS[col], Math.min(COLUMN_WIDTH_MAX, num));
    }
  }
  if (prefs.sftpNameWidth !== undefined) sftpNameWidth.value = Math.max(NAME_COLUMN_MIN, Math.min(NAME_COLUMN_MAX, prefs.sftpNameWidth));
  if (prefs.showHidden !== undefined) sftpShowHidden.value = prefs.showHidden;
  if (prefs.sort) sort.value = prefs.sort;
}

function restoreUiState() {
  // 两层布局持久化（issue #178 双方案合并）：1) pluginStore 全局偏好
  // （ssh-workbench-layout）先水合为新工作台初始值；2) UI 镜像
  // （ssh-workbench-ui，0.7.6-beta.1 起）作底层默认、initialState 作权威
  // 覆盖——宿主缺 workbenchState 桥时 initialState 恒空对象，镜像整组生效；
  // 桥在时每工作台状态照旧优先，镜像只补它没有的键。
  applyWorkbenchLayoutPrefs();
  const state = { ...readWorkbenchUiMirror(), ...initialState() };
  currentPath.value = typeof state.sftpPath === "string" ? normalizeRemotePath(state.sftpPath) : "/";
  splitRatio.value = typeof state.splitRatio === "number" && state.splitRatio >= 35 && state.splitRatio <= 80 ? state.splitRatio : splitRatio.value;
  paneOrder.value = state.paneOrder === "sftp-left" ? "sftp-left" : state.paneOrder === "terminal-left" ? "terminal-left" : paneOrder.value;
  // Dock panel surface：SFTP 面板默认不开（不自动列目录/不自动建 SFTP 会话，
  // dock 保持轻量）；需要文件面板时用 panel-actions 的开关手动打开（连接建立时
  // loadDirectory 本就会拉列表，打开即有数据）。
  sftpPaneOpen.value = panelSurface.value ? false : resolveSftpPaneOpen(state, sftpPaneOpen.value);
  // 本地终端工作台没有 SFTP 语义：连接less tab 的恢复态/布局镜像若带开合
  // 记录，会在 !localUiMode 门控下留下幽灵分栏（模板 v-if 只显式防了 RDP），
  // 恢复层一律收起。hostContext 在本函数之前已水合（initialize 7787）。
  if (readPluginMode(hostContext.value) === "local-terminal") sftpPaneOpen.value = false;
  // Dock panel surface：目录跟随是 SFTP 面板能力，面板一律关闭。
  // 恢复的 tab 用 per-workbench 状态；全新工作台回落全局偏好（pluginStore）。
  followDirectory.value = panelSurface.value ? false : resolveDirectoryFollow(state, loadDirectoryFollowPref());
  sudoMode.value = state.sudoMode === true && canWrite.value;
  // 一次性迁移：六列默认上线前的旧偏好重置为全开（之后用户自定义照常持久化）。
  const legacyColumns = state.visibleColumns != null && state.columnsV2 !== true;
  if (legacyColumns) visibleColumns.value = [...DEFAULT_VISIBLE_COLUMNS];
  else if (state.visibleColumns != null) visibleColumns.value = sanitizeVisibleColumns(state.visibleColumns);
  // 恢复列宽，未知列或越界用默认值兜底；state 无该列宽度时保持当前值
  // （布局偏好层已写入的值或默认值）。
  const saved = state.sftpColumnWidths || {};
  const allCols: SftpColumn[] = ["size", "modified", "owner", "group", "permissions"];
  for (const col of allCols) {
    const raw = saved[col];
    if (typeof raw === "number") sftpColumnWidths[col] = Math.max(COLUMN_MIN_WIDTHS[col], Math.min(COLUMN_WIDTH_MAX, raw));
  }
  if (typeof state.sftpNameWidth === "number") sftpNameWidth.value = Math.max(NAME_COLUMN_MIN, Math.min(NAME_COLUMN_MAX, state.sftpNameWidth));
  // 镜像/桥里若带 showHidden/sort（ssh-workbench-ui 布局子集，beta.1 起），
  // 字段存在即覆盖偏好水合值；缺字段时保持全局偏好或硬编码默认。
  if (typeof state.sftpShowHidden === "boolean") sftpShowHidden.value = state.sftpShowHidden;
  const savedSort = state.sftpSort;
  if (savedSort && (savedSort.column === "name" || savedSort.column === "size" || savedSort.column === "modified")) {
    sort.value = { column: savedSort.column, direction: savedSort.direction === "desc" ? "desc" : "asc" };
  }
  lastSequence = typeof state.terminalSequence === "number" ? state.terminalSequence : 0;
  // RDP 会话内选择的桌面尺寸（P0-2）：仅在契约门限内采用。
  const rdpSavedWidth = typeof state.rdpWidth === "number" ? state.rdpWidth : 0;
  const rdpSavedHeight = typeof state.rdpHeight === "number" ? state.rdpHeight : 0;
  rdpPersistedSize.value = rdpSavedWidth >= 640 && rdpSavedWidth <= 3840 && rdpSavedHeight >= 480 && rdpSavedHeight <= 2160
    ? { width: rdpSavedWidth, height: rdpSavedHeight }
    : null;
}

function writeWorkbenchState() {
  const uiMirror = {
    sftpPath: currentPath.value,
    followDirectory: followDirectory.value,
    sudoMode: sudoMode.value,
    splitRatio: splitRatio.value,
    paneOrder: paneOrder.value,
    sftpPaneOpen: sftpPaneOpen.value,
    visibleColumns: visibleColumns.value,
    columnsV2: true,
    sftpColumnWidths: { ...sftpColumnWidths } as Record<string, number>,
    sftpNameWidth: sftpNameWidth.value,
    sftpShowHidden: sftpShowHidden.value,
    sftpSort: { column: sort.value.column, direction: sort.value.direction },
  };
  // 布局镜像双写（issue #178）：宿主缺 workbenchState 桥时它是唯一持久层；
  // 桥在时镜像只作恢复兜底（见 restoreUiState 的合并方向）。
  writeWorkbenchUiMirror(uiMirror);
  // 同一载荷写穿跨重启的全局布局偏好（ssh-workbench-layout）：workbenchState 按
  // sessionId 键控、重启即孤儿化，这里把最近一次布局/视图状态另存一份作新工作台初始值。
  try {
    pluginStore.setItem(WORKBENCH_LAYOUT_KEY, JSON.stringify({
      paneOrder: paneOrder.value,
      paneDirection: paneDirection.value,
      splitRatio: splitRatio.value,
      sftpPaneOpen: sftpPaneOpen.value,
      visibleColumns: visibleColumns.value,
      sftpColumnWidths: { ...sftpColumnWidths },
      sftpNameWidth: sftpNameWidth.value,
      showHidden: sftpShowHidden.value,
      sort: sort.value,
    }));
  } catch {
    // 存储不可用（沙箱降级链耗尽）：会话恢复态照写，全局偏好退化为本次会话。
  }
  // 宿主桥可能尚未注入（window.dbxPlugin 未定义）或为旧宿主（无
  // workbenchState）：返回 resolved promise 而不是解引用抛 TypeError。
  // persistThrottle 虽会吞掉落盘异常，但这里仍保持 writeWorkbenchState 的
  // Promise 契约，避免每次调度都走异常路径。
  const workbenchState = window.dbxPlugin?.workbenchState;
  if (!workbenchState) return Promise.resolve();
  return workbenchState.set({
    sessionId: session.value?.sessionId,
    terminalSequence: lastSequence,
    ...uiMirror,
    sftpColumnWidths: { ...sftpColumnWidths },
    ...(rdpPersistedSize.value ? { rdpWidth: rdpPersistedSize.value.width, rdpHeight: rdpPersistedSize.value.height } : {}),
  }).catch(() => undefined);
}

/**
 * 工作台状态落盘节流：终端持续输出时 drain 每帧都会调用 persistState()——
 * 原 150 ms 防抖会在连续输出下被无限重置，导致序列号/布局永不落盘。
 * 现语义：空闲期首次调用立即落盘；连续调用合并到 150 ms；任何挂起变更自
 * 首次变更起最多 1 s 必强制落盘（不会被输出饿死）。
 */
const persistThrottle = createPersistThrottle(
  () => {
    void writeWorkbenchState();
  },
  { intervalMs: 150, maxWaitMs: 1000 },
);

function persistState() {
  persistThrottle.schedule();
}

// —— 自定义 tooltip（对齐 DBX 宿主的气泡提示）：收口在 composables/useTerminalTooltip ——
const { tooltip, tooltipBubble, hideTooltip, onTooltipOver, onTooltipOut, onTooltipFocusIn, onTooltipFocusOut } = useTerminalTooltip();

// 宿主派生的终端主题（未启用配色方案时的最终结果）：DBX 面板色 + 内置 16 色
// ANSI。作为「跟随宿主」基底，也是设置页预览的基准。
// 终端外观/主题/字体跟随：收口在 composables/useTerminalAppearance。
const {
  terminalTheme,
  hostTerminalTheme,
  applyTerminalPaddingVars,
  applyTerminalAppearance,
  updateTerminalAppearance,
  applyTerminalAppearanceTheme,
  saveTerminalAppearanceTheme,
  deleteTerminalAppearanceTheme,
  addImportedSchemes,
  removeImportedScheme,
  applyAppearance,
  registerOscColorQueryHandlers,
  hostTerminalFontFamily,
  hostFontObserver,
} = useTerminalAppearance({
  t, showNotice, appearance, terminalAppearance, terminalFontOverride, terminalFontSize,
  terminalWriteThrottle, terminalSyncOutput,
  terminal: () => terminal,
  scheduleFit,
  setTerminalFont: (family, size) => setTerminalFont(family, size),
});

// 分屏终端主题：terminalTheme() 每次调用返回新对象，直接绑定会让分屏的
// deep watcher 在主页面每次重渲染时都重设主题并重排；computed 按依赖缓存。
const splitTerminalTheme = computed(() => terminalTheme());

/** 主终端宿主级监听：分屏/收起分屏会重建 #main 插槽 DOM，绑定必须随宿主迁移。 */
function attachTerminalHost(host: HTMLElement) {
  if (terminalPasteHandler) host.addEventListener("paste", terminalPasteHandler, true);
  if (terminalWheelHandler) host.addEventListener("wheel", terminalWheelHandler, { passive: false, capture: true });
  if (terminalMouseDownHandler) host.addEventListener("mousedown", terminalMouseDownHandler);
  if (terminalMouseUpHandler) host.addEventListener("mouseup", terminalMouseUpHandler);
  ensureTerminalResizeObserver().observe(host);
}

function detachTerminalHost(host: HTMLElement) {
  if (terminalPasteHandler) host.removeEventListener("paste", terminalPasteHandler, true);
  if (terminalWheelHandler) host.removeEventListener("wheel", terminalWheelHandler, true);
  if (terminalMouseDownHandler) host.removeEventListener("mousedown", terminalMouseDownHandler);
  if (terminalMouseUpHandler) host.removeEventListener("mouseup", terminalMouseUpHandler);
  resizeObserver?.unobserve(host);
}

// 终端宿主尺寸变化（窗口缩放/命令条让位/分屏）时：fit 重排行数之外，还要
// 重读建议浮层锚点——光标行/列的像素位置随行高与滚动变化，浮层开着时
// 停在旧位置会错位或越过新边界（review 第二批：resize 主动重定位）。
function ensureTerminalResizeObserver(): ResizeObserver {
  resizeObserver ??= new ResizeObserver(() => {
    scheduleFit();
    syncSuggestionAnchorsOnSettle();
    syncGhostAnchorOnSettle();
  });
  return resizeObserver;
}

function createTerminal() {
  if (!terminalHost.value || terminal) return;
  // 用户设置优先、未设置跟随宿主（issue #31）：合成一次，字号/字体族同步生效；
  // 重连/新开终端都经此路径拿到最终值。
  const font = resolveTerminalFont(terminalFontOverride.value, {
    fontFamily: hostTerminalFontFamily(appearance.value),
    fontSize: appearance.value.terminal.fontSize,
  });
  const optionPatch = terminalOptionPatch(terminalAppearance.value.settings);
  terminalFontSize.value = font.fontSize;
  const behaviorPatch = terminalBehaviorOptionPatch(terminalBehavior.value);
  terminal = new Terminal({
    convertEol: false,
    cursorBlink: optionPatch.cursorBlink,
    // 默认细竖线（bar）：块状光标在宽字距下显得笨重，竖线更接近常规输入框观感。
    // 具体形态由外观设置覆盖（样式/闪烁/失焦态三档）。
    cursorStyle: optionPatch.cursorStyle,
    cursorInactiveStyle: optionPatch.cursorInactiveStyle,
    fontFamily: font.fontFamily,
    fontSize: font.fontSize,
    fontWeight: optionPatch.fontWeight,
    fontWeightBold: optionPatch.fontWeightBold,
    lineHeight: optionPatch.lineHeight,
    letterSpacing: optionPatch.letterSpacing,
    drawBoldTextInBrightColors: optionPatch.drawBoldTextInBrightColors,
    minimumContrastRatio: optionPatch.minimumContrastRatio,
    // 行为类选项（对标 Tabby「Terminal」页）：回滚行数默认与既有硬编码一致。
    scrollback: behaviorPatch.scrollback,
    scrollOnUserInput: behaviorPatch.scrollOnUserInput,
    wordSeparator: behaviorPatch.wordSeparator,
    ignoreBracketedPasteMode: behaviorPatch.ignoreBracketedPasteMode,
    macOptionIsMeta: behaviorPatch.macOptionIsMeta,
    // SearchAddon 的 highlight decorations 走 proposed API，缺这一项会在
    // findNext/registerDecoration 时直接抛 "allowProposedApi option"。
    allowProposedApi: true,
    theme: terminalTheme(),
  });
  fitAddon = new FitAddon();
  searchAddon = new SearchAddon();
  terminal.loadAddon(fitAddon);
  terminal.loadAddon(searchAddon);
  // 链接点击处理器自持：既做「需按住修饰键才可点」的门禁，也复刻 addon 默认的
  // 反制反向标签劫持（开空白窗 → 清 opener → 导航），不给安全打折扣。
  terminal.loadAddon(new WebLinksAddon(openTerminalLink));
  terminal.open(terminalHost.value);
  // 对标 electerm 的终端体验增强（须在 open 之后挂载）：
  // - Unicode 11 宽度表：emoji/新版 CJK 符号按两列计宽，旧宽度表会错位对齐；
  // - 内联图像（sixel + iTerm2 OSC 1337）：imgcat/chafa/htop 图表可渲染，
  //   像素上限与 electerm 同款 32MiB。
  // 连字 addon（@xterm/addon-ligatures）暂不引入：其 opentype.js 依赖走 Node
  // 内置模块，Electron（electerm）可用，本插件的沙箱 iframe 模块求值即崩，
  // 上游尚无浏览器安全构建。
  terminal.loadAddon(new Unicode11Addon());
  terminal.unicode.activeVersion = "11";
  terminal.loadAddon(new ImageAddon({ pixelLimit: 33_554_432 }));
  registerOscColorQueryHandlers();
  osc52Disposable = terminal.parser.registerOscHandler(52, (data) =>
    handleOsc52ClipboardWrite(data, (text) => {
      // 远端主动写剪贴板同样进插件视图副本，供沙箱宿主的右键粘贴降级。
      terminalCopyCache.set(text);
      return writeClipboardText(text, clipboardDeps());
    }),
  );
  // WT-2（WezTerm 对标）：OSC 通知与 SetUserVar 白名单通道。xterm 内核对未知
  // OSC 静默吞掉不渲染不回显（terminalProtocolMatrix.spec.ts 实测锚点），要
  // 消费必须在 parser 层挂 handler。xterm 6 对同一 OSC id 的多 handler 按
  // 「后注册先执行」派发、true 即截停：本组 handler 只对白名单 payload 返回
  // true，其余返回 false 落回 ImageAddon（OSC 1337 内联图像）等既有通道。
  const showTerminalOscNotice = (title: string, body: string) => {
    // 通知来自远端会话，正文原样透出（已截断/压行）；无标题时补七语来源标题，
    // 与 SFTP/下载等同面通知区分开。
    showNotice(`${title || t("terminalOsc.defaultTitle")}: ${body}`);
  };
  oscFeedDisposables = [
    // OSC 9（iTerm2 growl 风格）：payload 即正文。
    terminal.parser.registerOscHandler(9, (data) => {
      const notice = parseOsc9Notification(data);
      if (notice) showTerminalOscNotice(notice.title, notice.body);
      return notice !== null;
    }),
    // OSC 777：仅消费 `notify;TITLE;BODY` 类（kitty/urxvt 约定），其余种类不认识
    // 不消费（白名单语义）。
    terminal.parser.registerOscHandler(777, (data) => {
      const notice = parseOsc777Notify(data);
      if (notice) showTerminalOscNotice(notice.title, notice.body);
      return notice !== null;
    }),
    // OSC 1337 SetUserVar（iTerm2 shell 集成）：cwd 元数据走目录跟随优先通道，
    // cd 即刻跟随、不等提示符时刻的 OSC 7；其余 user-var 只确认消费不动作
    // （WezTerm user-var-changed 事件的等价消费面，后续可在此扩展）。
    terminal.parser.registerOscHandler(1337, (data) => {
      const userVar = parseOsc1337SetUserVar(data);
      if (!userVar) return false;
      const cwd = cwdFromUserVar(userVar.name, userVar.value);
      if (cwd) {
        userVarCwdEvent = { path: cwd, at: Date.now() };
        terminalCwd.value = cwd;
      }
      return true;
    }),
  ];
  terminal.attachCustomKeyEventHandler(handleTerminalKey);
  searchAddon.onDidChangeResults(handleSearchResults);
  // Named handler: main's WKWebView input-loss fallback shares this route.
  // Gate is local-terminal aware (a4): no SSH session AND no local shell means
  // there is no PTY to receive input.
  const routeTerminalData = (data: string) => {
    if (!session.value && !localSession.value) return;
    // 文件传输占用路由：trzsz 持有流时，传输中的输入进 filter（Ctrl+C 停传输、
    // 其余吞掉），等待协商期直接吞掉（防止杂散键入干扰 trz 握手）；zmodem 持有
    // 流时输入保持阻塞，否则走普通 PTY 键盘写入（8 字节序号前缀已封装）。
    const route = resolveTerminalInputRoute({ zmodemBusy: zmodemBusy.value, trzszBusy: trzszBusy.value });
    if (route === "trzsz") {
      feedTrzszTerminalInput(data);
      terminalDiag.swallowed += 1;
      return;
    }
    if (route === "blocked") {
      terminalDiag.swallowed += 1;
      return;
    }
    // `#` AI 命令搜索（Warp AI Command Search 同位）：等待冻结、模式内编辑与
    // 空行激活统一由共享 composable 判定；消费的字节不进 PTY。
    if (aiSearch.handleData(data)) {
      terminalDiag.swallowed += 1;
      return;
    }
    // 首帧优化：键入字节先发送，输入记录随后同步完成，bookkeeping 不阻塞发送路径。
    // 命令建议（P1-1）：行快照先于 trackPendingInput 取（\r 会清空行缓冲），
    // 之后按输入事件推进抑制门并刷新浮层。快速命令/粘贴/自动应答不走路由，
    // 天然不会触发浮层，也不会进入采集。
    sendTerminalBytes(new TextEncoder().encode(data));
    const lineBeforeInput = pendingTerminalInput;
    trackPendingInput(data);
    refreshSuggestionsAfterInput(data, lineBeforeInput);
    refreshGhostAfterInput(data);
    terminalDiag.keys += 1;
  };
  disposeInput = terminal.onData(routeTerminalData);
  // xterm.js 6.1 still drops rapid direct commits on macOS WKWebView when an
  // IME reports printable keys as keyCode=229 (#5887/#6045/#6144 upstream).
  // The adapter runs before xterm's hidden textarea listeners and routes only
  // single-byte text outside real composition through the same PTY path.
  // 右键菜单打开时暂停直写捕获（P2-8）：菜单操作不该漏进 PTY。
  disposeWebkitInputFallback = installMacWebkitInputFallback({ terminal, onData: (data) => {
    if (terminalMenuOpen.value) return;
    routeTerminalData(data);
  } });
  // 选中复制（可在设置里关闭）：选择一变化即静默写入剪贴板，不弹提示。
  // 系统剪贴板写链可能整体失败（沙箱 iframe），插件视图副本必须照记——
  // 右键粘贴在宿主读链断掉时靠它兜底。
  disposeSelectionCopy = terminal.onSelectionChange(() => {
    if (!termSelectCopy.value || !terminal?.hasSelection()) return;
    const selection = terminal.getSelection();
    terminalCopyCache.set(selection);
    void writeClipboardText(selection, clipboardDeps()).catch(() => undefined);
  });
  // 终端响铃（对标 Tabby「Terminal → Sound」）：xterm 6.x 移除了 bellStyle，
  // 只在每次响铃时抛 onBell，因此「关闭 / 视觉 / 听觉」三态只能由这里自行实现。
  disposeTerminalBell = terminal.onBell(handleTerminalBell);
  // 捕获阶段的 paste 监听：拦截 Ctrl+V 之外的所有粘贴路径（浏览器右键菜单等），
  // 统一走风险确认后再写入终端。
  terminalPasteHandler = (event) => interceptTerminalPaste(event);
  terminalWheelHandler = (event) => handleTerminalWheel(event);
  terminalMouseDownHandler = (event) => handleTerminalMouseDown(event);
  terminalMouseUpHandler = (event) => handleTerminalMouseUp(event);
  attachTerminalHost(terminalHost.value);
  if (webglEnabled.value && !wallpaperActive.value) {
    webglRenderer.value = attachWebglRenderer(terminal, () => new WebglAddon(), webglRecoveryOptions());
  }
  if (highlightEnabled.value) attachHighlightRender();
  if (actionLinksEnabled.value) attachActionLinks();
  if (isGutterActive()) attachGutterListeners();
  scheduleFit();
}

// 布局树结构变化（首次分屏/嵌套分屏/收起分屏/重置）时 #main 插槽会被 Vue
// 重建：已打开的 xterm 根节点留在被移除的旧宿主里，新宿主是空的——主窗格
// （上/左）因此空白且无法输入。把终端节点迁到新宿主，重挂宿主级监听并重新
// fit；分屏页签由各自的 SplitTerminalPane 自行创建，不受影响。
watch(
  terminalHost,
  (host, previous) => {
    if (previous && previous !== host) detachTerminalHost(previous);
    if (!host || !terminal?.element) return;
    if (terminal.element.parentElement !== host) host.appendChild(terminal.element);
    if (host === previous) return;
    attachTerminalHost(host);
    // 迁 DOM 后 WebGL/Canvas 画布不会自行重绘；刷新一次再交给 fit 校正行长。
    if (terminal.rows > 0) terminal.refresh(0, terminal.rows - 1);
    scheduleFit();
  },
  { flush: "post" },
);

// 设置开关即时生效：开=挂 renderer（失败静默回退 DOM），关=dispose。
// 偏好写入统一走 settingsPreferences（唯一落盘点；弹窗共享同一 ref）。
function setWebglEnabled(next: boolean) {
  settingsPreferences.setWebglEnabled(next);
  if (!terminal) return;
  webglRenderer.value = syncWebglRenderer(terminal, next, webglRenderer.value, () => new WebglAddon(), webglRecoveryOptions());
}

/**
 * 终端快捷键派发（对标 Tabby「Hotkeys」页）：先由 lib 侧把事件折算成规范组合串
 * （基于 event.code，Shift 恒保留为修饰键，故 Ctrl+= 与 Ctrl+Shift+= 不会塌成一个），
 * 再到用户可改写的注册表里查动作。lib 只做纯解析与匹配，命令执行留在 App。
 *
 * 默认表刻意不绑裸 Ctrl+A / Ctrl+C / Ctrl+F：这些要留给远端 shell 的
 * readline 与 SIGINT，只有 macOS 的 Cmd 系列、以及其余平台的 Ctrl+Shift 系列被占用。
 */
function handleTerminalKey(event: KeyboardEvent) {
  if (event.type !== "keydown") return true;
  // 右键菜单打开时暂停终端键盘捕获（P2-8）：按键归菜单导航，不落远端 shell
  // （macOS 直写路径已在 onData 包装层同步暂停）。不取消浏览器默认动作。
  if (terminalMenuOpen.value) return false;
  // xterm 的 false 只跳过终端处理，不会取消浏览器默认动作或冒泡。
  const consume = () => {
    event.preventDefault();
    event.stopPropagation();
    return false;
  };
  // IME 组合中不出 ghost（组合文本尚未落行；提交后的 onData 会重算）。
  if (event.isComposing || event.keyCode === 229) hideGhostSuggestion();
  // ghost 逐词接受（Ctrl+→ / Ctrl+Shift+→，macOS / Windows·Linux 同收；
  // Warp word-accept）：只注入剩余文本的首个「空白串 + 词」块。#138 批 2 起
  // 与结构化补全菜单同屏共存——菜单只占用 ↑↓/Tab/Esc，带修饰的 → 归 ghost。
  // 复查 commandRunning/传输占用（与 evaluateGhost 同门）：update 与 accept
  // 之间远端可能已开跑（回车竞态），不能把剩余字节打进运行中的命令。
  if (
    ghostMatch.value &&
    !suggestionOpen.value &&
    matchesGhostWordAccept(event.key, event) &&
    !(commandRunning.value || terminalTransferBusy.value)
  ) {
    acceptGhostSuggestionWord();
    return consume();
  }
  // ghost 整段接受（Warp 口径三键恒可用：裸 → / Ctrl+F / Ctrl+E——ghost 态
  // 光标在行尾，截获 0x06/0x05 等价 → 与 End，无 ghost 时照发远端 readline；
  // 「Tab 接受建议」开关开启时 Tab 在无菜单态也接受，菜单开着则归菜单）：
  // #138 批 2 起不再被结构化补全菜单抑制（菜单不占用裸 →；ghost 是行内灰
  // 字、菜单在光标行上/下，同屏共存）。历史建议浮层开着时无 ghost（数据分
  // 工），此判保持。未命中接受键的按键原样放行给 shell。
  if (
    ghostMatch.value &&
    !suggestionOpen.value &&
    (matchesGhostFullAccept(event.key, event) || (ghostTabAccept.value && event.key === "Tab" && !completionOpen.value)) &&
    !(event.ctrlKey && event.key === "Tab") &&
    !(commandRunning.value || terminalTransferBusy.value)
  ) {
    // 用过即散：接受 ghost 命中引导条教的键位（→ / Ctrl+F / Ctrl+E），此后不再教。
    dismissPromptHints();
    acceptGhostSuggestion();
    return consume();
  }
  // Ctrl+F 终端搜索（issue #108a，iTerm2 同位）：ghost 建议可见待接受时上方
  // 分支已把 Ctrl+F 消费为整段接受（Warp 键位模型不变）；无 ghost 才轮到
  // 搜索。传输占用（trz/zmodem 协议拥有输入流）时放行——与 resolveTerminalInputRoute
  // 同判，不打断传输（Ctrl+F 的 0x06 由传输 filter/远端自行解释）。
  if (
    event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey &&
    event.key.toLowerCase() === "f" &&
    resolveCtrlFRoute({ ghostVisible: !!ghostMatch.value, transferBusy: terminalTransferBusy.value }) === "search"
  ) {
    openTerminalSearch();
    return consume();
  }
  // 命令建议浮层开启时优先消费导航/回填键；Tab 仅在延伸/显式选中时回填，
  // 其余关闭浮层放行 shell 补全（issue #138）。结构化补全浮层（线 2）优先级
  // 更高，按键语义相同（↑↓/Tab/Enter/Esc）。
  if (
    completionOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "completion") &&
    handleCompletionKey(event)
  )
    return consume();
  if (
    suggestionOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "suggestion") &&
    handleSuggestionKey(event)
  )
    return consume();
  // Warp 式 history 面板（↑ 唤起）：面板开启时优先消费导航/回填键（↑↓ 移动、
  // Enter/Tab 回填、Esc 关闭），其余按键原样放行——焦点回到终端时打字仍会
  // 走 onData 链刷新锚点。未开启时裸 ↑ 经仲裁器唤起门打开全量视图（过滤交给
  // 面板内搜索框，不读行缓冲）；alternate 屏（vim/htop 靠 ↑ 导航）、命令运行中、
  // 传输占用、既有浮层开启时不抢 ↑，shell 原生 readline 历史在那些场景依旧可达。
  if (
    historyPanelOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "historyPanel") &&
    handleHistoryPanelKey(event)
  )
    return consume();
  if (
    event.type === "keydown" &&
    event.key === "ArrowUp" &&
    !(event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    terminalOverlayKeyOwners().arrowUp === "historyPanel"
  ) {
    openHistoryPanel();
    return consume();
  }
  // 搜索框已打开时 Esc 先关面板，不参与快捷键匹配（关闭键不可改写）。
  {
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.shiftKey && (event.key === "d" || event.key === "D")) {
      // 快速输入丢失诊断浮层（#33/#71）：三计数锁定丢失层，双击浮层关闭。
      terminalDiagVisible.value = !terminalDiagVisible.value;
      return consume();
    }
  }
  if (event.key === "Escape" && terminalOverlayKeyOwners().escape === "search") {
    closeTerminalSearch();
    return consume();
  }
  // ghost 键位胶囊菜单开着时 Esc 先收菜单，不落远端（批 4e）。
  if (event.key === "Escape" && terminalOverlayKeyOwners().escape === "ghost") {
    ghostKeycapMenuOpen.value = false;
    return consume();
  }
  // Quick Select 浮层按键（WT-1）：↑↓ 移动、Enter 复制当前项、Esc 关闭。
  // 浮层是被动的（焦点留在终端、不接管键盘），未命中的按键原样放行远端 shell。
  if (
    quickSelectOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "quickSelect") &&
    handleQuickSelectKey(event)
  )
    return consume();
  const combo = keyComboFromEvent(event);
  if (!combo) return true;
  switch (matchTerminalHotkey(terminalHotkeys.value, combo)) {
    case "search":
      openTerminalSearch();
      return consume();
    case "quick-select":
      if (quickSelectOpen.value) closeQuickSelect();
      else openQuickSelect();
      return consume();
    case "command-history":
      // ↑ 裸键之外的补充唤起（可配置键位，默认含 Ctrl+R——Warp Command Search
      // 的默认键 workspace:show_command_search）：同一 gate，开↔关 toggle；
      // 热键入口聚焦搜索框（批 3d）——热键用户要的就是 Warp Ctrl+R 式搜索流。
      // 总开关关闭（issue #169）或 alternate 屏（vim 的 Ctrl+R 是 redo，less/
      // tmux copy-mode 同理，#108 反馈二顺带修复）时不 consume：原样透传远端；
      // 其余 gate 关闭场景维持吞键（传输占用放行 ^R 会污染 zmodem/trzsz
      // 字节流，交互提示待答会把控制字符打进 OTP 输入）。
      if (commandHistoryHotkeyPassthrough({ enabled: historyPanelEnabled.value, alternateActive: terminal?.buffer.active.type === "alternate" })) return true;
      if (historyPanelOpen.value) closeHistoryPanel();
      else if (historyPanelGateOpen()) openHistoryPanel(true);
      return consume();
    case "completions":
      // 手动唤起结构化补全菜单（Warp completions 手动键同位）：只在输入面
      // 可用时开门——history 面板/终端搜索占用时不开（避免浮层叠在其下），
      // alternate 屏/命令运行中/传输占用与自动弹出路径同门；菜单已开时等于
      // 重算刷新。引擎 off 或无候选由 controller 原样关闭，无副作用。
      if (historyPanelOpen.value || searchOpen.value) return consume();
      if (commandRunning.value || terminalTransferBusy.value || terminal?.buffer.active.type === "alternate") return consume();
      // 用过即散：手动补全命中引导条教的键位（Ctrl+Space），此后不再教。
      dismissPromptHints();
      openCompletionsManually();
      return consume();
    case "ai-fix":
      // 失败命令 AI 修复（Warp Fix-with-AI 同位）：修复条在场才动作；缺席时
      // 放行远端（键位未占用任何远端常用语义，放行最诚实）。
      if (!aiFixBar.value) return true;
      void aiFixFromBar();
      return consume();
    case "ai-assist":
      // 唤起 AI 助手（Warp Agent Mode 入口同位）：开关关闭/能力缺失给可见
      // 降级提示并消费；就绪则选中文本/屏幕尾部快照发起 ask 会话。
      if (!aiBridgeOk() || !aiSettingsState.value.assist) {
        showNotice(t("aiAssist.unavailable"));
        return consume();
      }
      void aiAssistFromTerminal();
      return consume();
    case "copy":
      // 无选区时不消费：裸 Ctrl+C 仍要作为 SIGINT 发给远端。
      if (!terminal?.hasSelection()) return true;
      void copyTerminalSelection();
      return consume();
    case "paste":
      // 不取消默认动作：放行浏览器原生 paste 事件（自带真实 clipboardData，
      // 沙箱 iframe 中无需剪贴板读权限），由 terminalHost 的 capture 拦截器
      // 统一走风险确认。stopPropagation 挡住宿主/文档级快捷键；返回 false
      // 让 xterm 跳过该键，否则 Ctrl+V 会先作为 ^V 字符发给远端。
      event.stopPropagation();
      return false;
    case "select-all":
      selectAllTerminal();
      return consume();
    case "clear":
      clearTerminal();
      return consume();
    case "zoom-in":
      adjustTerminalZoom(1);
      return consume();
    case "zoom-out":
      adjustTerminalZoom(-1);
      return consume();
    case "reset-zoom":
      resetTerminalZoom();
      return consume();
    case "scroll-to-top":
      terminal?.scrollToTop();
      return consume();
    case "scroll-to-bottom":
      terminal?.scrollToBottom();
      return consume();
    default:
      return true;
  }
}

// 终端响铃：视觉/听觉两态收口在 composables/useTerminalBell。
const {
  terminalBellFlash,
  handleTerminalBell,
} = useTerminalBell({ terminalBehavior });

/**
 * 链接点击（对标 Tabby「Mouse → Require a key to click links」）：链接修饰键未按下
 * 时直接忽略，把点击还给下面的终端内容。
 *
 * 打开动作逐句复刻 addon 内置处理器（先开空窗拿句柄 → 清 opener → 改写 location），
 * 只是多了上面这道修饰键闸门：清 opener 是防「反向标签劫持」的关键，不能省。
 * 沙箱 iframe 未开 allow-popups 时 window.open 会返回 null，此时静默放弃。
 */
// 终端鼠标交互：链接/中键粘贴/缩放滚轮/点击定位光标收口在 composables/useTerminalMouse。
const {
  openTerminalLink,
  handleTerminalMiddleClick,
  handleTerminalWheel,
  handleTerminalMouseDown,
  handleTerminalMouseUp,
} = useTerminalMouse({
  terminalBehavior, terminalMenuOpen,
  closeFileMenu: () => { fileMenu.value = undefined; },
  pasteTerminal,
  adjustTerminalZoom: (delta) => adjustTerminalZoom(delta),
  terminal: () => terminal,
  getTerminalHost: () => terminalHost.value,
  session,
  sendTerminalBytes,
  zmodemBusy: () => zmodemBusy.value,
  trzszBusy: () => trzszBusy.value,
});

// 终端缩放与字体设置落地：收口在 composables/useTerminalFontZoom。
const {
  adjustTerminalZoom,
  resetTerminalZoom,
  setTerminalFont,
  applyTerminalFontSize,
  applyTerminalFontSettings,
} = useTerminalFontZoom({
  t, showNotice, appearance, terminalAppearance, terminalFontOverride, terminalFontSize,
  terminal: () => terminal,
  scheduleFit,
  hostTerminalFontFamily,
});

// 跨表面终端外观同步（tab ↔ dock 底部栏）：两个表面是独立 webview，外观三键
// 写穿宿主 storage 后对端无通知（桥无 storage 变更事件）——host 通道低频轮询
// 直读桥、localStorage 档（浏览器直连）另有原生 storage 事件即时补拍；隐藏
// 期间停轮询、复现即补。防「自写未落地被读回旧值回落」的双闸门在 lib 状态机，
// 外观与字体写入分别经两条深度 watch 进闸。
const appearanceCrossSurfaceSync = useAppearanceCrossSurfaceSync({
  reader: createCrossSurfaceStorageReader(),
  subscribe: subscribeStorageEvents,
  currentAppearance: () => terminalAppearance.value,
  currentFont: () => terminalFontOverride.value,
  adopt: (decision) => {
    // 只落地非空部分：字体未变时不重写终端字体（避免无谓的持久化回声）。
    if (decision.appearance) {
      terminalAppearance.value = decision.appearance;
      applyTerminalAppearance();
    }
    if (decision.font) setTerminalFont(decision.font.fontFamily, decision.font.fontSize);
  },
});
watch(terminalAppearance, () => appearanceCrossSurfaceSync.noteAppearanceWrite(), { deep: true });
watch(terminalFontOverride, () => appearanceCrossSurfaceSync.noteFontWrite());

// ---------------------------------------------------------------------------
// Quick Select Mode（WT-1，对标 WezTerm Quick Select）：注册表动作唤起浮层，
// 纯逻辑模块抽取可视区命中（正则/容量/去重见 lib/quickSelect.ts），这里只做
// 状态与复制接线。复制链与选中复制一致（插件视图副本 + 剪贴板桥逐级降级）。
// ---------------------------------------------------------------------------
// Quick Select：命中抽取/浮层/按键消费收口在 composables/useQuickSelect。
const {
  quickSelectOpen,
  quickSelectHits,
  quickSelectActive,
  openQuickSelect,
  closeQuickSelect,
  moveQuickSelectActive,
  copyQuickSelectHit,
  handleQuickSelectKey,
} = useQuickSelect({
  t, showNotice, showError,
  terminal: () => terminal,
  terminalMenuOpen, terminalCopyCache, clipboardDeps,
});

// ---------------------------------------------------------------------------
// 浮层键权仲裁器接线（docs/plans/IMPL_PLAN_OVERLAY_KEY_ARBITER.zh-CN.md）：
// claims 的可见性沿用各浮层既有判定（can*/gate/open 布尔），判定零迁移——
// 仲裁器只把「这个键维度此刻归谁」收口为查表。惰性求值：仅浮层分支到达时
// 才构建/采样（普通键入不读 buffer，热路径零增量）。优先级一律取
// ARBITER_PRIORITIES 表，接线处不写数字。ghost 的 → 族接受分支不查表（其
// 组合键条件即让位编码的另一半，见 overlayArbiter.ts ClaimKey 注）。
function priority(key: ClaimKey, overlay: OverlayId): number {
  return ARBITER_PRIORITIES[key][overlay] ?? 0;
}

function terminalOverlayKeyOwners(): KeyOwners {
  const panelOpen = historyPanelOpen.value;
  return resolveKeyOwners([
    // 历史面板：开着全占导航/接受/Esc；关着仅持有裸 ↑ 的唤起——gate 判
    // false 即不可见，↑ 归远端（#150 交互提示待答态由此编码）。
    ...panelClaims(panelOpen),
    // 引导条锚点：唯一锚点 claimant；其他浮层在场时其自身 overlayOpen 门
    // （shouldShowPromptHints）已置不可见，仲裁器兜底不双持。
    { overlay: "promptHints", key: "anchor", priority: priority("anchor", "promptHints"), visible: promptHintsVisible.value },
    // ghost：→ 族词块/整段接受（让位建议浮层——数据分工；运行中/传输占用
    // 在分支内复查回车竞态）与键位胶囊菜单的 Esc；Tab 接受仅在开关开启且
    // 菜单未开时可见（对 completion 让位）。→ 族分支本体不查表（其组合键
    // 条件即让位编码的另一半），Tab/Esc 维度由 handleSuggestionKey 等查表。
    { overlay: "ghost", key: "tab", priority: priority("tab", "ghost"), visible: !!ghostMatch.value && !suggestionOpen.value && ghostTabAccept.value && !completionOpen.value },
    { overlay: "ghost", key: "accept", priority: priority("accept", "ghost"), visible: !!ghostMatch.value && !suggestionOpen.value },
    { overlay: "ghost", key: "escape", priority: priority("escape", "ghost"), visible: ghostKeycapMenuOpen.value },
    // 补全/建议/quick-select/搜索：开着即声明各自键域（判定在各 handler
    // 内，查表只决定轮不轮到它）。
    ...fourKeyClaims("completion", completionOpen.value),
    ...fourKeyClaims("suggestion", suggestionOpen.value),
    { overlay: "quickSelect", key: "arrowUp", priority: priority("arrowUp", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "arrowDown", priority: priority("arrowDown", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "enter", priority: priority("enter", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "escape", priority: priority("escape", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "search", key: "escape", priority: priority("escape", "search"), visible: searchOpen.value },
  ]);
}

/** 面板 claims：开 = 导航/Enter/Tab/Esc 全占；关 = 仅裸 ↑ 的唤起门。
 *  唤起门叠加 issue #169 的功能开关：关闭后 gate 不可见，↑ 归远端 shell
 *  原生历史；热键入口（Ctrl+R，见 command-history 分支）同样受开关控制。 */
function panelClaims(panelOpen: boolean): ArbiterClaim[] {
  const gate = priority("arrowUp", "historyPanel");
  const arrowUpGate = panelOpen || (historyPanelGateOpen() && historyPanelEnabled.value);
  return [
    { overlay: "historyPanel", key: "arrowUp", priority: gate, visible: arrowUpGate },
    { overlay: "historyPanel", key: "arrowDown", priority: gate, visible: panelOpen },
    { overlay: "historyPanel", key: "enter", priority: priority("enter", "historyPanel"), visible: panelOpen },
    { overlay: "historyPanel", key: "tab", priority: priority("tab", "historyPanel"), visible: panelOpen },
    { overlay: "historyPanel", key: "escape", priority: priority("escape", "historyPanel"), visible: panelOpen },
  ];
}

/** 导航类浮层（补全/建议）的标准四键 claims：↑↓/Enter/Tab/Esc。 */
function fourKeyClaims(overlay: OverlayId, visible: boolean): ArbiterClaim[] {
  return (["arrowUp", "arrowDown", "enter", "tab", "escape"] as const).map((key) => ({
    overlay,
    key,
    priority: priority(key, overlay),
    visible,
  }));
}

/** 事件 → 键维度；不映射的按键（字母/修饰系）不参与浮层仲裁。 */
function keyEventClaimKey(event: KeyboardEvent): ClaimKey | null {
  if (event.key === "ArrowUp") return "arrowUp";
  if (event.key === "ArrowDown") return "arrowDown";
  if (event.key === "Enter") return "enter";
  if (event.key === "Tab") return "tab";
  if (event.key === "ArrowRight") return "accept";
  if (event.key === "Escape") return "escape";
  return null;
}

function overlayOwnsKey(owners: KeyOwners, event: KeyboardEvent, overlay: OverlayId): boolean {
  const dimension = keyEventClaimKey(event);
  return dimension !== null && owners[dimension] === overlay;
}

// AI 助手（Warp AI 对齐批）：设置/修复条/输出采集/推荐位在
// composables/useAiAssistant；`#` 命令搜索状态机在 useTerminalAiSearch
// （上面的 aiSearch）。三条体验都只「发送快照到宿主 AI 面板」——AI 生成内容
// 一律不回写 PTY、不代执行（执行权在人）；插件零密钥，能力缺失全链路可见
// 降级（aiBridgeOk 各调用点新鲜探测）。

// —— 行改写追踪（issue #192 失真回退采集）：键入模型在 Tab 补全 / ↑ 召回 /
// 光标移动等 shell 改写行的事件后与真实行失真（事件本身不进模型），主路径
// 的「模型行 ⊂ 屏幕行」对照从此永远失配 → 漏采。状态机在 EnterLineTracker
// （lib/terminalEnterCapture.ts）：改写前缀 = 首个改写事件时的模型行（提取
// 锚），改写后键入 = 回显活性证据（关回显的凭据输入永不上屏 → 拒采）。
const enterLineTracker = new EnterLineTracker();
// 反斜杠续行草稿（多行命令）：暂存去续行反斜杠的前段，下一行回车拼接。
let pendingContinuation = "";

function trackPendingInput(data: string) {
  if (data.includes("\u001b")) {
    // ESC 序列块（↑ 召回 / Home / End / 方向键）：readline 会改写整行，键入
    // 模型自此失真——整块跳过模型记账（原行为不变），但改写事件必须记账，
    // 否则回退采集失去全部证据（issue #192：↑ 召回改参后回车不入史）。
    enterLineTracker.noteRewrite(pendingTerminalInput);
    return;
  }
  for (const character of data) {
    if (character === "\r" || character === "\n" || character === "\u0003") {
      // 回车执行点（Ctrl+C 是清行不是执行）：远端未装 shell integration 的
      // SSH 会话没有 OSC 633 E 帧，手动键入的命令只有这里能入 commandHistory
      // （↑ history 面板的数据源）。门过滤 + 回显对照见 lib/terminalEnterCapture.ts。
      if (character !== "\u0003") captureEnterLine(pendingTerminalInput);
      pendingTerminalInput = "";
      enterLineTracker.reset();
      // Ctrl+C 放弃当前（可能未完成的续行）命令：续行草稿一并作废。
      if (character === "\u0003") pendingContinuation = "";
    }
    else if (character === "\u007f") {
      pendingTerminalInput = pendingTerminalInput.slice(0, -1);
      // 退格与远端行编辑同步（该字符模型与屏幕都不再有），证据尾巴同步削。
      enterLineTracker.noteBackspace();
    }
    else if (character === "\t") {
      // Tab 补全：shell 改写行且补全插入段不进模型（#169 只覆盖「补全后立
      // 即回车」——前缀仍是子串；继续键入即失真）。记改写事件走回退采集。
      enterLineTracker.noteRewrite(pendingTerminalInput);
    }
    else {
      // readline 行编辑控制字节（Ctrl+U 清行 / Ctrl+W 删词）：回显不可见但
      // 真实行已变，模型必须跟上，否则建议/补全对着死字符检索（#138 review
      // P0-B）。未认领的字节按原逻辑放行（可打印追加，其余忽略）——忽略的
      // 控制字节（Ctrl+A/E/L/R 等）同样是 shell 行改写，记改写事件。
      const edited = applyLineEditControlChar(pendingTerminalInput, character);
      if (edited !== null) pendingTerminalInput = edited;
      else if (character >= " ") {
        pendingTerminalInput += character;
        enterLineTracker.notePrintable(character);
      }
      else enterLineTracker.noteRewrite(pendingTerminalInput);
    }
  }
  // 行缓冲变更点（FIG wave-1 锚点）：revision 前进作废在途结果 + 防抖调度
  // （输入门未放行时只作废不调度；常规键入路径随后由
  // refreshSuggestionsAfterInput 的 request("typing") 即时冲掉防抖）。
  completionController.lineChanged();
  // 引导条的行空判定与行缓冲同步推进（pendingTerminalInput 是普通变量，
  // 引导条显隐要响应式，就在这个唯一汇点镜像一份）。
  promptHintsLineEmpty.value = pendingTerminalInput === "";
}

/** 回车行兜底采集：门过滤 + 回显对照后写入 commandHistory；与 E 帧/命令
 *  弹窗采集共用 pushTerminalCommandHistory，重复命令由环去重置顶，时间戳
 *  覆盖更新（回车先采、E 帧后到，最终值一致）。
 *  issue #192 两段增强：① 模型失真回退——Tab 补全后继续键入 / ↑ 召回后编
 *  辑的行，模型与屏幕对照必然失配，改以行改写状态（改写前缀锚 + 键入尾巴
 *  回显证据）从屏幕直接提取（extractDesyncedCommand），凭据/选择提示形态
 *  命中即拒；② 反斜杠续行合并——尾奇数反斜杠的行暂存，与下一行拼接成一条
 *  入史（与远端历史文件导入 parseShellHistoryText 同语义），Ctrl+C 作废草稿。 */
function captureEnterLine(line: string) {
  if (
    !canCaptureEnterLine(line, {
      alternateActive: terminal?.buffer.active.type === "alternate",
      transferBusy: terminalTransferBusy.value || serialUploadBusy.value,
      shellCommandActive: commandMarker.active,
    })
  ) {
    return;
  }
  const echo = readEchoTextBeforeCursor();
  // 以屏幕回显提取的命令入史（issue #169）：模型行只含键入前缀，Tab 补全/
  // 历史展开后的真实命令（`cd blen` → `cd blender/`）只在回显里；提取失败
  // （屏幕不可测）回落模型行，不劣于旧行为。模型失配时走失真回退提取
  // （issue #192），两级都拒采则放弃（宁可漏采，不错采）。
  const extracted =
    echoConfirmsLine(echo, line)
      ? extractEchoedCommand(echo, line) ?? line
      : extractDesyncedCommand(echo, enterLineTracker.snapshot());
  if (!extracted) return;
  const { entry, pending } = acceptCommandWithContinuation(pendingContinuation, extracted);
  pendingContinuation = pending;
  if (entry) pushTerminalCommandHistory(entry);
}

/**
 * 读光标前（含向上折行链）的屏幕文本，作回显对照源。拼接含提示符前缀
 * （prompt 边界不可知），对照语义因此是「模型行是屏幕文本的子串」而非全等；
 * buffer 不可测（测试/渲染器未就绪）返回 null，由 echoConfirmsLine 放行。
 */
function readEchoTextBeforeCursor(): string | null {
  if (!terminal) return null;
  try {
    const buffer = terminal.buffer.active;
    if (buffer.type !== "normal") return null;
    const rowY = cursorAbsoluteRow(buffer);
    const cursorRow = buffer.getLine(rowY);
    if (!cursorRow) return null;
    const parts: string[] = [];
    let text = "";
    for (let x = 0; x < buffer.cursorX; x += 1) text += cursorRow.getCell(x)?.getChars() ?? "";
    parts.push(text);
    let row = cursorRow;
    while (row.isWrapped && rowY - parts.length >= 0) {
      const previous = buffer.getLine(rowY - parts.length);
      if (!previous) break;
      row = previous;
      let wrapped = "";
      for (let x = 0; x < terminal.cols; x += 1) wrapped += row.getCell(x)?.getChars() ?? "";
      parts.unshift(wrapped);
    }
    return parts.join("");
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// 命令输入建议浮层（P1-1）：采集→抑制门→检索→定位→按键消费。
// 采集走三条真实来源：① 命令弹窗/命令条执行（已入 commandHistory）；
// ② OSC 633 shell-integration E 帧（applyCommandMarker）；③ 回车行兜底
// 采集（trackPendingInput 的 \r 分支 → captureEnterLine，覆盖无 shell
// integration 的 SSH/本地会话——远端没装 integration 时 E 帧不存在，
// 手动键入的命令此前从不入史）。兜底路径的门过滤 + 回显对照（凭据类
// 关回显输入绝不入史）在 lib/terminalEnterCapture.ts。Expect/OTP 自动
// 应答由 sidecar 直接注入 PTY，与 onData 用户输入不同源，永不入历史。
// ---------------------------------------------------------------------------

/** 仅收历史建议浮层（不 dismiss 补全调度）：历史建议无匹配的回落分支用。
 *  worker 模式下本次键入的补全响应还在途——dismiss 会把 revision 顶掉，
 *  响应到达时被三重 guard 丢弃，表现为"敲错→退格→再输入，补全不再弹出"
 *  （每个键入都重复 dismiss → 永久失效，直到命中历史建议才恢复）。 */

// ---------------------------------------------------------------------------
// 终端行内 ghost 自动建议（对标 Warp/fish autosuggest）：门状态机与前缀扩展
// 匹配在 lib/terminalGhostSuggest.ts，这里只做三件事——xterm buffer 行尾采样、
// overlay 锚点计算、接受时向 PTY 注入剩余字节（等价用户键入，无协议改动）。
// 渲染选 overlay DOM 而非 xterm decoration：ghost 逐键刷新，decoration 注册/
// 销毁生命周期重；overlay 与既有 action-link-hint 同机制，零 buffer 侵入，
// 不影响选区/搜索/屏幕阅读器。
// ---------------------------------------------------------------------------

/** 设置开关（SettingsDialog 自治持久化，经 update:ghost-suggest 即时上抛）。 */
// ghost 行内建议：状态机/门/接受语义收口在 composables/useGhostSuggest。
const {
  ghostEnabled,
  ghostMatch,
  ghostAnchor,
  setGhostEnabled,
  hideGhostSuggestion,
  resetGhostSuggestion,
  refreshGhostAfterInput,
  acceptGhostSuggestion,
  acceptGhostSuggestionWord,
  syncGhostAnchorOnSettle,
} = useGhostSuggest({
  terminal: () => terminal,
  sendTerminalBytes,
  getPendingTerminalInput: () => pendingTerminalInput,
  appendToPendingTerminalInput: (data) => { pendingTerminalInput += data; },
  commandRunning, terminalTransferBusy, commandHistory, quickCommands,
  suggestionOpen, historyPanelOpen, completionController,
  suggestionMinCharsState, suggestionMaxCharsState,
  // 终端交互提示待答（issue #150 及其反馈扩展）：询问输入/选择处不出 ghost。
  isInteractivePromptPending: () => interactivePromptGatePending(),
  readTerminalCellFrame,
});
function sendTerminalBytes(data: Uint8Array) {
  // 串口会话优先：B1 主路径走 `serial/terminal/in/{id}` 二进制写通道（专用
  // 有序队列 + Stdin 流标签帧）；宿主报错（未知通道/旧 sidecar）一次性降级
  // serial/write JSON 兼容路径。
  if (serialSession.value) {
    // 上传进行中吞掉键入：X/Y/ZMODEM 的 ACK/NAK/CAN 控制字符窗口内，
    // 用户字节会污染协议流（进度状态在终端 overlay 上提示"传输中"）。
    // B1 互斥：二进制写帧与 JSON 键入共用这道前端闸门；sidecar 侧另有
    // 二进制帧拒收后盾。
    if (serialUploadBusy.value) return;
    if (serialBinaryInput.value) {
      serialInputQueue.enqueue(`serial:${serialSession.value.sessionId}`, normalizeTerminalInputBytes(data));
      return;
    }
    const dataBase64 = window.dbxPlugin.encodeBase64(normalizeTerminalInputBytes(data));
    void window.dbxPlugin.invoke("serial/write", { sessionId: serialSession.value.sessionId, dataBase64 }).catch((cause) => showError(cause, "terminal"));
    return;
  }
  // Telnet 会话优先：同一终端视图同一时刻只挂一个会话（SSH/本地/Telnet 互斥），
  // telnet: 前缀在队列 send 回调里拆成 telnet/terminal/in/{id} 通道。
  if (telnetSession.value) {
    terminalInputQueue.enqueue(`telnet:${telnetSession.value.sessionId}`, normalizeTerminalInputBytes(data));
    return;
  }
  // Ignore typing while an SSH session is reconnecting. Keeping bytes in a
  // stale session queue can replay partial commands after the browser returns.
  if (!localSession.value && terminalState.value !== "connected") return;
  const sessionId = localSession.value?.sessionId ?? session.value?.sessionId;
  if (!sessionId) return;
  terminalInputQueue.enqueue(sessionId, normalizeTerminalInputBytes(data));
}

function scheduleFit() {
  window.clearTimeout(resizeTimer);
  resizeTimer = window.setTimeout(() => {
    if (!terminal || !fitAddon || !terminalHost.value?.clientWidth || !terminalHost.value.clientHeight) return;
    try {
      fitAddon.fit();
      // trzsz 进度条按终端列宽渲染（filter 内部文本进度条虽未启用，列宽保持同步）。
      setTrzszTerminalColumns(terminal.cols);
      const sshSessionId = activeTerminalSessionId || session.value?.sessionId;
      if (telnetSession.value) {
        // Telnet NAWS：sidecar 发 SB NAWS 子协商，尽力而为。
        void window.dbxPlugin.notify("telnet/resize", { sessionId: telnetSession.value.sessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
      } else if (localSession.value) {
        void window.dbxPlugin.notify("local/terminal/resize", { sessionId: localSession.value.sessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
      } else if (sshSessionId) {
        void window.dbxPlugin.notify("ssh/terminal/resize", { sessionId: sshSessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
      }
    } catch {
      // The iframe can briefly be detached while DBX switches tabs.
    }
  }, 20);
}

/** Synchronous fit for spawn-time sizing: returns the real panel cols/rows.
 * Sessions MUST be spawned at this size — spawning at xterm's 80×24 default
 * and fitting right after makes ConPTY/PSReadLine repaint the prompt at the
 * new geometry (a second prompt line with a blank gap), which users read as
 * broken rendering. fit() throws when the iframe is detached mid-tab-switch;
 * fall back to the current terminal dims in that case. */
function fitTerminalDimensions(): { cols: number; rows: number } {
  if (!terminal || !fitAddon) return { cols: 120, rows: 32 };
  try {
    if (!terminalHost.value?.clientWidth || !terminalHost.value.clientHeight) {
      return { cols: terminal.cols, rows: terminal.rows };
    }
    fitAddon.fit();
    setTrzszTerminalColumns(terminal.cols);
  } catch {
    // The iframe can briefly be detached while DBX switches tabs.
  }
  return { cols: terminal.cols, rows: terminal.rows };
}

function nextAnimationFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

/** Spawn-size gate for local shells. ConPTY repaints the whole screen buffer
 * on every changed resize, and one racing the shell's opening output renders
 * as a duplicated prompt/banner. On a dock "+" fresh tab the webview can
 * still be settling when the start request fires — zero-size host (fit then
 * falls back to xterm's default 80×24) or a pending web font swap (changes
 * cell metrics) — so the spawn size would differ from the panel's steady
 * state and the later real fit becomes a cross-geometry repaint. Poll frames
 * until two consecutive fits agree on a laid-out host; after maxWaitMs give
 * up with the last sample (never blocks boot indefinitely). */
async function settledTerminalDimensions(maxWaitMs = 1500): Promise<{ cols: number; rows: number }> {
  const deadline = Date.now() + maxWaitMs;
  try {
    // Wait out the font swap with half the budget so the sample loop below
    // keeps its share; document.fonts can be missing in older webviews.
    await Promise.race([
      document.fonts?.ready ?? Promise.resolve(),
      new Promise((resolve) => window.setTimeout(resolve, maxWaitMs / 2)),
    ]);
  } catch {
    // Font readiness is best-effort; the consecutive-fit samples still guard.
  }
  let last: { cols: number; rows: number } | null = null;
  while (Date.now() < deadline) {
    await nextAnimationFrame();
    const host = terminalHost.value;
    if (!host?.clientWidth || !host.clientHeight) {
      // Not laid out yet — a fit here would silently measure nothing.
      last = null;
      continue;
    }
    const dims = fitTerminalDimensions();
    if (last && dims.cols === last.cols && dims.rows === last.rows) return dims;
    last = dims;
  }
  return last ?? fitTerminalDimensions();
}

/** The terminal's on-screen cursor (1-based) at spawn time. ConPTY's opening
 * CSI 6n asks where the cursor is and starts the shell there
 * (PSEUDOCONSOLE_INHERIT_CURSOR), so the sidecar must answer with this — and
 * the answer decides where the banner/prompt is drawn: the viewport size (what
 * the sidecar used to send) inherits the last row and re-serializes the screen
 * mid-banner, which is the stray first glyph users see. cursorY is
 * viewport-relative (0 = the row at baseY) and cursorX is the column, so both
 * need the +1 to become the 1-based position conhost expects. */
function terminalCursorPosition(): { cursorRow: number; cursorCol: number } {
  const buffer = terminal?.buffer.active;
  if (!buffer) return { cursorRow: 1, cursorCol: 1 };
  return { cursorRow: buffer.cursorY + 1, cursorCol: buffer.cursorX + 1 };
}

function stopCommandMarkerTick() {
  if (commandMarkerTimer) {
    window.clearInterval(commandMarkerTimer);
    commandMarkerTimer = 0;
  }
  commandMarkerElapsed.value = null;
}

function startCommandMarkerTick(startedAt: number) {
  stopCommandMarkerTick();
  commandMarkerTimer = window.setInterval(() => {
    commandMarkerElapsed.value = runningCommandElapsedMs(startedAt, Date.now());
  }, 1000);
  commandMarkerElapsed.value = runningCommandElapsedMs(startedAt, Date.now());
}

function resetCommandMarker() {
  commandMarkerParser.reset();
  // 提示符换行状态同属会话流：不把上一会话的行首游标与不完整序列带进新流。
  promptNewline.reset();
  stopCommandMarkerTick();
  // 会话切换/断开连带复位 SetUserVar cwd 优先通道：不把上一会话的目录裁决
  // 带进新会话（OSC 7 回落立即恢复权威）。
  userVarCwdEvent = null;
  commandMarker.installed = false;
  commandMarker.active = false;
  commandMarker.command = "";
  commandMarker.exitCode = null;
  commandMarker.durationMs = null;
  commandMarker.cwd = "";
  commandMarker.startedAt = null;
  // 会话切换/断开：建议浮层与抑制门锁存一并复位（P1-1）；ghost 门同步复位。
  resetSuggestionsForSession();
  closeHistoryPanel();
  resetGhostSuggestion();
  // AI 助手状态随会话复位（Warp AI 对齐批）：# 模式/修复条/推荐卡/输出采集
  // 缓冲（解码器重建，避免跨会话的多字节残态）。
  aiSearch.reset();
  resetPromptHintsQuiet();
  resetAiFixForSession();
}

function applyCommandMarker(updates: Osc633StreamUpdates) {
  if (updates.shellIntegrationInstalled !== undefined) commandMarker.installed = updates.shellIntegrationInstalled;
  if (updates.commandActive !== undefined) commandMarker.active = updates.commandActive;
  if (updates.command !== undefined) commandMarker.command = updates.command;
  // A fresh "E" frame starts a new command: clear the previous result so the
  // strip flips to the running state. The "A" frame's lastExitCode=null reset
  // is ignored on purpose — the finished result stays visible at the prompt
  // until the next command starts.
  // 最近命令收集：仅 E 帧写 updates.command，故以其存在为准——不能挂在
  // commandActive 上，E 与 D 常在同一段输出里（命令快进快出时合并后的终值
  // 是 false），挂在 phase 上会漏采（与 D/A 退出码覆写同源的合并陷阱）。
  if (updates.command !== undefined && updates.command.trim()) {
    const command = updates.command.trim();
    lastTerminalCommand.value = command;
    // 命令历史采集（P1-1）：shell integration 会话（本地 + SSH）把 E 帧命令行
    // 写入 commandHistory 环形；持久化沿用 SECRET_LIKE/长度过滤，命令执行中
    // 顺带收起浮层。与 onData 回车行采集去重由 pushCommandHistory 保证。
    pushTerminalCommandHistory(command);
    if (isLocalMode.value && command !== localRecentCommands.value[0]) {
      localRecentCommands.value = [command, ...localRecentCommands.value.filter((c) => c !== command)].slice(0, 20);
    }
    closeSuggestions();
    // 命令开始执行：history 面板一并收起（面板开启时 ↑↓/Enter 均被吞键，
    // 正常到不了这里；E 帧旁路（粘贴多行/快速命令）执行时兜底关闭）。
    closeHistoryPanel();
    // AI 修复（Warp AI 对齐批）：E 帧 = 新命令边界——快进快出命令的
    // commandActive 合并终值是 false（E/C/D 同 chunk），清零不能挂在它上。
    // 采集缓冲清零 + 旧修复条让位 + 推荐卡撤下。
    beginAiCommand();
  }
  if (updates.commandActive === true) {
    commandMarker.exitCode = null;
    commandMarker.durationMs = null;
    // A fresh "E" frame also starts the 1s tick so the marker strip shows a
    // live duration while the command runs; the final durationMs from the
    // "D" frame takes over once the tick stops.
    commandMarker.startedAt = Date.now();
    startCommandMarkerTick(commandMarker.startedAt);
  } else if (updates.commandActive === false) {
    stopCommandMarkerTick();
  }
  if (updates.lastExitCode !== undefined && updates.lastExitCode !== null) commandMarker.exitCode = updates.lastExitCode;
  if (updates.lastCommandDuration !== undefined) commandMarker.durationMs = updates.lastCommandDuration;
  // 富元数据采集（批 4d）：D 帧的退出码/时长归档到命令 → 元数据映射（Warp
  // command search 的时长/退出码列）；合并写（两值分帧到达），随时间映射同款
  // 存续集淘汰 + 持久化。
  if ((updates.lastExitCode !== undefined && updates.lastExitCode !== null) || updates.lastCommandDuration !== undefined) {
    const command = commandMarker.command;
    if (command) {
      recordCommandExecutionMeta(command, {
        exitCode: updates.lastExitCode !== undefined && updates.lastExitCode !== null ? updates.lastExitCode : undefined,
        durationMs: updates.lastCommandDuration ?? undefined,
      });
    }
  }
  if (updates.cwd !== undefined) {
    commandMarker.cwd = updates.cwd;
    if (updates.cwd) terminalCwd.value = updates.cwd;
    if (isLocalMode.value) localLastCwd.value = updates.cwd;
  }
}

function writeTerminalOutput(data: Uint8Array) {
  const now = Date.now();
  for (const path of directoryParser.push(data)) {
    // WT-2：SetUserVar cwd 优先通道在位且刚更新时，提示符 OSC 7 的旧值不回踩
    // （裁决见 shouldOsc7FollowOverrideUserVarCwd；无该通道时行为与从前一致）。
    if (!shouldOsc7FollowOverrideUserVarCwd(userVarCwdEvent, path, now)) continue;
    terminalCwd.value = path;
  }
  // AI 修复输出采集（Warp AI 对齐批）：解码与 OSC 解析同流，命令执行期
  // （active）的输出文本进尾部环形缓冲；快进快出的命令（E/C/D 同一 chunk）
  // 由 D 帧判定一并采集——wasActive 覆盖长输出的中间 chunk，D 帧 chunk 覆盖
  // 尾段正文。剥 ANSI 后入缓冲——633 帧噪声不进 AI 快照。
  markTerminalOutputActive();
  const aiChunkWasActive = commandMarker.active;
  const aiChunkUpdates = commandMarkerParser.push(data);
  applyCommandMarker(aiChunkUpdates);
  // AI 修复输出采集（Warp AI 对齐批）：解析（E 帧清零缓冲）之后的同 chunk
  // 正文照采——执行期 chunk 靠 active 判定，快进快出命令靠 D 帧 chunk 判定。
  captureOutputChunk(data, aiChunkWasActive || commandMarker.active || aiChunkUpdates.lastExitCode != null);
  maybeOfferAiFix(aiChunkUpdates);
  // 提示符换行修复（t8y2/dbx#10750）：OSC 7 帧前按需补 CRLF。备用屏
  // （vim/top 绝对定位重绘）与 zmodem/trzsz 传输期间只跟踪不插入——二进制
  // 显示流与全屏应用的画面不容忍多余换行。directoryParser/采集链仍吃原始
  // 字节：变换器只做插入，帧本身原样保留。
  const suppressPromptNewline = terminalTransferBusy.value || terminal?.buffer.active.type === "alternate";
  terminalWriteThrottle.write(promptNewline.push(data, { suppressInsert: suppressPromptNewline }));
}

/**
 * Terminal output dispatch: ZMODEM owns the stream while busy; otherwise the
 * frame feeds the trzsz filter, which passes it through to the terminal and
 * watches for the remote `::TRZSZ:TRANSFER:` announce to take over exactly
 * one transfer. Plain frames thus reach the terminal untouched.
 */
function dispatchTerminalOutput(data: Uint8Array) {
  if (zmodemBusy.value) {
    writeTerminalOutput(data);
    return;
  }
  const announce = detectTrzszAnnounceFromBytes(data);
  if (announce) handleTrzszAnnounce(announce);
  ensureTrzszFilter().processServerOutput(data);
}

// ZMODEM 上传/下载：sentry/检测/进度/落盘/收尾收口在 composables/useZmodem。
const {
  zmodemState,
  zmodemSpeed,
  zmodemFileIndex,
  zmodemFileCount,
  zmodemBusy,
  zmodemPercent,
  zmodemStatusLabel,
  resetZmodemSentry,
  finishZmodemUpload,
  cancelZmodemUpload,
  onZmodemInput,
  consumeFrameViaZmodem,
} = useZmodem({
  t, showNotice, showError, connected,
  terminal: () => terminal,
  sendTerminalBytes, dispatchTerminalOutput,
  loadDirectory: (path?: string) => loadDirectory(path),
  // 懒引用：save 链/trzszBusy 都在本调用点之后才声明（TDZ），闭包只在传输时执行。
  receiveAllowed: () => !trzszBusy.value,
  // 经典 rz 提示（issue #171）：选文件必须在用户手势内触发隐藏 input click；
  // 取消/超时后收起提示横幅。
  pickUploadFiles: () => zmodemInput.value?.click(),
  dismissNotice: () => { noticeOpen.value = false; noticeActions.value = []; },
  probeLocalCapabilities: () => probeLocalCapabilities(),
  saveHostFile: (chunks, fileName) => saveHostFile(chunks, fileName),
  loadDownloadUseDefaultDir, askDownloadTarget, loadDownloadDir, resolveDownloadConflictFor,
  applyChosenDirAsDefault,
  revealTransferTarget: (path) => revealTransferTarget(path),
});

// trzsz (trz / tsz)：状态/进度/落盘收口在 composables/useTrzsz（终端流集成点在上方）。
const {
  trzszInput,
  trzszPhase,
  trzszDirection,
  trzszFileName,
  trzszFileIndex,
  trzszFileCount,
  trzszPercent,
  trzszSpeed,
  trzszMessage,
  trzszBusy,
  trzszOverlayVisible,
  trzszStatusLabel,
  ensureTrzszFilter,
  handleTrzszAnnounce,
  cancelTrzszTransfer,
  teardownTrzsz,
  chooseTrzszUpload,
  onTrzszPickInput,
  onTrzszPickCancel,
  feedTrzszTerminalInput,
  setTrzszTerminalColumns,
} = useTrzsz({
  t, showNotice, showError, session, canWrite, zmodemBusy, terminalMenuOpen,
  terminal: () => terminal,
  writeTerminalOutput, sendTerminalBytes,
  probeLocalCapabilities: () => probeLocalCapabilities(),
  saveHostFile: (chunks, fileName) => saveHostFile(chunks, fileName),
  loadDownloadUseDefaultDir, askDownloadTarget, loadDownloadDir, resolveDownloadConflictFor,
  applyChosenDirAsDefault,
  revealTransferTarget: (path) => revealTransferTarget(path),
});

// 协议帧缓冲（local/telnet/serial）：收口在 composables/useProtocolFrameBuffers。
const {
  localLastSequence,
  localPendingFrames,
  localReplayInFlight,
  localReplayNoProgress,
  markLocalExited,
  drainLocalTerminalFrames,
  telnetLastSequence,
  telnetPendingFrames,
  telnetReplayInFlight,
  telnetReplayNoProgress,
  markTelnetClosed,
  drainTelnetFrames,
  serialLastSequence,
  serialPendingFrames,
  serialReplayInFlight,
  serialReplayNoProgress,
  serialUnknownStreamFrames,
  markSerialClosed,
  drainSerialFrames,
  requestLocalReplay,
} = useProtocolFrameBuffers({
  t,
  dispatchTerminalOutput, showNotice,
  localSession, localState, localExitCode, stopCommandMarkerTick,
  telnetSession, telnetState, telnetError,
  serialSession, serialState, serialError,
});

function handleBinary(event: DbxPluginBinaryEvent) {
  if (event.channel.startsWith("local/terminal/out/")) {
    const localId = event.channel.slice("local/terminal/out/".length);
    if (!localSession.value || localId !== localSession.value.sessionId) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (payload.length < 9) return;
    const sequence = readU64(payload, 1);
    if (sequence <= localLastSequence.value) return;
    localPendingFrames.set(sequence, { stream: payload[0], data: payload.slice(9) });
    drainLocalTerminalFrames();
    return;
  }
  if (event.channel.startsWith("telnet/terminal/out/")) {
    // Telnet 输出帧与 local/SSH 同形（9 字节 TerminalFrame 前缀），独立
    // sequence/pending 状态避免与 SSH/本地流互染。
    const telnetId = event.channel.slice("telnet/terminal/out/".length);
    if (!telnetSession.value || telnetId !== telnetSession.value.sessionId) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (payload.length < 9) return;
    const sequence = readU64(payload, 1);
    if (sequence <= telnetLastSequence.value) return;
    telnetPendingFrames.set(sequence, { stream: payload[0], data: payload.slice(9) });
    drainTelnetFrames();
    return;
  }
  if (event.channel.startsWith("serial/terminal/out/")) {
    // 串口输出帧与 local/SSH/Telnet 同形（9 字节 TerminalFrame 前缀，stream
    // 恒为 Stdout），独立 sequence/pending 状态避免与其它会话流互染。
    const serialId = event.channel.slice("serial/terminal/out/".length);
    if (!serialSession.value || serialId !== serialSession.value.sessionId) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (payload.length < 9) return;
    // B1 解码契约：未知流标签（> Stdin=3）一律静默丢帧并计数，不断连。
    if (!isKnownStreamTag(payload[0])) {
      serialUnknownStreamFrames.value += 1;
      return;
    }
    const sequence = readU64(payload, 1);
    if (sequence <= serialLastSequence.value) return;
    serialPendingFrames.set(sequence, { stream: payload[0], data: payload.slice(9) });
    drainSerialFrames();
    return;
  }
  if (event.channel.startsWith("vnc/frame/")) {
    // VNC 帧补丁（44 字节头 + RGBA payload，sidecar 单调递增 sequence）：
    // 会话隔离后交给 useVncSession 的帧入口，由 VncSurface 解码 + rAF 合帧；
    // 乱序帧在组件内丢弃。
    const vncId = event.channel.slice("vnc/frame/".length);
    if (!vncSession.value || vncId !== vncSession.value.sessionId) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    handleVncFrame(vncId, payload);
    return;
  }
  if (event.channel.startsWith("rdp/frame/")) {
    // RDP 帧补丁：与 vnc/frame 同一 44 字节 patch 头 + RGBA（解码复用
    // vncFrame），sequence 跨重连单调。首个桌面帧同时把状态推到 running。
    const rdpId = event.channel.slice("rdp/frame/".length);
    if (!rdpSession.value || rdpId !== rdpSession.value.sessionId) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (rdpState.value.state !== "running") rdpState.value = { ...rdpState.value, state: "running" };
    rdpSurface.value?.acceptFrame(payload);
    return;
  }
  const sessionId = activeTerminalSessionId || session.value?.sessionId;
  if (sessionId && event.channel === `ssh/terminal/out/${sessionId}`) {
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (payload.length < 9) return;
    const sequence = readU64(payload, 1);
    if (sequence <= lastSequence) return;
    pendingTerminalFrames.set(sequence, { stream: payload[0], data: payload.slice(9) });
    drainTerminalFrames();
    return;
  }
  const taskId = event.channel.startsWith("sftp/download/") ? event.channel.slice("sftp/download/".length) : "";
  const waiter = downloadChunkWaiters.get(taskId);
  if (!waiter) return;
  const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
  if (payload.length < 8 || readU64(payload, 0) !== waiter.offset) return;
  window.clearTimeout(waiter.timer);
  downloadChunkWaiters.delete(taskId);
  waiter.resolve(payload.slice(8));
}

function drainTerminalFrames() {
  let frame = pendingTerminalFrames.get(lastSequence + 1);
  while (frame) {
    pendingTerminalFrames.delete(lastSequence + 1);
    lastSequence += 1;
    if (frame.stream === 2) {
      const state = new TextDecoder().decode(frame.data);
      if (state === "directory-tracking-unavailable") {
        followDirectory.value = false;
        directoryTrackingSupported.value = false;
        showNotice(t("directoryTrackingUnavailable"));
      } else {
        terminalState.value = "disconnected";
        terminalError.value = state === "ssh-transport-disconnected" ? t("transportDisconnected") : state || t("disconnected");
        if (state === "ssh-transport-disconnected" && session.value?.sessionId) {
          terminalTransportWatch.reportUnavailable(session.value.sessionId);
        }
      }
    } else {
      consumeFrameViaZmodem(frame.data);
    }
    frame = pendingTerminalFrames.get(lastSequence + 1);
  }
  persistState();
  // A stalled gap must not grow the pending map without bound: once the
  // buffer overshoots, drop it and let the replay re-deliver everything
  // after the last in-order sequence.
  if (pendingTerminalFrames.size > TERMINAL_PENDING_FRAME_LIMIT) {
    pendingTerminalFrames.clear();
  }
  const firstPending = Math.min(...pendingTerminalFrames.keys());
  if (Number.isFinite(firstPending) && firstPending > lastSequence + 1 && !replayInFlight && session.value) {
    const holeAt = lastSequence;
    replayInFlight = true;
    // A session-gone replay failure must resync and reconnect: the hole can
    // never be filled (the session object is gone server-side), so the
    // ssh-transport-disconnected state frame stuck behind it would otherwise
    // freeze the drain forever and the tab keeps claiming "connected".
    let sessionGone = false;
    void window.dbxPlugin.invoke<ReplayResult>("ssh/terminal/replay", { sessionId: session.value.sessionId, afterSequence: lastSequence })
      .then((result) => {
        if (!result.complete) {
          terminalState.value = "error";
          terminalError.value = t("sessionUnrecoverable");
          return;
        }
        // The replay returned healthy but the hole below firstPending is still
        // there: those frames are gone for good (e.g. sequence numbering
        // restarted across a reconnect). Retry once more, then resync the
        // cursor past the hole — dropping the missing prefix beats spinning
        // this replay loop forever and freezing the workbench.
        if (lastSequence === holeAt) {
          replayNoProgress += 1;
          if (replayNoProgress >= 3) {
            lastSequence = firstPending - 1;
            replayNoProgress = 0;
          }
        } else {
          replayNoProgress = 0;
        }
      })
      .catch((cause) => {
        if (isSessionGoneError(cause) && firstPending > lastSequence) {
          sessionGone = true;
          lastSequence = firstPending - 1;
          replayNoProgress = 0;
          return;
        }
        showError(cause, "terminal");
      })
      .finally(() => {
        replayInFlight = false;
        drainTerminalFrames();
        // Run after the resync drain above delivered the buffered disconnect
        // state frame (it flips the state off "connected"); the ladder then
        // picks the reconnect up unless one is already pending.
        if (sessionGone && !reconnectPending.value && !disposed) scheduleSessionReconnect();
      });
  }
}

// 本地终端输出与 SSH 同一帧协议（stream + u64 sequence），复用乱序重组与
// 空洞补发；State 帧到来即落退出态。补发不完整只 resync 到缓冲尾部——
// 环形缓冲淘汰不可恢复，但活会话不能被误判成已退出。

// 传输断开/会话被杀的统一入口：有界退避自动重连，梯子耗尽才落到
// disconnected 终态等待手动重连。
// 重连梯子：收口在 composables/useSessionReconnect。
const {
  reconnectPending,
  reconnectCountdown,
  reconnectWasPending,
  reconnectTimer,
  reconnectAttempt,
  reconnectCountdownTimer,
  reconnectNextAt,
  reconnectDelayMs,
  scheduleSessionReconnect,
} = useSessionReconnect({
  t,
  terminalState, terminalError,
  isDisposed: () => disposed,
  openSession,
  requestHostReopenConnection,
});
const terminalTransportWatch = useTerminalTransportWatch({
  t,
  terminalState,
  terminalError,
  session,
  resetInputQueue: () => terminalInputQueue.reset(),
  probeTransport: (sessionId) => window.dbxPlugin.invoke<ReplayResult>(
    "ssh/terminal/replay",
    { sessionId, afterSequence: lastSequence },
    { timeoutMs: TERMINAL_TRANSPORT_PROBE_TIMEOUT_MS },
  ),
  attachSession,
});
// 连接卡片四态与手动重连入口：收口在 composables/useConnectCard（会话主干
// openSession/closeSession/宿主重开留守 App.vue，经 options 注入）。
const { connectCardState, connectLogEntries, reconnect, cancelConnect, startConnect, reconnectNow } = useConnectCard({
  t,
  terminalState,
  connectCancelled, connectSucceeded, connectLog,
  reconnectPending, reconnectAttempt, reconnectTimer,
  getTerminal: () => terminal,
  resetGutterTimestamps,
  requestHostReopenConnection,
  openSession,
  closeSession,
});

// 宿主推给 webview 的 connection 上下文缺 external_config.protocol，向
// sidecar 查询 lifecycle 暂存的权威协议；宿主尚未推送该连接时返回空串。
// sidecar 现在对已登记的 SSH 连接也回答 "ssh"（backend/src/main.rs 的
// connection/protocol），所以 SSH 连接首个查询即可命中——这个窗口只用于吸收
// 恢复态标签更晚的凭据重推，以及非 SSH 协议的迟到推送：恢复态 4×250ms、常规
// 2×250ms。旧 sidecar 对 SSH 仍回空串，最多多等 0.75s，路由结果不变；真未命中
// 的连接落进既有重试梯子（每次重试都会重新查询，协议一到就路由）。
async function resolveStoredConnectionProtocol(connectionId: string, bootRestore: boolean): Promise<{ protocol: string; shell: string; cwd: string; env: Array<{ name: string; value: string }> }> {
  const attempts = bootRestore ? 4 : 2;
  const delayMs = 250;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const result = await window.dbxPlugin.invoke<{ protocol?: string; shell?: string; cwd?: string; env?: Array<{ name?: unknown; value?: unknown }> }>("connection/protocol", { connectionId });
      const protocol = typeof result?.protocol === "string" ? result.protocol : "";
      if (protocol) {
        return {
          protocol,
          shell: typeof result.shell === "string" ? result.shell : "",
          // 连接表单的启动目录/环境变量（旧 sidecar 无键 → 空 = 未配置）。
          cwd: typeof result.cwd === "string" ? result.cwd.trim() : "",
          env: Array.isArray(result.env)
            ? result.env
                .filter((entry): entry is { name: string; value: string } => typeof entry === "object" && entry !== null && typeof (entry as { name?: unknown }).name === "string" && typeof (entry as { value?: unknown }).value === "string")
                .map((entry) => ({ name: entry.name, value: entry.value }))
            : [],
        };
      }
    } catch {
      // sidecar 未就绪/旧版 sidecar 无此方法：按未暂存处理，走推送协议。
      return { protocol: "", shell: "", cwd: "", env: [] };
    }
    if (attempt + 1 < attempts) await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  return { protocol: "", shell: "", cwd: "", env: [] };
}

// RDP 路由重入保护（与 sidecar 同 workbenchId 幂等查重双保险）：reconnect
// 梯子/宿主 replay 并发触发的 openSession 会同时抵达 RDP 路由，两次 rdp/start
// 会让 Windows 单会话策略互踢会话 → 前端持有的旧 sessionId 收到断开事件 →
// 重连梯子再触发，工作台卡死在"连接中"。启动期间置位吞掉并发重入。
let rdpRouteInFlight = false;
async function routeRdpConnection(): Promise<void> {
  if (rdpRouteInFlight) return;
  rdpRouteInFlight = true;
  try {
    if (!(await startRdpFromConnection())) rdpDialogOpen.value = true;
  } finally {
    rdpRouteInFlight = false;
  }
}

// 对话框发起的 RDP（回落/手动路径）：不经 openSession 的协议查询，表单协议
// 无从判定——发起前把 tab 级协议置为 rdp，会话断开/关闭后连接级门控仍成立。
async function connectViaRdpDialog(options: RdpConnectOptions): Promise<boolean> {
  storedConnectionProtocol.value = "rdp";
  return startRdpSession(options);
}

async function openSession(forceNew = false, bootRestore = false, isRetry = false, hostReopenAttempted = false) {
  if (!connectionId.value || !workbenchId.value) return;
  // 协议路由（对标 Tabby profile）：Telnet/VNC 连接不建 SSH 会话，直接驱动
  // 各自的会话启动；启动失败回落连接弹窗（预填 host/port，可改后重试）。
  // 宿主推给 webview 的 connection 上下文缺 external_config.protocol（该字
  // 段只随 lifecycle connection/connect 走向 sidecar），SSH 形态先向 sidecar
  // 查询权威协议再路由——lifecycle 推送与 webview 创建存在固有竞态，用短重
  // 试吸收（恢复态标签的凭据重推更晚，窗口放宽到 ~5s，对齐 BOOT_RESTORE 节奏）。
  if (connectionProtocol.value === "ssh" && connectionId.value) {
    const stored = await resolveStoredConnectionProtocol(connectionId.value, bootRestore);
    const storedProtocol = stored.protocol;
    // 权威协议写回响应式状态（storedConnectionProtocol）：路由之外的连接级
    // 门控（isRdpConnection）也以它为准，否则真实宿主下 RDP 连接的 SSH 入口
    // 整组漏出（宿主推送上下文缺 protocol，查询结果不落状态就只够当次路由）。
    if (storedProtocol) storedConnectionProtocol.value = storedProtocol;
    if (storedProtocol === "telnet") {
      if (!(await startTelnetFromConnection())) telnetDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "vnc") {
      if (!(await startVncFromConnection())) vncDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "serial") {
      if (!(await startSerialFromConnection())) serialDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "rdp") {
      await routeRdpConnection();
      return;
    }
    if (storedProtocol === "local") {
      // 本地 Shell 连接（LOCAL_CONNECTION_PROVIDER_ID，见 shared/lib/pluginIds.ts）：无网络语义，直接在连接绑定的
      // 工作台里开本地终端；连接表单 shell/cwd/env 为空时逐级回落偏好/默认
      // 解析链（cwd/env 的合并优先级见 startLocalTerminal）。
      await startLocalTerminal(stored.shell.trim() || undefined, {
        cwd: stored.cwd || undefined,
        env: stored.env.length ? stored.env : undefined,
      });
      return;
    }
  }
  if (connectionProtocol.value === "telnet") {
    if (!(await startTelnetFromConnection())) telnetDialogOpen.value = true;
    return;
  }
  if (connectionProtocol.value === "vnc") {
    if (!(await startVncFromConnection())) vncDialogOpen.value = true;
    return;
  }
  // M32-B：serial/rdp 连接记录与 telnet/vnc 同模式路由——配置驱动会话启动，
  // 缺字段/启动失败回落各自连接表单（对话框自带上次参数预填）。
  if (connectionProtocol.value === "serial") {
    if (!(await startSerialFromConnection())) serialDialogOpen.value = true;
    return;
  }
  if (connectionProtocol.value === "rdp") {
    await routeRdpConnection();
    return;
  }
  window.clearTimeout(reconnectTimer.value);
  reconnectAttempt.value = 0;
  // Boot-time tab restore can race the host's plugin activation and fail the
  // very first ssh/session/open; a bounded retry self-heals the restored
  // terminal instead of parking it on a manual reconnect button.
  // P1-1：重试计数只在"新入口"（用户动作 / 断线重连 / 初次打开）归零；
  // 重试定时器重入时必须保留计数，否则 OPEN_RETRY_MAX 永远打不满，
  // 认证失败等秒级永久错误会无限重试、错误文案永不呈现。
  if (!isRetry) openRetryAttempt = 0;
  // 重试重入保留等待提示；新入口（用户动作 / 断线重连 / 初次打开）重置。
  if (!isRetry) inactiveWaiting.value = false;
  // A session opened over a stale one must not inherit a stuck ZMODEM
  // overlay (zmodemBusy would keep swallowing terminal input).
  cancelZmodemUpload();
  // 同理不继承上一个会话的 trzsz 传输占用（在途传输一并停掉）。
  teardownTrzsz();
  // 同理不继承上一个会话的 AI 审批队列 / 执行横幅（切换会话清空全部排队挑战）。
  clearAgentPrompts();
  agentRunning.value = undefined;
  terminalState.value = "connecting";
  terminalError.value = "";
  reconnectPending.value = false;
  terminalInputQueue.reset();
  resetCommandMarker();
  connectCancelled.value = false;
  connectSucceeded.value = false;
  connectLog.push("info", t("connectCard.log.attempt", { attempt: openRetryAttempt + 1 }));
  if (forceNew && session.value) await closeSession(false);
  createTerminal();
  // A retry is only worth it for fast failures (boot-restore races with
  // plugin activation). A real dial failure takes tens of seconds — retrying
  // those just turns one error into minutes of spinner.
  const attemptStarted = Date.now();
  const attemptTimeoutMs = openRetryAttempt === 0 ? 120_000 : 30_000;
  try {
    const requestedSessionId = randomUUID();
    activeTerminalSessionId = requestedSessionId;
    lastSequence = 0;
    pendingTerminalFrames.clear();
    replayNoProgress = 0;
    const info = await window.dbxPlugin.invoke<SessionInfo>("ssh/session/open", {
      connectionId: connectionId.value,
      workbenchId: workbenchId.value,
      requestedSessionId,
      ...sessionTransportOpenParams(transportReuseState),
      // spawn 即真实面板尺寸，理由同 local/terminal/start（fitTerminalDimensions）。
      ...fitTerminalDimensions(),
    }, { timeoutMs: attemptTimeoutMs });
    // 用户取消后在途 open 仍可能成功（invoke 无法中止）：立即关闭这个孤儿
    // 会话并提前返回——不置 connected、不补发 replay，卡片停在已取消态。
    if (connectCancelled.value) {
      void window.dbxPlugin.invoke("ssh/session/close", { sessionId: info.sessionId }).catch(() => undefined);
      connectLog.push("warn", t("connectCard.log.orphanClosed"));
      return;
    }
    // The caller-provided id lets the binary listener accept first output before
    // the long-running open RPC resolves. The returned id remains authoritative.
    const requestedMatches = info.sessionId === requestedSessionId;
    markSessionTransportOpenSucceeded(transportReuseState, info.sessionId);
    activeTerminalSessionId = info.sessionId;
    session.value = info;
    directoryTrackingSupported.value = info.directoryTrackingSupported ?? true;
    if (!requestedMatches) {
      // Older sidecars ignore requestedSessionId, or the backend had to avoid a
      // collision. Discard frames captured under the speculative id and replay
      // the authoritative stream from its beginning.
      lastSequence = 0;
      pendingTerminalFrames.clear();
      replayNoProgress = 0;
    }
    connectSucceeded.value = true;
    hostReopenFailed = false;
    inactiveWaiting.value = false;
    connectLog.push("info", t("connectCard.log.connected", { seconds: ((Date.now() - attemptStarted) / 1000).toFixed(1) }));
    const replay = await window.dbxPlugin.invoke<ReplayResult>("ssh/terminal/replay", {
      sessionId: info.sessionId,
      afterSequence: requestedMatches ? lastSequence : 0,
    });
    if (!replay.complete) throw new Error(t("sessionUnrecoverable"));
    connectSucceeded.value = false;
    terminalState.value = "connected";
    await afterSessionConnected();
    // WKWebView（macOS 宿主）在"卡片遮盖 → 终端显示"过渡后可能漏一帧重绘，
    // 高亮装饰停留在过渡前状态直到首次交互（Mac 用户反馈"连接成功后 IP 高亮
    // 闪烁、点一下就好"）。状态落定为 connected 后下一帧主动全量刷新并重扫
    // 高亮，等价于那次点击；对 Chromium 是无害的一次多余重绘。
    window.requestAnimationFrame(() => {
      if (disposed || !terminal || terminalState.value !== "connected") return;
      terminal.refresh(0, terminal.rows - 1);
      rescanHighlightViewport();
    });
  } catch (cause) {
    if (disposed) return;
    connectSucceeded.value = false;
    // 用户已取消：不重试、不呈现错误，卡片停在已取消态等 Connect 重新发起。
    if (connectCancelled.value) return;
    // The selected source may close between opening the child workbench and
    // its first sidecar call. Downgrade once, explicitly, to a normal login;
    // subsequent failures follow the ordinary permanent-error policy.
    if (
      isDuplicatedTransportUnavailableError(cause)
      && fallbackToFreshTransport(transportReuseState)
    ) {
      connectLog.push("warn", t("connectCard.log.duplicateFallback"));
      await openSession(false, bootRestore, true);
      return;
    }
    const attemptMs = Date.now() - attemptStarted;
    // "Connection is not active"：sidecar 连接注册表还没有该连接。web 刷新后
    // 宿主进程不重启、不会重放 connection/connect（#202），boot 恢复与手动
    // 场景等价——首败都主动请求宿主按最新配置重开一次（幂等；webview 拿不到
    // 凭据，boot 态调用安全），成功后在有界窗口内轮询。宿主请求失败则立即
    // 呈现错误。认证 / host-key 拒绝是秒级永久错误，重试不可能自愈——跳过
    // 重试直接进 error 态，呈现 friendly 文案 + Reconnect 出口（P1-1）。
    // 决策细节见 connectRetry.ts。
    const inactive = isConnectionInactiveError(cause);
    // A live workbench can outlast its sidecar connection registration. Ask
    // the host to replay the vault-backed lifecycle once before polling for
    // the new registration. The webview never receives the credentials.
    if (inactive && openRetryAttempt === 0 && !hostReopenAttempted) {
      await requestHostReopenConnection();
    }
    // 预拨号面板的引导竞态：宿主在创建条目时已开始 connect，openSession 只
    // 是跑在了 connect 推送前面——用短间隔轮询等它落地，而不是 2s 退避梯子。
    const preconnect = bootRestore && panelSurface.value && hostContext.value.connectionPreconnected === true;
    const decision = decideConnectRetry({
      cause,
      attempt: openRetryAttempt,
      maxAttempts: OPEN_RETRY_MAX,
      attemptMs,
      inactive,
      bootRestore,
      preconnect,
      hostReopenFailed,
    });
    if (decision.kind === "retry") {
      openRetryAttempt = decision.attempt;
      // 只有手动入口的 inactive 轮询需要"请在侧边栏重开"提示；boot 恢复由宿主自动重放。
      inactiveWaiting.value = inactive && !bootRestore;
      terminalState.value = "connecting";
      connectLog.push("warn", t("connectCard.log.retry", { seconds: decision.delayMs / 1000 }));
      reconnectTimer.value = window.setTimeout(() => {
        if (!disposed) void openSession(false, bootRestore, true);
      }, decision.delayMs);
      return;
    }
    inactiveWaiting.value = false;
    terminalState.value = "error";
    activeTerminalSessionId = "";
    // 日志记录分类后的友好原因（与卡片错误行同源），未分类时保留原始错误串。
    const rawReason = cause instanceof Error ? cause.message : String(cause ?? "");
    const reasonKind = classifyConnectError(rawReason);
    connectLog.push("error", t("connectCard.log.failed", { reason: inactive ? t("connectionInactive") : reasonKind ? t(connectErrorKey(reasonKind)) : rawReason }));
    showError(inactive ? new Error(t("connectionInactive")) : cause, "terminal");
  }
}

async function attachSession(sessionId: string, retryReference: string = initialState().sessionId || "") {
  terminalState.value = "connecting";
  terminalInputQueue.reset();
  connectCancelled.value = false;
  // attach（tab 恢复）不播成功过渡动画，直接进终端。
  connectSucceeded.value = false;
  activeTerminalSessionId = sessionId;
  try {
    const info = await window.dbxPlugin.invoke<SessionInfo>("ssh/session/attach", {
      connectionId: connectionId.value,
      workbenchId: workbenchId.value,
      afterSequence: lastSequence,
    }, { timeoutMs: 15_000 });
    if (info.sessionId !== sessionId) throw new Error(t("errors.sessionChanged"));
    session.value = info;
    terminalState.value = "connected";
    reconnectPending.value = false;
    if (info.replay && !info.replay.complete) {
      terminalState.value = "error";
      terminalError.value = t("sessionUnrecoverable");
      return;
    }
    reconnectAttempt.value = 0;
    await afterSessionConnected();
  } catch (cause) {
    if (!shouldReattachTerminal({ disposed, state: terminalState.value, expectedSessionId: sessionId, currentSessionId: retryReference })) {
      terminalState.value = "error";
      reconnectPending.value = false;
      activeTerminalSessionId = "";
      showError(cause, "terminal");
      return;
    }
    const delay = terminalReconnectDelay(reconnectAttempt.value++);
    // Once the backoff ladder is exhausted, the stored session is gone for
    // good (e.g. the app was killed while the tab was open): re-attaching a
    // dead session id can never succeed, so fall back to a fresh open.
    if (reconnectAttempt.value > TERMINAL_RECONNECT_DELAYS.length) {
      reconnectAttempt.value = 0;
      reconnectPending.value = false;
      reconnectTimer.value = window.setTimeout(() => void openSession(true), delay);
      terminalError.value = t("reattachingTerminal");
      return;
    }
    reconnectNextAt.value = Date.now() + delay;
    reconnectDelayMs.value = delay;
    reconnectPending.value = true;
    reconnectTimer.value = window.setTimeout(() => void attachSession(sessionId), delay);
    terminalError.value = t("reattachingTerminal");
  }
}

async function afterSessionConnected() {
  terminalError.value = "";
  terminal?.focus();
  scheduleFit();
  await writeWorkbenchState();
  if (followDirectory.value) await setDirectoryTracking(true);
  if (sftpAutoConnect.value) {
    // #209 自动首拉走安静失败模式：失败落面板内软提示条，不弹全局错误横幅
    // （服务端停在堡垒机列表等场景的失败对用户是噪音）。home 探测与首拉并行，
    // 完成后初始化单栏树（目标 = 跟随 cwd / home）。
    await Promise.all([refreshSftpHomePath(), loadDirectory(currentPath.value, false, true), restoreTransfers()]);
    void initializeSftpTree();
  } else {
    // 自动连接已关：连接成功不主动碰 SFTP 子系统——列表首拉/主目录探测只在
    // 面板打开时由 initializeSftpTree 惰性补载（面板收起零请求）。
    await restoreTransfers();
    void initializeSftpTree();
  }
  // 「在外部编辑器中打开」菜单项的可用性依赖本机落盘能力，连接后即探测
  // （结果按工作台生命周期缓存，web/docker 为 false → 菜单项保持禁用）。
  void probeLocalCapabilities();
  // 远端 shell 历史导入(↑ history 面板内容对齐 shell history):异步拉取,
  // 不阻塞会话建立;失败静默(受限 shell/无历史文件时回落仅本会话采集)。
  const connectedSessionId = session.value?.sessionId;
  if (connectedSessionId) void loadRemoteShellHistory(connectedSessionId);
  // After an auto-reconnect succeeds, tell the user the session is back and
  // which working directory context it resumed with (pure-function chosen).
  if (reconnectWasPending.value) {
    reconnectWasPending.value = false;
    const restored = describeReconnectRestoredNotice({ wasReconnecting: true, path: currentPath.value });
    // Dock panel surface：目录提示属 SFTP/目录跟随域，面板里不弹。
    if (restored && !panelSurface.value) showNotice(t(restored.key, restored.values));
  }
}

async function closeSession(updateStatus = true) {
  const sessionId = session.value?.sessionId;
  session.value = undefined;
  activeTerminalSessionId = "";
  clearAgentPrompts();
  agentRunning.value = undefined;
  // Closing mid-ZMODEM aborts the transfer silently instead of leaving the
  // busy overlay and the dead sentry attached to the workbench.
  cancelZmodemUpload();
  // trzsz 在途传输同样静默停止（死会话上的 sendToServer 会因无 sessionId 空转）。
  teardownTrzsz();
  pendingTerminalFrames.clear();
  lastSequence = 0;
  terminalInputQueue.reset();
  reconnectPending.value = false;
  resetCommandMarker();
  // 新会话的 SFTP 目录视为未加载：自动连接关闭时，重连后手动打开面板会重新补载。
  sftpListingLoaded.value = false;
  // 单栏树按会话重建（根 "/"），避免旧连接的 home/展开分支串到新会话。
  resetSftpTree();
  // #209：面板内软错误条（跟随/自动首拉失败态）不跨会话存留。
  sftpSoftError.value = null;
  // 外部编辑器监听挂在会话上：断开前先停掉（后端 ssh/session/close 兜底）。
  // M15 多文件：清空整个 watch 注册表与逐文件确认队列。
  if (sessionId) {
    void window.dbxPlugin.invoke("watch/stop-all", { sessionId }).catch(() => undefined);
    activeExternalWatches.value = {};
    watchModifiedQueue.value = [];
  }
  if (sessionId) await window.dbxPlugin.invoke("ssh/session/close", { sessionId }).catch(() => undefined);
  if (updateStatus) {
    terminalState.value = "disconnected";
    terminalError.value = t("disconnected");
  }
  persistState();
}

// —— 本地终端生命周期 ——
// 退出态统一入口：exitCode 为 null 表示 sidecar 未上报（进程被杀/会话已回收），
// 覆盖层对 null 只显示"已退出"，有值时显示退出码。

async function startLocalTerminal(
  shellOverride?: string,
  connection?: { cwd?: string; env?: Array<{ name: string; value: string }> },
) {
  if (localState.value === "starting" || isLocalMode.value) return;
  localState.value = "starting";
  // 本地模式与 SFTP 分栏/批量栏互斥（对齐 RDP 进入会话前的收口惯例 4348）：
  // 按钮层的 !localUiMode 门控只藏入口、不收已开的面板——SSH 工作台带分栏切
  // 本地、或恢复态 tab 从布局镜像水合出 sftpPaneOpen=true，都会留下对着空
  // 会话的幽灵分栏。persistState 由既有 watch 带上。
  if (sftpPaneOpen.value) closeRightPanel();
  if (batchBarOpen.value) batchBarOpen.value = false;
  try {
    const info = await window.dbxPlugin.invoke<{ sessionId: string; shell: string }>("local/terminal/start", {
      workbenchId: workbenchId.value,
      // spawn 尺寸必须等于面板稳定后的尺寸（单靠 fitTerminalDimensions 不够——
      // dock「+」新 tab 此时可能还没布局完，fit 回落 80×24，稍后的真实 fit 是
      // 一次跨几何 resize，ConPTY 整屏重绘＝提示符/横幅渲染两遍）。
      ...(await settledTerminalDimensions()),
      // ConPTY 以 PSEUDOCONSOLE_INHERIT_CURSOR 创建，开场 CSI 6n 问的是
      // 「终端光标在哪」，子进程从该位置开始输出——必须回真实光标位置；
      // 回视口尺寸等于宣称光标在最后一行，横幅会被从底部重画（顶部孤字/
      // 双提示符的来源）。
      ...terminalCursorPosition(),
      // Shell precedence: explicit dock choice > user preference > auto-detection.
      ...(shellOverride?.trim() ? { shell: shellOverride.trim() } : localShellPref.value ? { shell: localShellPref.value } : {}),
      ...(localShellIntegrationPref.value ? {} : { shellIntegration: false }),
      // cwd 优先级：重开继承上次跟踪目录 > 连接表单启动目录 > 偏好启动目录
      // > 不传（家目录）。目录可能已被删，sidecar 会回落家目录。
      ...(localLastCwd.value
        ? { cwd: localLastCwd.value }
        : connection?.cwd
          ? { cwd: connection.cwd }
          : localShellCwdPref.value
            ? { cwd: localShellCwdPref.value }
            : {}),
      // 环境变量：偏好与连接表单合并（同名时连接表单覆盖），sidecar 侧还有
      // 一道同规则净化兜底。
      ...(() => {
        const pairs = [...localShellEnvClean.value, ...(connection?.env ?? [])];
        return pairs.length ? { env: Object.fromEntries(pairs.map((pair) => [pair.name, pair.value])) } : {};
      })(),
      // spawn 是快路径，但宿主桥丢响应时 promise 永久 pending＝纯黑屏；
      // 与同文件其他交互调用一致地给超时，失败落入 catch 走 showError。
    }, { timeoutMs: 10_000 });
    if (disposed) {
      void window.dbxPlugin.invoke("local/session/close", { sessionId: info.sessionId }, { timeoutMs: 10_000 }).catch(() => undefined);
      return;
    }
    localSession.value = { sessionId: info.sessionId, shell: info.shell };
    localState.value = "running";
    // The restored shell ends here: the session was explicitly restarted.
    localShellRestored.value = false;
    localExitCode.value = null;
    localLastSequence.value = 0;
    localPendingFrames.clear();
    localReplayNoProgress.value = 0;
    resetCommandMarker();
    await nextTick();
    scheduleFit();
    terminal?.focus();
  } catch (cause) {
    localState.value = "exited";
    showError(cause, "terminal");
  }
}

async function closeLocalTerminal() {
  const sessionId = localSession.value?.sessionId;
  localSession.value = null;
  localState.value = "exited";
  localPendingFrames.clear();
  localOpenConfirmOpen.value = false;
  sessionMenuOpen.value = false;
  sessionMenuShellOpen.value = false;
  sessionMenuShellPickOpen.value = false;
  if (!sessionId) return;
  await window.dbxPlugin.invoke("local/session/close", { sessionId }, { timeoutMs: 10_000 }).catch(() => undefined);
  terminal?.focus();
}

// —— Telnet 会话生命周期（P2-3，与本地终端同款互斥与补发机制）——
// 退出态统一入口：error 为 null 表示 sidecar 未带原因（会话已被回收），
// 非空时在退出覆盖层展示（连接失败/对端断开）。

async function startTelnetSession(options: TelnetConnectOptions): Promise<boolean> {
  // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
  if (telnetSession.value && telnetState.value !== "closed") await closeTelnetSession();
  // 自动登录两种形态互斥：声明式（提示正则 + 凭据，sidecar 落内置默认正则）
  // 优先；否则走 Expect 规则 + 密文槽。凭据只进 start 载荷与发送计划，
  // sidecar 侧不落日志/事件。
  const autoLogin = options.declarative
    ? { declarative: options.declarative }
    : options.rules
      ? {
          rules: options.rules,
          secrets: [options.secret1 ?? "", options.secret2 ?? ""],
        }
      : undefined;
  try {
    const info = await window.dbxPlugin.invoke<{ sessionId: string; host: string; port: number }>("telnet/start", {
      connectionId: connectionId.value || undefined,
      workbenchId: workbenchId.value,
      host: options.host,
      port: options.port,
      runtimeHost: runtimeEndpoint.value.host || options.host,
      runtimePort: runtimeEndpoint.value.port || options.port,
      enterMode: options.enterMode,
      backspaceMode: options.backspaceMode,
      ...fitTerminalDimensions(),
      ...(autoLogin ? { autoLogin } : {}),
    });
    if (disposed) {
      void window.dbxPlugin.invoke("telnet/close", { sessionId: info.sessionId }).catch(() => undefined);
      return false;
    }
    telnetSession.value = { sessionId: info.sessionId, host: info.host, port: info.port };
    telnetState.value = "connecting";
    telnetError.value = "";
    telnetLastSequence.value = 0;
    telnetPendingFrames.clear();
    telnetReplayNoProgress.value = 0;
    await nextTick();
    scheduleFit();
    terminal?.focus();
    return true;
  } catch (cause) {
    showError(cause, "terminal");
    return false;
  }
}

async function closeTelnetSession() {
  const sessionId = telnetSession.value?.sessionId;
  telnetSession.value = null;
  telnetState.value = "idle";
  telnetError.value = "";
  telnetPendingFrames.clear();
  if (!sessionId) return;
  await window.dbxPlugin.invoke("telnet/close", { sessionId }).catch(() => undefined);
  terminal?.focus();
}

// —— VNC 会话生命周期（nyaterm-parity P2 2d，与 Telnet 同款互斥展示）——
// 画面走 VncSurface 画布；帧/输入/剪贴板各走独立通道。连接/关闭/输入/帧
// 入口已收口在 composables/useVncSession，App.vue 只保留连接路由与装配。

// —— 连接驱动的会话启动（对标 Tabby profile 打开）：非 SSH 连接从宿主
// 连接表单直启各自会话；参数 = 连接 host/port + 上次使用记忆的偏好。
// 凭据不随连接持久化，需要自动登录/VNC 密码时从工具栏弹窗进入；直启
// 失败回落弹窗（预填 host/port，可改后重试）。
async function startTelnetFromConnection(): Promise<boolean> {
  const conn = connection.value;
  if (!conn.host) return false;
  const last = loadLastConnectParams<TelnetConnectOptions>("telnet-connect-last");
  return startTelnetSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 23,
    enterMode: last.enterMode === "cr" || last.enterMode === "lf" ? last.enterMode : "crlf",
    backspaceMode: last.backspaceMode === "ctrl_h" ? "ctrl_h" : "del",
  });
}

async function startVncFromConnection(): Promise<boolean> {
  const conn = connection.value;
  if (!conn.host) return false;
  const last = loadLastConnectParams<VncConnectOptions>("vnc-connect-last");
  return startVncSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 5900,
    scaleMode: last.scaleMode === "stretch" || last.scaleMode === "actual" ? last.scaleMode : "fit",
  });
}

// M32-B：串口连接路由——从连接 config 组装 SerialConnectOptions。设备路径
// 是唯一硬前提（其余参数有 sidecar 默认值）；startSerialSession 内部吞错并
// 落 showError，成功与否以 serialSession 挂载为准。
async function startSerialFromConnection(): Promise<boolean> {
  const conn = connection.value;
  if (!conn.serialPort) return false;
  await startSerialSession({
    portName: conn.serialPort,
    baudRate: conn.serialBaud ?? 115200,
    dataBits: conn.serialDataBits ?? "8",
    parity: conn.serialParity ?? "none",
    stopBits: conn.serialStopBits ?? "1",
    backspaceMode: conn.serialBackspace ?? "del",
  });
  return serialSession.value !== null;
}

// M32-B：RDP 连接路由——host/port 走连接绑定字段，domain/分辨率/证书策略/
// 剪贴板走 rdp_* config 字段；凭据（密码）不在连接配置内，由 RDP 会话自身的
// NLA/证书提示流兜底。失败回落 RdpConnectDialog（上次参数预填）。
async function startRdpFromConnection(): Promise<boolean> {
  const conn = connection.value;
  if (!conn.host) return false;
  // RDP 连接不渲染 SFTP 面板/分栏/批量发送栏：若同 tab 之前是 SSH 会话残留
  // 的开启态，进入 RDP 会话前收掉，避免隐藏门控下的幽灵分栏占位。
  if (sftpPaneOpen.value) closeRightPanel();
  if (batchBarOpen.value) batchBarOpen.value = false;
  return startRdpSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 3389,
    username: conn.username || "",
    // 尺寸优先级（P0-2）：workbenchState 里用户会话内选择的尺寸 > 连接表单
    // rdp_resolution > 默认 1280x800。
    width: rdpPersistedSize.value?.width ?? conn.rdpWidth ?? 1280,
    height: rdpPersistedSize.value?.height ?? conn.rdpHeight ?? 800,
    certificatePolicy: conn.rdpCertificatePolicy ?? "prompt",
    clipboard: conn.rdpClipboard ?? true,
    scaleMode: "fit",
    ...(conn.rdpDomain ? { domain: conn.rdpDomain } : {}),
  });
}

// —— RDP 会话生命周期（nyaterm-parity P3-4，与 VNC 同款互斥展示）——
// 画面走 RdpSurface 画布；帧/输入/剪贴板/指针各走独立通道，断线重连由
// sidecar 退避梯子驱动（reconnecting 态展示进度），graceful close/终态错误
// 落退出覆盖层（带 errorKind 友好文案 + 手动 rdp/reconnect 出口）。

/** 退出覆盖层主文案：errorKind 友好化（raw error 作细节行展示）。 */
const rdpClosedTitle = computed(() => {
  const kind = rdpErrorKindKey(rdpState.value.errorKind);
  return kind ? t(`rdp.error.${kind}`) : t("rdp.closed");
});


// —— 串口会话生命周期（P3，与 Telnet 同款互斥展示；无 replay，掉帧仅按
// pending 上限清空兜底）——
// 退出态统一入口：error 为 null 表示 sidecar 未带原因（主动关闭），
// 非空时在退出覆盖层展示（读线程 IO 失败/设备拔线）。

async function startSerialSession(options: SerialConnectOptions) {
  // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
  if (serialSession.value && serialState.value !== "closed") await closeSerialSession();
  try {
    // 线上字段为 snake_case：SerialStartRequest 未启用 camelCase rename；
    // 响应则由 sidecar 手拼 json!，sessionId/port/baudRate 为 camelCase，
    // binaryInput 为 B1 能力字段（旧 sidecar 缺失 → JSON 兼容路径）。
    const info = await window.dbxPlugin.invoke<{ sessionId: string; port: string; baudRate: number; binaryInput?: boolean }>("serial/start", {
      workbenchId: workbenchId.value,
      port_name: options.portName,
      baud_rate: options.baudRate,
      data_bits: options.dataBits,
      parity: options.parity,
      stop_bits: options.stopBits,
      backspace_mode: options.backspaceMode,
    });
    if (disposed) {
      void window.dbxPlugin.invoke("serial/close", { sessionId: info.sessionId }).catch(() => undefined);
      return;
    }
    serialSession.value = { sessionId: info.sessionId, port: info.port, baudRate: info.baudRate };
    serialState.value = "running";
    serialError.value = "";
    serialLastSequence.value = 0;
    serialPendingFrames.clear();
    serialUnknownStreamFrames.value = 0;
    // 能力探测降级（设计稿 §2）：未声明 binaryInput 的 sidecar 走 JSON
    // serial/write；通道报错时再一次性降级（send 回调）。
    serialBinaryInput.value = supportsBinaryInput(info);
    serialInputQueue.reset();
    // 从 A4 恢复外壳 tab 直接起串口时清掉外壳态，退出覆盖层随即让位。
    localShellRestored.value = false;
    await nextTick();
    scheduleFit();
    terminal?.focus();
  } catch (cause) {
    showError(cause, "terminal");
  }
}

async function closeSerialSession() {
  const sessionId = serialSession.value?.sessionId;
  serialSession.value = null;
  serialState.value = "idle";
  serialError.value = "";
  serialPendingFrames.clear();
  serialUnknownStreamFrames.value = 0;
  serialBinaryInput.value = true;
  serialInputQueue.reset();
  // 上传挂在会话上：随会话关闭一并终止（sidecar cancel 幂等）。
  cancelSerialUploadForSessionClose(sessionId);
  if (!sessionId) return;
  await window.dbxPlugin.invoke("serial/close", { sessionId }).catch(() => undefined);
  terminal?.focus();
}

// HOST_PLUGIN_UI_SPEC §8.3/§7.4 workbench/close 两段式关闭：宿主拆除 panel/tab webview
// 前先通知本 workbench 释放自己的 sidecar scope（PTY 会话），避免孤儿 PTY 活到 sidecar
// 退出。特 性探测：旧宿主不发 workbench/close，也无此 API。置 disposed 拦住在途的
// 异步启动流程（startLocalTerminal 会据此回收刚开的会话）。
if (window.dbxPlugin.workbench?.onClose) {
  window.dbxPlugin.workbench.onClose(async () => {
    disposed = true;
    // 只枚举本工作台自有会话（workbenchCloseTargets 契约）：同连接其他 tab、
    // 底部 dock 终端等兄弟工作台的会话不在列表里，关闭本 tab 绝不波及它们。
    // 连接级清场（connection/disconnect）是宿主生命周期语义，插件不代劳。
    const ownedSessions = workbenchCloseTargets([
      ["local/session/close", localSession.value?.sessionId],
      ["ssh/session/close", session.value?.sessionId],
      // 分屏/内嵌页签 PTY 使用独立 workbenchId 后缀，workbench/close 不会扫到它们。
      ...[...shellTabSessions.values()].map((sessionId) => ["ssh/session/close", sessionId] as const),
      ["telnet/close", telnetSession.value?.sessionId],
      ["serial/close", serialSession.value?.sessionId],
      ["vnc/close", vncSession.value?.sessionId],
      ["rdp/close", rdpSession.value?.sessionId],
    ]);
    await Promise.allSettled(ownedSessions.map(([method, sessionId]) => window.dbxPlugin.notify(method, { sessionId })));
    localSession.value = null;
    session.value = undefined;
    telnetSession.value = null;
    serialSession.value = null;
    vncSession.value = null;
    rdpSession.value = null;
    rdpDesktopSize.value = null;
    rdpClipboardChunks.clear();
  });
}

// "Close" on a restored shell: there is no session to close — fall into the same disconnected state as an SSH restored tab
// (no connection replay, spec §7.6) and the exit overlay steps aside.
function dismissRestoredLocalShell() {
  localShellRestored.value = false;
  localState.value = "exited";
  terminalState.value = "disconnected";
  terminalError.value = t("restartDisconnected");
}

// Toolbar local-terminal button: running -> close; restored shell -> reopen directly (nothing to close, skipping
// SSH confirm flow); serial/telnet mode -> close that session; an SSH state walks the existing confirm flow.
function toggleLocalTerminal() {
  if (isRdpMode.value) {
    void closeRdpSession();
    return;
  }
  if (isVncMode.value) {
    void closeVncSession();
    return;
  }
  if (isSerialMode.value) {
    void closeSerialSession();
    return;
  }
  if (isTelnetMode.value) {
    void closeTelnetSession();
    return;
  }
  if (localShellRestored.value) {
    void restartLocalTerminal();
    return;
  }
  if (isLocalMode.value) {
    void closeLocalTerminal();
    return;
  }
  requestLocalTerminal();
}

// 右键菜单"重跑最近命令"：把命令写入本地 PTY（危险命令复用粘贴确认），
// 补回车立即执行；多行命令归一为回车分隔。
async function rerunLocalCommand(command: string) {
  if (!localSession.value) return;
  const payload = command.replace(/\r\n|\r|\n/g, "\r") + "\r";
  trackPendingInput(payload);
  await sendConfirmedPaste(payload);
}

async function restartLocalTerminal(shellOverride?: string) {
  await closeLocalTerminal();
  await startLocalTerminal(shellOverride);
}

// 一次性 shell override 的确认挂起：requestLocalTerminal 在 SSH 在连时先走
// 确认弹窗，所选 shell 存这里；确认/取消两路各自消费/清空（watch 兜底）。
let pendingLocalShellOverride = "";

// SSH 会话在连/连接中/重连中时先经确认关闭（本地模式与 SSH 会话互斥展示，
// connecting 途中放行会让在途 ssh/session/open 成功后与本地会话抢同一终端
// 视图），再开本地终端。
function requestLocalTerminal(shellOverride?: string) {
  // 串口/Telnet/VNC/RDP 会话占用终端视图时不开本地终端（互斥展示）。
  if (isSerialMode.value || isTelnetMode.value || isVncMode.value || isRdpMode.value) return;
  if (isLocalMode.value || localState.value === "starting") return;
  if (session.value || reconnectPending.value || terminalState.value === "connecting") {
    pendingLocalShellOverride = shellOverride ?? "";
    localOpenConfirmOpen.value = true;
    return;
  }
  void startLocalTerminal(shellOverride);
}

async function confirmLocalTerminal() {
  localOpenConfirmOpen.value = false;
  const shellOverride = pendingLocalShellOverride || undefined;
  pendingLocalShellOverride = "";
  await closeSession();
  await startLocalTerminal(shellOverride);
}

// 确认弹窗取消（按钮/X/外点/Esc 多路）统一清掉挂起的 override，避免残留到
// 下一次无 override 的确认；confirm 路径在本行同步消费后才会触发本 watch。
watch(localOpenConfirmOpen, (open) => {
  if (!open) pendingLocalShellOverride = "";
});

// webview 重建后接回 sidecar 里仍活着的本地 shell（workbench/close 才回收）。
async function reattachLocalSession(): Promise<boolean> {
  try {
    const result = await window.dbxPlugin.invoke<{ sessions?: Array<{ sessionId: string; workbenchId: string; shell: string }> }>(
      "local/session/list",
      {},
      { timeoutMs: 10_000 },
    );
    const match = (result?.sessions || []).find((session) => session.workbenchId === workbenchId.value);
    if (!match) return false;
    localSession.value = { sessionId: match.sessionId, shell: match.shell };
    localState.value = "running";
    localExitCode.value = null;
    localLastSequence.value = 0;
    localPendingFrames.clear();
    // 回放走统一补发路径（requestLocalReplay，自带超时与 resync）：环形缓冲
    // 淘汰过帧的活会话（累计输出 > 2MiB 必然发生）接回尾部继续，而不是被
    // !complete 误判成已退出——shell 在 sidecar 里还活着。
    requestLocalReplay(0, null);
    await nextTick();
    scheduleFit();
    return true;
  } catch {
    return false;
  }
}

// 手动重连前请宿主按当前最新配置重开连接：连接在侧边栏被编辑（如改密码）后，
// 宿主会摘掉 connected 标记且不回推插件，sidecar 里存的凭据就此过期——不先
// 重开的话 openSession 只会拿旧凭据反复失败。宿主以打开连接的同款流程重新
// 下发配置（含 vault 里的最新凭据）。失败记录下来，避免后续对空连接表
// 反复发起 ssh/session/open；旧宿主仍可在连接表已有记录时继续拨号。
let hostReopenFailed = false;
async function requestHostReopenConnection(): Promise<boolean> {
  if (!connectionId.value) return false;
  const restored = await reopenHostConnection(window.dbxPlugin, connectionId.value);
  hostReopenFailed = !restored;
  return restored;
}

// 同工作台 spawn 新 tab（新建/复制/命令会话）：收口在
// composables/useWorkbenchSpawnTabs（context 克隆与宿主桥细节见该文件）。
const {
  openNewSessionTab,
  openCopiedSessionTab,
  spawnSessionDialogOpen,
  spawnSessionCommand,
  openCommandSessionTab,
  confirmCommandSessionTab,
} = useWorkbenchSpawnTabs({
  hostContext,
  connectionId: () => connectionId.value,
  session,
  connected: () => connected.value,
});


// 传输历史 + 断点续传：状态与 sidecar 接线收口在 composables/useTransferHistory。
const {
  transferHistory,
  transferHistoryLoading,
  transferHistoryFailed,
  transferHistoryClearOpen,
  refreshTransferHistory,
  confirmTransferHistoryClear,
  resumableTasks,
  resumableLoading,
  resumeTargetTaskId,
  refreshResumableUploads,
  beginResumeUpload,
  onResumeFilePicked,
} = useTransferHistory({ t, showNotice, showError, uploadSource, loadDirectory: (path?: string) => loadDirectory(path) });


async function resolveHostKey(accept: boolean) {
  const prompt = hostKeyPrompt.value;
  if (!prompt) return;
  hostKeyPrompt.value = undefined;
  try {
    await window.dbxPlugin.invoke("connection/challenge/resolve", {
      challengeId: prompt.challengeId,
      operationId: prompt.operationId,
      accept,
      remember: accept && rememberHostKey.value,
    });
    connectLog.push("info", t("connectCard.log.hostKeyResolved"));
  } catch (cause) {
    showError(cause, "terminal");
  }
}


// AI 终端同步执行：审批队列/执行横幅/模式切换收口在 composables/useAgentTerminalMode。
const {
  agentPromptCommand,
  agentPromptRemaining,
  agentPromptExpired,
  agentPromptRemember,
  agentPromptQueue,
  agentRunning,
  agentModeOpen,
  agentMode,
  agentModeBusy,
  agentModeHint,
  agentPromptHead,
  clearAgentPrompts,
  stopAgentPromptTimer,
  dismissAgentPrompt,
  resolveAgentPrompt,
  interruptAgentRun,
  toggleAgentModeMenu,
  refreshAgentMode,
  applyAgentMode,
} = useAgentTerminalMode({ t, showError, session, connectionId: () => connectionId.value, sendTerminalBytes, closeToolbarPopovers });

// 告警排查：状态与分诊交互收口在 composables/useAlertTriage。
const {
  alertTriageOpen,
  alertTriageBusy,
  alertTriageError,
  alertTriagePayload,
  alertTriageResult,
  openAlertTriage,
  runAlertTriage,
  sendSuggestionToTerminal,
  copySuggestions,
} = useAlertTriage({ t, showError, showNotice, session, terminal: () => terminal, trackPendingInput, sendTerminalBytes, clipboardDeps });

// 端口映射（-L/-R）管理弹窗：全部逻辑在 components/PortForwardDialog.vue，
// 这里只保留工具栏入口的开关状态。
const forwardsOpen = ref(false);

// 容器管理：右侧栏第三视图（与 SFTP/指标互斥）；面板本体仍由 DockerPanel
// 自治（会话解析/轮询/动作/引擎设置都在组件内），切走即卸载停轮询。
// 打开期间的分栏轴向/比例由上方 dockerPanelOpen + paneLayout 接管。

// 关键词高亮：规则表 CRUD 收口在 composables/useHighlightRules（渲染引擎在下方）。
const {
  highlightRules,
  highlightSaving,
  highlightEnabled,
  highlightDisplayMode,
  compiledHighlightRules,
  hydrateHighlightRules,
  saveHighlightRule,
  toggleHighlightRule,
  deleteHighlightRule,
  setHighlightDisplayMode,
} = useHighlightRules({ showError });

// 「字体颜色」= 把规则色交给 decoration 的 foregroundColor，由渲染器写进该格字形：
// DOM 渲染器落到单元格 span 的 color（DomRendererRowFactory），WebGL 渲染器在
// CellColorResolver 里把 foregroundColorRGB 折进该格 fg 属性（glyph atlas 以 fg
// 为键、会自动重新光栅化）——两条渲染路径都生效，故无需回退 DOM、GPU 照常。
const keywordTextColorActive = computed(() => highlightDisplayMode.value === "text");

// ---- decoration 引擎 ----
// onRender({start,end}) 给的是"本帧实际重绘的行区间"（输入时常常只有光标一行），
// 不是整个视口：合并进 pending 区间，经 setTimeout 节流（≤30fps）后统一扫描；
// 扫描时装饰去留按 buffer.viewportY 的真实视口判定（见 scanHighlightRange）。
// alt buffer 与 normal buffer 走同一路径（buffer.active 直接扫描）。
let highlightRenderDisposable: { dispose(): void } | undefined;
// 每行一个组（marker + decorations + 该行登记时的文本）；行滚出视口整组 dispose。
// text 用于"文本未变则整组保留"：xterm 在装饰 dispose/注册后自身会再触发整幅
// 重绘（实测 30fps 持续循环），无脑拆建会让空闲终端陷入"重绘→扫描→拆建→重绘"
// 的自激回路，高亮层反复摘挂即是用户看到的闪烁。
const highlightDecorationsByRow = new Map<number, { text: string; dispose(): void }>();
let highlightDecorationCount = 0;
let highlightScanScheduled = false;
let highlightLastScanAt = 0;
let highlightPendingRange: { start: number; end: number } | undefined;

function clearHighlightDecorations() {
  for (const entry of highlightDecorationsByRow.values()) entry.dispose();
  highlightDecorationsByRow.clear();
  highlightDecorationCount = 0;
}

function attachHighlightRender() {
  if (!terminal || highlightRenderDisposable || !highlightEnabled.value) return;
  highlightRenderDisposable = terminal.onRender(({ start, end }) => scheduleHighlightScan(start, end));
  rescanHighlightViewport();
}

function detachHighlightRender() {
  highlightRenderDisposable?.dispose();
  highlightRenderDisposable = undefined;
  clearHighlightDecorations();
}

function rescanHighlightViewport() {
  if (!terminal || !highlightEnabled.value) return;
  scheduleHighlightScan(0, terminal.rows - 1);
}

function scheduleHighlightScan(start: number, end: number) {
  if (!terminal || !highlightEnabled.value || !compiledHighlightRules.value.length) return;
  // 大输出保护生效期挂起扫描，恢复时由 onOutputGateRelease 补扫视口。
  if (outputGate.mode === "strained") return;
  highlightPendingRange = highlightPendingRange
    ? { start: Math.min(highlightPendingRange.start, start), end: Math.max(highlightPendingRange.end, end) }
    : { start, end };
  if (highlightScanScheduled) return;
  highlightScanScheduled = true;
  const wait = Math.max(0, HIGHLIGHT_SCAN_MIN_INTERVAL_MS - (performance.now() - highlightLastScanAt));
  window.setTimeout(runHighlightScan, wait);
}

function runHighlightScan() {
  highlightScanScheduled = false;
  highlightLastScanAt = performance.now();
  const range = highlightPendingRange;
  highlightPendingRange = undefined;
  if (!range || !terminal || !highlightEnabled.value) return;
  scanHighlightRange(range.start, range.end);
}

function scanHighlightRange(start: number, end: number) {
  const term = terminal;
  if (!term) return;
  const buffer = term.buffer.active;
  // onRender 的 start/end 是视口相对行号，这里的 row / viewportY 是缓冲绝对行号
  // （换算与裁剪见 toAbsoluteRowRange：漏了这步，滚过一屏后脏行判定永不命中）。
  const dirty = toAbsoluteRowRange(start, end, buffer.viewportY, buffer.length);
  // onRender 给的是"本帧重绘的行"（输入时常常只有光标行），不是视口——装饰的
  // 去留必须按视口判定，否则每次击键都把整屏高亮 dispose 掉再异步补回（可见闪烁）。
  const vpFrom = Math.max(0, Math.min(buffer.viewportY, buffer.length - 1));
  const vpTo = Math.min(buffer.length - 1, vpFrom + term.rows - 1);
  for (const [row, entry] of highlightDecorationsByRow) {
    // 拆组条件（视口外 / 本帧重绘且文本变了）见 shouldRebuildHighlightRow；
    // "重绘但文本没变就保留"是掐断"拆建→重绘"自激回路的关键。
    const dirtyRow = row >= dirty.from && row <= dirty.to;
    const currentText = dirtyRow ? buffer.getLine(row)?.translateToString(true) ?? "" : entry.text;
    if (!shouldRebuildHighlightRow({ row, viewportFrom: vpFrom, viewportTo: vpTo, dirty: dirtyRow, previousText: entry.text, currentText })) continue;
    entry.dispose();
    highlightDecorationsByRow.delete(row);
  }
  const compiled = compiledHighlightRules.value;
  if (!compiled.length) return;
  // registerMarker 的 offset 相对光标绝对行（baseY + cursorY）；marker dispose 时
  // xterm 会连带 dispose 挂在其上的 decoration。
  const base = buffer.baseY + buffer.cursorY;
  for (let row = vpFrom; row <= vpTo; row++) {
    if (highlightDecorationsByRow.has(row)) continue;
    if (highlightDecorationCount >= HIGHLIGHT_DECORATION_LIMIT) return;
    const bufferLine = buffer.getLine(row);
    const lineText = bufferLine?.translateToString(true) ?? "";
    if (!lineText) continue;
    const matches = matchesInLine(lineText, compiled);
    if (!matches.length) continue;
    // 命中是文本下标，decoration 的 x/width 是单元格列号：宽字符（CJK、全角标点）在文本里
    // 占 1 个单元、在终端里占 2 列，直接用下标会让整段高亮左移——前面有几个宽字符就少几列
    // （中文行里越靠右的命中越歪，IP/时间戳会整段露白）。只有真有命中的行才补一次列号表，
    // 其余行继续走 translateToString 的内部缓存。
    const columns = bufferLine ? lineColumnTable(bufferLine, lineText.length) : undefined;
    const marker = term.registerMarker(row - base);
    if (!marker) continue;
    const disposables: Array<{ dispose(): void }> = [marker];
    const entry = {
      text: lineText,
      decorations: 0,
      dispose() {
        for (const disposable of disposables.splice(0)) disposable.dispose();
        // 计数只增不减会顶到全局上限、高亮逐渐不再出现（看起来像闪烁后消失）。
        highlightDecorationCount -= entry.decorations;
        entry.decorations = 0;
      },
    };
    for (const match of matches) {
      if (highlightDecorationCount >= HIGHLIGHT_DECORATION_LIMIT) break;
      // 「字体颜色」模式：把规则色交给 decoration 的 foregroundColor——无覆盖层、
      // 无需逐帧重放（渲染器每次重绘都会重新查询 decoration），也不与配色方案/主题
      // 的调色板打架。DOM 与 WebGL 两条渲染路径都消费该字段。
      const { x, width } = matchToColumns(columns, match.start, match.end);
      const decoration = term.registerDecoration(
        keywordTextColorActive.value
          ? { marker, x, width, foregroundColor: highlightTextStyle(match.color) }
          : { marker, x, width },
      );
      if (decoration) {
        if (!keywordTextColorActive.value) {
          // 背景色块模式保持原样：xterm 6 的 DOM renderer 仍不把 registerDecoration
          // 的 backgroundColor 选项应用到 cell（且 CM_RGB 路径会丢 alpha，实心色会
          // 盖字形），着色继续走 onRender 在 overlay 元素上半透明自绘。注意 overlay
          // 元素由 BufferDecorationRenderer 在渲染刷新帧里创建：窗口被遮挡/rAF 冻结
          // 期间注册的高亮要等下一次刷新帧才显色（可见后一帧内自愈，属显示时序而非
          // 逻辑缺陷——逐字符 span 是 cell 层 run 拆分，不代表 decoration 丢失）。
          decoration.onRender((element) => {
            element.style.backgroundColor = highlightFillStyle(match.color);
          });
        }
        disposables.push(decoration);
        entry.decorations++;
        highlightDecorationCount++;
      }
    }
    highlightDecorationsByRow.set(row, entry);
  }
}

// 规则表/显示方式变化：全量清理后重扫当前视口（即时生效语义）。显示方式切换必须
// 重建 decoration——背景色块画在 overlay 元素上、字体颜色是 registerDecoration 的
// foregroundColor 选项，两者无法就地改样式互转。
watch([compiledHighlightRules, highlightDisplayMode], () => {
  clearHighlightDecorations();
  rescanHighlightViewport();
});

// ---------------------------------------------------------------------------
// 动作链接 + 行号/时间戳 gutter（IMPL_PLAN Task P1-2 / P1-3，均默认关闭）。
// 偏好权威态在此，经 sidecar preferences.json（backend/src/preferences.rs 的
// 固定 allowlist，local/preferences/get|set）持久化；设置页控件在
// SettingsDialog「终端」分类，经 update:* 增量上抛。
// ---------------------------------------------------------------------------

async function persistTerminalFeaturePrefs(patch: Record<string, unknown>) {
  try {
    await window.dbxPlugin.invoke("local/preferences/set", patch);
  } catch {
    // 旧 sidecar 无这些键位：与 localShell 等同款，当前会话内存态兜底。
  }
}

// ---- 动作链接（P1-2）----
const actionLinksSettings = ref<ActionLinksSettings>(sanitizeActionLinksSettings(undefined));
const actionLinksEnabled = computed(() => actionLinksSettings.value.enabled);
// 与关键词高亮同帧率上限 / 同量级装饰总数护栏。
const ACTION_LINK_SCAN_MIN_INTERVAL_MS = 33;
const ACTION_LINK_DECORATION_LIMIT = 400;
let actionLinkProviderDisposable: { dispose(): void } | undefined;
let actionLinkRenderDisposable: { dispose(): void } | undefined;
let actionLinkScanScheduled = false;
let actionLinkLastScanAt = 0;
let actionLinkPendingRange: { start: number; end: number } | undefined;
// 每行一组（marker + 虚线 decorations + 登记文本）；去留条件复用关键词高亮的
// shouldRebuildHighlightRow——"本帧重绘且文本未变则整组保留"是防自激回路的
// 关键（xterm 在装饰注册/销毁后会再触发整幅重绘）。
const actionLinkDecorationsByRow = new Map<number, { text: string; dispose(): void }>();
let actionLinkDecorationCount = 0;
// 悬停 / Alt+点击的命令预览浮签（terminal-pane 内绝对定位，pointer-events 关）。
const actionLinkHint = ref<{ x: number; y: number; text: string } | null>(null);

function clearActionLinkDecorations() {
  for (const entry of actionLinkDecorationsByRow.values()) entry.dispose();
  actionLinkDecorationsByRow.clear();
  actionLinkDecorationCount = 0;
}

function attachActionLinks() {
  if (!terminal || actionLinkProviderDisposable || !actionLinksEnabled.value) return;
  actionLinkProviderDisposable = terminal.registerLinkProvider(
    createActionLinkProvider(terminal, {
      matchers: actionLinksSettings.value.matchers,
      callbacks: {
        onActivate: handleActionLinkActivate,
        onHover: showActionLinkHintAt,
        onLeave: hideActionLinkHint,
      },
    }),
  );
  actionLinkRenderDisposable = terminal.onRender(({ start, end }) => scheduleActionLinkScan(start, end));
  scheduleActionLinkScan(0, terminal.rows - 1);
}

function detachActionLinks() {
  actionLinkProviderDisposable?.dispose();
  actionLinkProviderDisposable = undefined;
  actionLinkRenderDisposable?.dispose();
  actionLinkRenderDisposable = undefined;
  clearActionLinkDecorations();
  hideActionLinkHint();
}

// 点击 = 把建议命令送进现有 PTY 输入通路（不含换行：shell 输入行停在原地，
// 用户可补改后再回车执行）。Alt+点击 = 仅预览命令文本，不向 PTY 写入。
function handleActionLinkActivate(match: ActionLinkMatch, event: MouseEvent) {
  if (event.altKey) {
    showActionLinkHintAt(match, event);
    return;
  }
  const sessionId = localSession.value?.sessionId ?? session.value?.sessionId;
  if (!sessionId) return;
  sendTerminalBytes(new TextEncoder().encode(match.command));
  terminal?.focus();
}

// ---- Docker 面板「在终端打开」（M3 遗留 6）----
// DockerPanel 挂在终端区右上浮层内，面板经 window 自定义事件把命令字符串直达
// 这里；走与建议浮层同款的「填入输入行不回车」
// 通道（replaceTerminalLineWith）：命令落在 shell 输入行原地，用户确认后自行回车。
// 无终端会话时 toast 提示先连接，不静默丢弃。
function handleDockerOpenInTerminal(event: Event) {
  const command = (event as CustomEvent<{ command?: string }>).detail?.command ?? "";
  if (!command) return;
  const hasTerminalSession = Boolean(
    localSession.value ?? session.value ?? serialSession.value ?? telnetSession.value,
  );
  if (!terminal || !hasTerminalSession) {
    showNotice(t("docker.terminalNeedSession"));
    return;
  }
  replaceTerminalLineWith(command, false);
  terminal.focus();
}

function showActionLinkHintAt(match: ActionLinkMatch, event: MouseEvent) {
  const host = terminalHost.value;
  if (!host) return;
  const bounds = host.getBoundingClientRect();
  actionLinkHint.value = {
    x: Math.min(Math.max(event.clientX - bounds.left + 10, 4), Math.max(4, bounds.width - 280)),
    y: Math.max(4, event.clientY - bounds.top - 34),
    text: match.command,
  };
}

function hideActionLinkHint() {
  actionLinkHint.value = null;
}

function scheduleActionLinkScan(start: number, end: number) {
  if (!terminal || !actionLinksEnabled.value) return;
  // 大输出保护生效期挂起扫描，恢复时由 onOutputGateRelease 补扫视口。
  if (outputGate.mode === "strained") return;
  actionLinkPendingRange = actionLinkPendingRange
    ? { start: Math.min(actionLinkPendingRange.start, start), end: Math.max(actionLinkPendingRange.end, end) }
    : { start, end };
  if (actionLinkScanScheduled) return;
  actionLinkScanScheduled = true;
  const wait = Math.max(0, ACTION_LINK_SCAN_MIN_INTERVAL_MS - (performance.now() - actionLinkLastScanAt));
  window.setTimeout(runActionLinkScan, wait);
}

function runActionLinkScan() {
  actionLinkScanScheduled = false;
  actionLinkLastScanAt = performance.now();
  const range = actionLinkPendingRange;
  actionLinkPendingRange = undefined;
  if (!range || !terminal || !actionLinksEnabled.value) return;
  scanActionLinkRange(range.start, range.end);
}

function scanActionLinkRange(start: number, end: number) {
  const term = terminal;
  if (!term) return;
  const buffer = term.buffer.active;
  // onRender 的视口相对行号 → 缓冲绝对行号 + 视口判定，与关键词高亮同款换算。
  const dirty = toAbsoluteRowRange(start, end, buffer.viewportY, buffer.length);
  const vpFrom = Math.max(0, Math.min(buffer.viewportY, buffer.length - 1));
  const vpTo = Math.min(buffer.length - 1, vpFrom + term.rows - 1);
  for (const [row, entry] of actionLinkDecorationsByRow) {
    // 防自激：仅"滚出视口"或"本帧重绘且文本确实变化"才拆组重建。
    const dirtyRow = row >= dirty.from && row <= dirty.to;
    const currentText = dirtyRow ? buffer.getLine(row)?.translateToString(true) ?? "" : entry.text;
    if (!shouldRebuildHighlightRow({ row, viewportFrom: vpFrom, viewportTo: vpTo, dirty: dirtyRow, previousText: entry.text, currentText })) continue;
    entry.dispose();
    actionLinkDecorationsByRow.delete(row);
  }
  const matchers = actionLinksSettings.value.matchers;
  const keywordRules = compiledHighlightRules.value;
  const base = buffer.baseY + buffer.cursorY;
  for (let row = vpFrom; row <= vpTo; row++) {
    if (actionLinkDecorationsByRow.has(row)) continue;
    if (actionLinkDecorationCount >= ACTION_LINK_DECORATION_LIMIT) return;
    const bufferLine = buffer.getLine(row);
    const lineText = bufferLine?.translateToString(true) ?? "";
    if (!lineText) continue;
    const matches = matchActionLinks(lineText, matchers);
    if (!matches.length) continue;
    // 让位：与用户关键词高亮同段命中的范围跳过（高亮是用户显式配置的规则）。
    const keywordSpans = keywordRules.length ? matchesInLine(lineText, keywordRules) : [];
    const visible = keywordSpans.length
      ? matches.filter((match) => !keywordSpans.some((span) => match.start < span.end && match.end > span.start))
      : matches;
    if (!visible.length) continue;
    // 常驻虚线同样按单元格列号绘制（理由见关键词高亮处）；让位过滤仍在下标空间做，
    // 只比较命中区间的重叠关系，不需要换算。
    const columns = bufferLine ? lineColumnTable(bufferLine, lineText.length) : undefined;
    const marker = term.registerMarker(row - base);
    if (!marker) continue;
    const disposables: Array<{ dispose(): void }> = [marker];
    const entry = {
      text: lineText,
      decorations: 0,
      dispose() {
        for (const disposable of disposables.splice(0)) disposable.dispose();
        actionLinkDecorationCount -= entry.decorations;
        entry.decorations = 0;
      },
    };
    for (const match of visible) {
      if (actionLinkDecorationCount >= ACTION_LINK_DECORATION_LIMIT) break;
      const { x, width } = matchToColumns(columns, match.start, match.end);
      const decoration = term.registerDecoration({ marker, x, width });
      if (!decoration) continue;
      // 虚线下划线画在装饰元素下缘（装饰层在文字层上方，无填充不遮字形）。
      decoration.onRender((element) => {
        element.style.borderBottom = "1px dashed var(--primary)";
      });
      disposables.push(decoration);
      entry.decorations++;
      actionLinkDecorationCount++;
    }
    actionLinkDecorationsByRow.set(row, entry);
  }
}

// 设置变化（总开关或三类匹配器）：即时生效——provider 构造时快照 matchers，
// 任何变化都重建；关闭时零挂钩子（摘 provider + 清 decoration）。
watch(actionLinksSettings, (next) => {
  if (!terminal) return;
  if (next.enabled) {
    actionLinkProviderDisposable?.dispose();
    actionLinkProviderDisposable = undefined;
    clearActionLinkDecorations();
    attachActionLinks();
  } else {
    detachActionLinks();
  }
}, { deep: true });

// ---- 行号 / 时间戳 gutter（P1-3）----
const gutterSettings = ref<GutterSettings>(sanitizeGutterSettings(undefined));
const gutterRows = ref<GutterRow[]>([]);
const gutterCellHeight = ref<number | null>(null);
// 大输出写入期挂起重算（terminalWriteThrottle 积压 > 256KiB 即跳帧，落定后由
// 下一次 onRender/onScroll 事件跟上），避免 gutter 追帧放大 strained 场景开销。
const GUTTER_SUSPEND_PENDING_BYTES = 256 * 1024;
let gutterRafId = 0;
let gutterScreenOffsetTop = 0;
// 逻辑行首绝对行号 → 写入时刻。裁剪保留视口顶端前 3000 行。
const gutterTimestamps = new Map<number, number>();
let gutterLastStampedRow = -1;
let gutterRenderDisposable: { dispose(): void } | undefined;
let gutterScrollDisposable: { dispose(): void } | undefined;
let gutterResizeDisposable: { dispose(): void } | undefined;
let gutterEnterDisposable: { dispose(): void } | undefined;

const gutterPaneVisible = computed(() => gutterSettings.value.showLineNumbers || gutterSettings.value.showTimestamps);
// gutter 只占终端左 padding 环带：两种开关组合给固定宽度（行号 5 位 + 余量）。
const gutterWidth = computed(() => {
  if (!gutterPaneVisible.value) return 0;
  if (gutterSettings.value.showLineNumbers && gutterSettings.value.showTimestamps) return 132;
  return gutterSettings.value.showLineNumbers ? 56 : 88;
});
// 渲染尺寸读不到（渲染器未就绪/WebGL 恢复中）时整体隐藏降级，不报错。
const gutterVisible = computed(() => gutterPaneVisible.value && gutterCellHeight.value !== null && gutterRows.value.length > 0);
const gutterPaneStyle = computed(() => ({ "--dbx-gutter-width": `${gutterWidth.value}px` }));

function isGutterActive() {
  return gutterPaneVisible.value;
}

function attachGutterListeners() {
  if (!terminal || gutterRenderDisposable || !isGutterActive()) return;
  gutterRenderDisposable = terminal.onRender(() => scheduleGutterRecompute());
  gutterScrollDisposable = terminal.onScroll(() => scheduleGutterRecompute());
  gutterResizeDisposable = terminal.onResize(() => scheduleGutterRecompute());
  // 回车重盖光标逻辑行：独立 onData 挂子（routeTerminalData 输入路由不动）。
  gutterEnterDisposable = terminal.onData((data) => {
    if (!gutterSettings.value.showTimestamps) return;
    if (!data.includes("\r") && !data.includes("\n")) return;
    if (!terminal) return;
    const buffer = terminal.buffer.active;
    stampLogicalLineContaining(buffer.baseY + buffer.cursorY, Date.now());
    scheduleGutterRecompute();
  });
  scheduleGutterRecompute();
}

function detachGutterListeners() {
  gutterRenderDisposable?.dispose();
  gutterRenderDisposable = undefined;
  gutterScrollDisposable?.dispose();
  gutterScrollDisposable = undefined;
  gutterResizeDisposable?.dispose();
  gutterResizeDisposable = undefined;
  gutterEnterDisposable?.dispose();
  gutterEnterDisposable = undefined;
  if (gutterRafId) {
    cancelAnimationFrame(gutterRafId);
    gutterRafId = 0;
  }
  gutterRows.value = [];
  gutterCellHeight.value = null;
}

function scheduleGutterRecompute() {
  if (!isGutterActive()) return;
  // 大输出保护生效期挂起重算（与下方积压跳帧同一目标，口径更早介入）。
  if (outputGate.mode === "strained") return;
  if (gutterRafId) return;
  gutterRafId = window.requestAnimationFrame(runGutterRecompute);
}

function runGutterRecompute() {
  gutterRafId = 0;
  const term = terminal;
  if (!term || !isGutterActive()) return;
  if (terminalWriteThrottle.pendingBytes > GUTTER_SUSPEND_PENDING_BYTES) return;
  if (outputGate.mode === "strained") return;
  gutterCellHeight.value = getRenderCellHeight(term);
  const buffer = term.buffer.active;
  // 首视口行的像素起点 = xterm 元素的 padding-top（gutter 文本与画布行对齐）。
  const paddingTop = term.element ? Number.parseFloat(window.getComputedStyle(term.element).paddingTop) : Number.NaN;
  gutterScreenOffsetTop = Number.isFinite(paddingTop) ? paddingTop : 0;
  gutterRows.value = computeGutterRows({
    cellHeight: gutterCellHeight.value,
    screenOffsetTop: gutterScreenOffsetTop,
    scrollTop: buffer.viewportY,
    buffer: { type: buffer.type, length: buffer.length, getLine: (y) => buffer.getLine(y) },
    rows: term.rows,
    timestamps: gutterTimestamps,
    showLineNumbers: gutterSettings.value.showLineNumbers,
    showTimestamps: gutterSettings.value.showTimestamps,
    timestampFormat: gutterSettings.value.timestampFormat,
  });
  trimTimestampMap(gutterTimestamps, Math.max(0, buffer.viewportY - GUTTER_TIMESTAMP_RETENTION_ROWS));
}

// 时间戳采集挂点：写入节流 sink 的 terminal.write 完成回调（xterm 解析完这批
// 合并字节后触发）。对新写入的逻辑行首盖 Date.now()；原地重写（进度条）只重盖
// 底行；buffer 变短（clear/重连）时整表重置。
function stampGutterWrittenRows() {
  if (!gutterSettings.value.showTimestamps) return;
  if (!terminal) return;
  const buffer = terminal.buffer.active;
  if (buffer.type === "alternate") return;
  const last = buffer.length - 1;
  if (last < 0) return;
  if (last < gutterLastStampedRow) resetGutterTimestamps();
  const now = Date.now();
  const from = Math.max(0, gutterLastStampedRow);
  for (let row = from; row <= last; row++) {
    if (buffer.getLine(row)?.isWrapped) continue;
    gutterTimestamps.set(row, now);
  }
  gutterLastStampedRow = last;
}

// 回车重盖：光标所在逻辑行（含 wrapped 向上回溯）整体盖为回车时刻。
function stampLogicalLineContaining(absoluteRow: number, atMs: number) {
  if (!terminal) return;
  const buffer = terminal.buffer.active;
  if (buffer.type === "alternate") return;
  let row = Math.max(0, Math.min(absoluteRow, buffer.length - 1));
  let scanned = 0;
  while (row > 0 && scanned < 2048 && buffer.getLine(row)?.isWrapped) {
    row -= 1;
    scanned += 1;
  }
  gutterTimestamps.set(row, atMs);
}

function resetGutterTimestamps() {
  gutterTimestamps.clear();
  gutterLastStampedRow = -1;
}

// gutter 设置变化：挂/摘挂子 + 宽度变化后重算终端列宽（xterm 左 padding 随
// --dbx-gutter-width 变化，FitAddon 需要重新 fit）。
watch(gutterSettings, () => {
  if (terminal && isGutterActive()) attachGutterListeners();
  else detachGutterListeners();
  scheduleGutterRecompute();
  scheduleFit();
}, { deep: true });

// 设置页增量上抛：归一化 → sidecar 持久化（watcher 即时挂/摘）。
function updateActionLinksSettings(patch: { enabled?: boolean; matchers?: Partial<ActionLinkMatcherToggles> }) {
  const next = sanitizeActionLinksSettings({
    enabled: patch.enabled ?? actionLinksSettings.value.enabled,
    matchers: { ...actionLinksSettings.value.matchers, ...(patch.matchers ?? {}) },
  });
  actionLinksSettings.value = next;
  void persistTerminalFeaturePrefs({
    action_links_enabled: next.enabled,
    action_links_matchers: { ipv4: next.matchers.ipv4, host_port: next.matchers.hostPort, archive: next.matchers.archive },
  });
}

function updateGutterSettings(patch: { showLineNumbers?: boolean; showTimestamps?: boolean; timestampFormat?: string }) {
  const next = sanitizeGutterSettings({ ...gutterSettings.value, ...patch });
  gutterSettings.value = next;
  void persistTerminalFeaturePrefs({
    terminal_show_line_numbers: next.showLineNumbers,
    terminal_show_timestamps: next.showTimestamps,
    terminal_timestamp_format: next.timestampFormat,
  });
}


watch(() => session.value?.sessionId, (next, previous) => {
  if (next !== previous) {
    // 录制挂在具体 session 上：换会话后本端标记复位（后端随旧会话自动收尾）。
    recordingActive.value = false;
    cancelRecordCountdown();
    stopRecordingClock();
  }
});


// 审计日志查看：状态/拉取/清空确认收口在 composables/useAuditLogViewer。
const {
  auditEntries,
  auditLoading,
  auditLoadFailed,
  auditTruncated,
  auditKindFilter,
  loadAuditEntries,
  openAuditLog,
  visibleAuditEntries,
  auditClearOpen,
  auditClearSubmitting,
  confirmAuditClear,
  auditTime,
  auditRowKindClass,
} = useAuditLogViewer({ locale, auditOpen });


// —— SFTP 单栏文件树：懒加载/祖先路径揭示/目录跟随/显示隐藏切换收口在
// composables/useSftpTreeWorkspace；后段声明的导航/页签/重命名依赖惰性注入 ——
const {
  listSftpDirectory,
  loadSftpDetails,
  clearSftpDetailCache,
  revealSftpPath,
  refreshExpandedSftpTree,
  refreshSftpAfterMutation,
  initializeSftpTree,
  followSftpPath,
  toggleSftpTreeNode,
  openSftpTreeNode,
  commitTreeRename,
  resetSftpTree,
} = useSftpTreeWorkspace({
  session, sudoMode, entries, currentPath, sftpListingLoaded, sftpShowHidden,
  sftpTreeRoot, selectedUris, selectedPath, lastClickedUri, sftpErrorKey,
  loadedTreeEntries, sftpTreePane, sftpFiltersActive, sftpTypeFilter,
  sftpPaneOpen, isRdpConnection, localUiMode, isLocalMode, connected,
  followDirectory, activePaneCwd, sftpHomePath, showError,
  loadDirectory: (path, fromTerminal, quiet) => loadDirectory(path, fromTerminal, quiet),
  refreshSftpHomePath: () => refreshSftpHomePath(),
  clearSftpSearch: () => clearSftpSearch(),
  openEditorTab: (entry) => openEditorTab(entry),
  setRenameDraft: (value) => { renameDraft.value = value; },
  commitRenameEntry: (entry) => commitRename(entry),
});

watch(terminalCwd, (cwd) => {
  if (!followDirectory.value || !cwd || !connected.value) return;
  void followSftpPath(cwd);
});

// SFTP 文件树右键菜单与上传后增量刷新：收口在 composables/useSftpFolderActions
// （导航/树工作区经下方/上方既有 composable 回调接线）。
const {
  onFileAreaContextMenu,
  folderMenuAction,
  refreshSftpFolder,
  refreshSftpAfterUpload,
} = useSftpFolderActions({
  terminalMenuOpen,
  fileMenu,
  session: () => session.value,
  currentPath,
  sftpTreeRoot,
  sftpShowHidden,
  mkdirTargetDir,
  openMkdirDialog: (targetDir) => {
    mkdirTargetDir.value = targetDir;
    operationDraft.value = "";
    operationDialog.value = "mkdir";
  },
  beginSymlinkCreate: (targetDir) => beginSymlinkCreate(targetDir),
  chooseUpload: (targetDir) => { void chooseUpload(targetDir); },
  chooseFolderUpload: (targetDir) => chooseFolderUpload(targetDir),
  openNewFileDialog: (targetDir) => openNewFileDialog(targetDir),
  loadDirectory: () => loadDirectory(),
  revealSftpPath: (path, opts) => revealSftpPath(path, opts),
  listSftpDirectory: (path) => listSftpDirectory(path),
  refreshExpandedSftpTree: () => refreshExpandedSftpTree(),
  showError: (cause) => showError(cause),
});

/** 通用剪贴板写入 + 已复制提示（行菜单/侧栏菜单共用；失败走 sftp 错误条）。 */
function copyTextToClipboard(value: string, noticeKey: string, values?: Record<string, string | number>) {
  void writeClipboardText(value, clipboardDeps())
    .then(() => showNotice(t(noticeKey, values)))
    .catch((cause) => showError(cause instanceof Error ? cause : new Error(String(cause))));
}

// R3-P1-3：目录列表加载的单调请求序号。慢链路下"先发 A 后发 B、A 晚到"
// 会把列表/路径/历史整体回跳；只有最新请求允许落地，过期响应整体丢弃。
// SFTP 目录导航：loadDirectory 枢纽/sudo/路径跳转收口在 composables/useSftpNavigation。
const {
  loadHome,
  refreshSftpHomePath,
  loadDirectory,
  sftpSoftError,
  toggleSudoMode,
  enableSudoModeAndReload,
  goParent,
  setDirectoryTracking,
  goToPath,
  pathBarEditing, pathBarInputEl, pathBarDraft, pathCrumbs,
  beginPathBarEdit, cancelPathBarEdit, submitPathInput,
} = useSftpNavigation({
  t, showNotice, showError, session, sudoMode, connected, canWrite,
  currentPath, entries, loadingFiles, sftpError, sftpErrorOpen, sftpErrorKey, sftpHomePath,
  visibleColumns, followDirectory, directoryParser, selectedPath, directoryTrackingSupported,
  clearRowSelection: () => clearRowSelection(), parentPath, normalizeRemotePath,
  getPendingTerminalInput: () => pendingTerminalInput,
  persistState, refreshDiskUsage, persistDirectoryFollowPref,
  resetLinkTargets: () => { linkTargets.value = {}; },
  hydrateLinkTargets: (list, isCurrent) => hydrateLinkTargets(list, isCurrent),
  rememberPathHistory: (path) => rememberPathHistory(path),
  closePathHistoryMenu: () => { pathHistoryOpen.value = false; },
  onListingLoaded: () => { sftpListingLoaded.value = true; },
});

// 右侧栏三视图/分栏顺序开关：收口在 composables/useWorkbenchPanels。
// 指标视图的 open/close 由后段 useMetricsPanel 提供，按 getter 惰性求值。
const {
  togglePaneOrder,
  openRightPanel,
  closeRightPanel,
  toggleSftpPane,
  toggleMetricsPanel,
  toggleDockerPanel,
} = useWorkbenchPanels({
  paneOrder, sftpPaneOpen, rightPanelView, localUiMode, isRdpConnection, connected,
  metricsOpen: () => metricsOpen.value,
  openMetrics: () => openMetrics(),
  closeMetrics: () => closeMetrics(),
  persistState, scheduleFit,
});

// 下载/传输/命令建议偏好与水合、下载目标/冲突询问链已收口在
// composables/useWorkbenchPrefs（见上方早期声明点）。视图/终端行为偏好
// （SFTP 默认/自动连接、目录跟随、终端行为与快捷键）已收口在
// composables/useWorkbenchViewPrefs（见早期声明点）。

function startDividerDrag(event: PointerEvent) {
  const container = paneContainer.value;
  if (!container) return;
  const pointerId = event.pointerId;
  // 纵向布局（强制上下 / 窄屏断点 / Docker 底栏）把手沿 Y 轴拖拽：
  // 按下时定轴，拖动中不变；Docker 使用独立比例，避免改动 SFTP 分栏。
  const vertical = paneLayout.value.direction === "column";
  const targetRatio = dockerPanelOpen.value ? dockerSplitRatio : splitRatio;
  const move = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    const bounds = container.getBoundingClientRect();
    if (vertical) {
      // splitRatio 是终端占比；纵向终端恒在上，把手下移 = 终端变高。
      const fromTop = ((next.clientY - bounds.top) / bounds.height) * 100;
      targetRatio.value = Math.max(35, Math.min(80, fromTop));
    } else {
      const fromLeft = ((next.clientX - bounds.left) / bounds.width) * 100;
      const terminalPercent = paneOrder.value === "terminal-left" ? fromLeft : 100 - fromLeft;
      targetRatio.value = Math.max(35, Math.min(80, terminalPercent));
    }
    scheduleFit();
  };
  // pointercancel 触发时指针已被移出 active 集合：releasePointerCapture 对非
  // active 指针会抛 NotFoundError，必须先摘监听、再守卫释放，否则监听器泄漏、
  // 容器内后续鼠标移动会持续重算分栏（触屏拖动被浏览器接管时必现）。
  const release = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    container.removeEventListener("pointermove", move);
    container.removeEventListener("pointerup", release);
    container.removeEventListener("pointercancel", release);
    if (container.hasPointerCapture(pointerId)) {
      try { container.releasePointerCapture(pointerId); } catch { /* 已被隐式释放 */ }
    }
    persistState();
  };
  container.setPointerCapture(pointerId);
  container.addEventListener("pointermove", move);
  container.addEventListener("pointerup", release);
  container.addEventListener("pointercancel", release);
}

// 分栏把手键盘通道（pointer 拖拽对键盘用户不可达）：横向布局左右方向键
// ±16px，纵向布局上下方向键 ±16px，Shift 加速 ×3。`delta > 0` 表示「把手
// 向右/向下移」（终端占比随之增大/减小由 nudgeSplitDivider 按轴解释）。
const DIVIDER_KEY_STEP_PX = 16;
function onDividerKeydown(event: KeyboardEvent, nudge: (deltaPx: number) => void) {
  const vertical = paneLayout.value.direction === "column";
  const horizontalKeys = event.key === "ArrowLeft" || event.key === "ArrowRight";
  const verticalKeys = event.key === "ArrowUp" || event.key === "ArrowDown";
  if (vertical ? !verticalKeys : !horizontalKeys) return;
  event.preventDefault();
  const magnitude = event.shiftKey ? DIVIDER_KEY_STEP_PX * 3 : DIVIDER_KEY_STEP_PX;
  const forward = vertical ? event.key === "ArrowDown" : event.key === "ArrowRight";
  nudge(forward ? magnitude : -magnitude);
}

function nudgeSplitDivider(deltaPx: number) {
  const container = paneContainer.value;
  if (!container) return;
  const bounds = container.getBoundingClientRect();
  const targetRatio = dockerPanelOpen.value ? dockerSplitRatio : splitRatio;
  // splitRatio 是终端占比：横向 terminal-left 时把手右移 = 终端变宽（sftp-left
  // 反向）；纵向终端恒在上，把手下移 = 终端变高。
  if (paneLayout.value.direction === "column") {
    if (!bounds.height) return;
    targetRatio.value = Math.max(35, Math.min(80, targetRatio.value + (deltaPx / bounds.height) * 100));
  } else {
    if (!bounds.width) return;
    const sign = paneOrder.value === "terminal-left" ? 1 : -1;
    targetRatio.value = Math.max(35, Math.min(80, targetRatio.value + (deltaPx / bounds.width) * 100 * sign));
  }
  scheduleFit();
  persistState();
}


// 文件预览：收口在 composables/useFilePreview。
const {
  previewOpen, previewTitle, previewText, previewLoading, previewPath, previewSize,
  previewEditable, previewDraft, previewSaving, previewMode, previewImageUrl,
  previewImageZoomed, previewTruncated, previewDirty, previewEditableAllowed, jsonPreviewState,
  openEntry, openImagePreview, closePreview, beginPreviewEdit, cancelPreviewEdit, savePreview,
  MAX_INLINE_PREVIEW_BYTES,
} = useFilePreview({
  t, showNotice, showError, session, sudoMode, canWrite,
  loadDirectory: (path?: string) => loadDirectory(path),
  downloadEntry: (entry, forceSudo?: boolean) => downloadEntry(entry, forceSudo),
  pathFromUri,
});

// ---------------------------------------------------------------------------
// 终端区编辑器页签（树内双击/Enter 打开）：页签状态/远端读写收口在
// composables/useEditorTabs；预览弹窗与右键「预览」保持既有 useFilePreview。
// ---------------------------------------------------------------------------
const {
  tabs: editorTabs,
  activeId: activeEditorId,
  hasTabs: hasEditorTabs,
  openEntry: openEditorTab,
  activate: activateEditorTab,
  updateText: updateEditorText,
  saveTab: saveEditorTab,
  saveActive,
  closeTab: closeEditorTab,
  handleRename: handleEditorRename,
  closeTabsUnder: closeEditorTabsUnder,
} = useEditorTabs({
  t, showNotice, showError, session, sudoMode, canWrite, pathFromUri,
  // 保存成功后刷新当前列表、所有已展开树节点并重新揭示当前路径；失败由
  // useEditorTabs 统一转全局错误条，保存本身已成功不回滚脏标记。
  onSaved: refreshSftpAfterMutation,
});
// 编辑器分栏：比例/拖拽/键盘调整/焦点判定收口在 composables/useEditorSplitPane。
const {
  editorPaneRef,
  terminalStackRef,
  editorSplitRatio,
  startEditorDividerDrag,
  onEditorDividerKeydown,
  editorPaneContainsFocus,
} = useEditorSplitPane({ hasEditorTabs, scheduleFit });

// 压缩/解压：行内归档与解包收口在 composables/useSftpArchive。
const {
  archiveBusy,
  isArchiveName,
  archiveDirectoryName,
  archiveEntry,
  extractEntry,
} = useSftpArchive({
  t, showNotice, showError, session, currentPath, pathFromUri, joinRemote, loadDirectory,
  closeFileMenu: () => { fileMenu.value = undefined; },
});

// —— 打开方式子菜单与自定义编辑器命令（编辑器映射增强）：状态/动作收口在
// composables/useEditorOpenWith（声明点见 useExternalEdits 之后，编辑器配置
// ref 由该 composable 提供）——

/** 设置弹窗的外部编辑器适配器：权威态在本域（pluginStore 单键 + 探测缓存）。 */
const editorPrefsAdapter = {
  loadConfig: () => editorConfig.value,
  persistConfig: (value: EditorConfig) => updateEditorConfig(value),
  loadEditors: () => externalEditors.value,
  reloadEditors: () => ensureExternalEditors(),
};

// 行内重命名：提交/短路语义收口在 composables/useSftpRename。
const {
  renamingPath,
  renameDraft,
  renameSubmitting,
  beginRename,
  commitRename,
} = useSftpRename({
  t, showError, session, canWrite, sudoMode, selectedPath, pathFromUri, joinRemote, loadDirectory,
  onRenamed: (oldPath, newPath) => handleEditorRename(oldPath, newPath),
  onMutated: refreshSftpAfterMutation,
});

// 新建目录对话框收口在 composables/useSftpCreateDirectory。
const {
  operationDialog,
  operationDraft,
  createDirectory,
} = useSftpCreateDirectory({
  showError, session, sudoMode, currentPath,
  targetDir: mkdirTargetDir,
  joinRemote, loadDirectory,
  refreshAfterCreate: refreshSftpAfterUpload,
});


// ---------------------------------------------------------------------------
// SFTP 面板：搜索/多选/批量/新建文件/属性/路径历史/复制粘贴
// ---------------------------------------------------------------------------

// 路径历史：存储/记录/弹层开关收口在 composables/useSftpPathHistory。
const {
  pathHistoryOpen,
  pathHistories,
  currentPathHistory,
  rememberPathHistory,
  togglePathHistoryMenu,
} = useSftpPathHistory({ connectionId, closeToolbarPopovers });

// ---------------------------------------------------------------------------
// SFTP 路径书签：存储迁移/收藏/删除收口在 composables/useSftpBookmarks。
// ---------------------------------------------------------------------------
const {
  sftpBookmarks,
  bookmarkSaveOpen,
  bookmarkSaving,
  bookmarkLabelDraft,
  refreshBookmarks,
  toggleBookmarkSave,
  confirmBookmarkSave,
  removeBookmark,
} = useSftpBookmarks({
  t, showNotice, showError, connected, currentPath, pathHistoryOpen, closeToolbarPopovers,
});


// 删除与批量操作：收口在 composables/useSftpDelete。
const {
  confirmDelete,
  confirmBatchDelete,
  batchArchive,
} = useSftpDelete({
  t, showNotice, showError, session, sudoMode, currentPath,
  deleteTarget, deleteSubmitting, batchDeleteOpen, batchDeleteSubmitting, batchProgress,
  selectedEntries, pathFromUri, joinRemote, loadDirectory,
  clearRowSelection: () => clearRowSelection(),
  archiveBusy,
  onDeleted: (paths) => {
    for (const path of paths) closeEditorTabsUnder(path);
  },
  onMutated: refreshSftpAfterMutation,
});

function openNewFileDialog(targetDir?: string) {
  if (!connected.value || !canWrite.value) return;
  newFileTargetDir.value = targetDir;
  newFileDraft.value = "";
  newFileDialog.value = true;
}

async function createNewFile() {
  const sessionId = session.value?.sessionId;
  const name = newFileDraft.value.trim();
  if (!sessionId || !name || newFileSubmitting.value) return;
  const base = newFileTargetDir.value ?? currentPath.value;
  newFileSubmitting.value = true;
  try {
    await window.dbxPlugin.invoke(sudoMode.value ? "sudo/touch" : "sftp/touch", {
      sessionId,
      path: joinRemote(base, name),
    });
    newFileDialog.value = false;
    newFileTargetDir.value = undefined;
    showNotice(t("sftpNewFile.done", { name }));
    await refreshSftpAfterUpload(base);
  } catch (cause) {
    showError(cause);
  } finally {
    newFileSubmitting.value = false;
  }
}

// 文件属性对话框：stat/chmod 状态与动作收口在 composables/useSftpAttributes。
const {
  attrsTarget,
  attrsInfo,
  attrsOwnerDisplay,
  attrsLoading,
  attrsMode,
  attrsSubmitting,
  openAttributes,
  closeAttributes,
  saveAttributesPermissions,
} = useSftpAttributes({
  t, showNotice, showError, session, sudoMode, pathFromUri, loadDirectory,
  closeFileMenu: () => { fileMenu.value = undefined; },
});

// 选中与剪贴板粘贴：收口在 composables/useSftpSelectionClipboard。
const {
  clearRowSelection,
  selectFile,
  copySelectedEntries,
  pasteClipboard,
} = useSftpSelectionClipboard({
  t, showNotice, showError, session, canWrite, connectionId, currentPath,
  entries, visibleEntries: sftpTreeEntries, selectedPath, selectedUris, selectedUriSet, lastClickedUri,
  sftpClipboard, pasteBusy,
  fileMenu,
  closeFileMenu: () => { fileMenu.value = undefined; },
  joinRemote, remoteBasename, pathFromUri, loadDirectory,
});




// 外部编辑回传：watch 确认状态机/上传串行链收口在 composables/useExternalEdits。
const {
  externalEditBusy,
  activeExternalWatches,
  watchModifiedQueue,
  watchModifiedPrompt,
  alwaysUploadWatches,
  dismissWatchModified,
  uploadWatchedFileOnce,
  uploadWatchedFileAlways,
  handleWatchModified,
  joinLocalPath,
  openInExternalEditor,
  editorConfig,
  updateEditorConfig,
  externalEditors,
  ensureExternalEditors,
} = useExternalEdits({
  t, showNotice, showError, loadDirectory,
  session,
  openTransferPanel, transferTasks, pathFromUri, waitWhilePaused, cancelledTransferTasks, downloadChunkWaiters,
  waitForDownloadChunk: (taskId, offset) => waitForDownloadChunk(taskId, offset),
  waitForDownloadReady: (taskId, compression) => waitForDownloadReady(taskId, compression),
  probeLocalCapabilities: () => probeLocalCapabilities(),
  loadDownloadDir, copyTextToClipboard,
  closeFileMenu: () => { fileMenu.value = undefined; },
});

// 打开方式子菜单/自定义编辑器命令：收口在 composables/useEditorOpenWith。
const {
  availableExternalEditors,
  mappedEditorIdFor,
  openWithSystemEntry,
  openWithKnownEntry,
  openWithCustomEntry,
  customEditorOpen,
  customEditorName,
  customEditorCommand,
  customEditorRemember,
  customEditorPattern,
  beginCustomEditorCommand,
  commitCustomEditor,
} = useEditorOpenWith<SftpEntry>({
  editorConfig, externalEditors, updateEditorConfig, openInExternalEditor,
});


// 符号链接（P2-6）：对话框/提交/tooltip 缓存收口在 composables/useSymlink。
const {
  symlinkDialog,
  symlinkDraft,
  symlinkTargetDraft,
  symlinkSubmitting,
  linkTargets,
  beginSymlinkCreate,
  beginSymlinkEdit,
  commitSymlink,
  linkTargetTitle,
  hydrateLinkTargets,
} = useSymlink({
  t, showError, session, currentPath, pathFromUri, joinRemote, loadDirectory,
  refreshAfterCreate: refreshSftpAfterUpload,
});

const localDownloadDir = ref("");
const localCanSave = ref(false);
// 下载落盘链：收口在 composables/useSftpDownload。
const {
  probeLocalCapabilities,
  downloadEntry,
  waitForDownloadChunk,
  waitForDownloadReady,
  batchDownload,
  saveHostFile,
  revealTransferTarget,
  openTransferTarget,
} = useSftpDownload({
  t, showNotice, showError, session, entries, fileMenu,
  // 跨目录/被过滤折叠的选中项都按已加载树解析，批量下载不缩水。
  selectedEntries,
  localDownloadDir, localCanSave,
  transferTasks, cancelledTransferTasks, downloadChunkWaiters, waitWhilePaused,
  loadDownloadDir, loadDownloadUseDefaultDir, askDownloadTarget,
  applyChosenDirAsDefault, resolveDownloadConflictFor,
  openTransferPanel: () => openTransferPanel(),
  pathFromUri,
  // sudo 模式 + 可写连接时批量下载对普通文件走 sudo 车道（与右键单项一致）。
  sudoDownload: () => sudoMode.value && canWrite.value,
});

// —— 断点续传：暂停/恢复 + 可续传上传 ———


// —— 上传重复目标策略（P1-5）：上传前 sftp/exists 预检，按 transfer_duplicate_policy
// 决定重命名 / 覆盖 / 询问。询问弹窗支持「应用到全部」（批次内生效）。——

/** 批次级「应用到全部」决策：undefined = 尚未决定（逐个询问）。 */



/**
 * Focus the SFTP workbench itself when the user clicks its blank area. This
 * gives Ctrl/Cmd+V a stable native paste target without stealing focus from
 * path/search inputs or toolbar controls.
 */
function focusSftpPaneOnPointerDown(event: PointerEvent) {
  const target = event.target;
  if (target instanceof Element && target.closest("button, input, select, textarea, a, [contenteditable='true']")) return;
  sftpPane.value?.focus({ preventScroll: true });
}

// 上传链：五路上传入口/重复策略/拖拽落点收口在 composables/useUploadChain。
const {
  uploadInput,
  folderUploadInput,
  folderUploadSupported,
  folderUploadProgress,
  fileBatchProgress,
  dropUploadPrompt,
  dropUploadTarget,
  dropCwdTarget,
  dropCwdTargetWire,
  dropUploadPathInput,
  dropUploadPathInputEl,
  uploadDuplicatePrompt,
  uploadDuplicateApplyAll,
  resolveUploadDuplicateName,
  resolveUploadDuplicate,
  onUploadInput,
  chooseFolderUpload,
  onFolderUploadInput,
  onSftpClipboardPaste,
  handleHostFileDrop,
  onDrop,
  onSftpDragEnter,
  onSftpDragLeave,
  onTerminalDragEnter,
  onTerminalDragLeave,
  onTerminalDrop,
  chooseUpload,
  confirmDropUpload,
  resolveDropUpload,
} = useUploadChain({
  t, showNotice, showError, session, connected, canWrite, currentPath, sftpHomePath,
  terminalCwd, sftpNameEncodingState, dragActive, terminalDragActive, sftpPaneOpen: sftpViewVisible, terminalTransferBusy,
  uploadSource, loadTransferConcurrency, loadTransferMaxActive, loadTransferDuplicatePolicy, transferDuplicateState,
  transferTasks,
  terminal: () => terminal,
  loadDirectory,
  refreshAfterUpload: refreshSftpAfterUpload,
  openTransferPanel,
  joinRemote,
  // 与 useTransferQueue 同源判定：sudo 模式 + 可写连接（UploadSudo 车道）。
  sudoUpload: () => sudoMode.value && canWrite.value,
});

/** 树行 drop：文件夹行为目标目录；文件行回落当前目录（行本身不是落点）。
 *  事件必须同步交给上传链，webkitGetAsEntry 的 dataTransfer.items 只在
 *  当前 tick 内有效。 */
function onSftpTreeDrop(node: SftpFileTreeNode, event: DragEvent) {
  if (!sftpViewActive.value) return;
  onDrop(event, node.kind === "directory" ? node.path : currentPath.value);
}

// 沙箱 iframe 的剪贴板依赖注入：宿主桥是 optional 且现网宿主未提供，
// 缺失/拒绝时由 clipboardBridge 逐级降级（见 lib/clipboardBridge.ts）。
function clipboardDeps(): ClipboardDeps {
  return {
    bridge: window.dbxPlugin.clipboard ?? null,
    nativeClipboard: typeof navigator !== "undefined" ? (navigator as Navigator & { clipboard?: ClipboardDeps["nativeClipboard"] }).clipboard ?? null : null,
  };
}

async function copyTerminalSelection() {
  // 选区以菜单打开瞬间的快照优先：弹层接管焦点后实时 getSelection() 可能
  // 已被清空（clipboard 档直通路径不受影响，仍取实时选区）。
  const text = menuSelectionSnapshot.value || terminal?.getSelection() || "";
  menuSelectionSnapshot.value = "";
  // 无论成败都关菜单：静默早退会让菜单挂着，看起来像"复制无效"。
  terminalMenuOpen.value = false;
  if (!text) {
    showError(new Error(t("terminalCopyNoSelection")), "terminal");
    terminal?.focus();
    return;
  }
  terminalCopyCache.set(text);
  try {
    await writeClipboardText(text, clipboardDeps());
    showNotice(t("terminalCopied"));
  } catch {
    showError(new Error(t("terminalCopyUnavailable")), "terminal");
  }
  terminal?.focus();
}

async function pasteTerminal() {
  terminalMenuOpen.value = false;
  let clipboardText: string | null = null;
  let clipboardReadBlocked = false;
  try {
    clipboardText = await readClipboardText(clipboardDeps());
  } catch {
    // 宿主桥缺失 + 沙箱拒绝读：降级到插件视图内的复制副本（选中复制、
    // 菜单复制、远端 OSC 52 都会写入），XShell 式「选中→右键」因此闭环。
    clipboardReadBlocked = true;
  }
  const text = clipboardReadBlocked
    ? resolveTerminalPasteText({ cachedText: terminalCopyCache.get(), selectionText: terminal?.getSelection() || null })
    : clipboardText || null;
  if (!text) {
    // 无任何可用来源：引导走原生 paste 快捷键（Ctrl+V 走 paste 事件，不依赖读权限）。
    showError(new Error(t("terminalPasteUseShortcut")), "terminal");
    terminal?.focus();
    return;
  }
  await sendConfirmedPaste(text);
}

function interceptTerminalPaste(event: ClipboardEvent) {
  event.preventDefault();
  event.stopPropagation();
  // 粘贴路径不出 ghost：立即消除（onData 不经手粘贴正文，下一次可打印键入重算）。
  hideGhostSuggestion();
  const text = event.clipboardData?.getData("text/plain") || "";
  if (!text) return;
  void sendConfirmedPaste(text);
}

// —— 右键菜单「在线搜索」：引擎表/搜索动作收口在 composables/useOnlineSearch ——
const {
  ctxSearchEnginesText,
  ctxSearchEngines,
  updateCtxSearchEngines,
  searchSelectionOnline,
  searchTextOnline,
} = useOnlineSearch({
  terminal: () => terminal,
  terminalMenuOpen,
  copyTextToClipboard,
  persistPrefs: (patch) => persistTerminalFeaturePrefs(patch),
});

// —— 背景图：状态/持久化/WebGL 渲染器联动收口在 composables/useWallpaper ——

async function sendConfirmedPaste(text: string) {
  // 粘贴文本变换的唯一收口：所有粘贴路径（原生 Ctrl+V、右键/菜单粘贴、中键粘贴）
  // 都经这里，保证「去首尾空白 / 换行折空格」只实现一次、不会分叉。
  // 默认两开关均为关闭，因此这里的默认行为与改动前逐字节一致。
  const payload = transformPasteText(text, terminalBehavior.value);
  if (!payload) return;
  const accepted = await confirmRiskyPaste(payload);
  if (!accepted) {
    terminal?.focus();
    return;
  }
  // 会话门按活动传输判定（sendTerminalBytes 已分派 SSH/本地/Telnet/串口）：
  // 只看 SSH session 会让本地 Shell（及 Telnet/串口）的粘贴静默无效果——
  // 确认弹窗照出、字节却永不落终端（用户感知为「右键粘贴没动静」）。
  if (terminalTransferBusy.value) return;
  if (!session.value && !localSession.value && !telnetSession.value && !serialSession.value) return;
  trackPendingInput(payload);
  sendTerminalBytes(new TextEncoder().encode(payload));
  terminal?.focus();
}

let pasteConfirmAskChain: Promise<unknown> = Promise.resolve();

function confirmRiskyPaste(text: string): Promise<boolean> {
  // 多行/超长粘贴警告可关（对标 Tabby「Clipboard → Warn on multi-line paste」）；
  // 危险命令（rm -rf 等）的确认是安全兜底，不受该开关约束，永远要确认。
  const confirmation = buildPasteConfirmation(text, { warnOnMultiline: terminalBehavior.value.warnOnMultilinePaste });
  if (!confirmation.required) return Promise.resolve(true);
  const asked = pasteConfirmAskChain.then(
    () =>
      new Promise<boolean>((resolve) => {
        // 工作台已卸载：排队中的询问直接拒绝，不留孤儿弹窗态。
        if (disposed) {
          resolve(false);
          return;
        }
        pasteConfirmResolver = resolve;
        pasteConfirm.value = confirmation;
      }),
  );
  pasteConfirmAskChain = asked.catch(() => undefined);
  return asked;
}

function resolvePasteConfirm(accepted: boolean) {
  pasteConfirm.value = undefined;
  const resolve = pasteConfirmResolver;
  pasteConfirmResolver = undefined;
  resolve?.(accepted);
}

function selectAllTerminal() {
  terminal?.selectAll();
  terminalMenuOpen.value = false;
  terminal?.focus();
}

function clearTerminal() {
  terminal?.clear();
  closeHistoryPanel();
  resetGutterTimestamps();
  terminalMenuOpen.value = false;
  terminal?.focus();
}

function chooseZmodem() {
  terminalMenuOpen.value = false;
  zmodemInput.value?.click();
}


// 命令执行弹窗：状态/执行/历史浏览收口在 composables/useCommandDialog。
const {
  commandOpen,
  commandDraft,
  commandUseSudo,
  commandExecId,
  commandResult,
  commandError,
  openCommandDialog,
  handleCommandInputKeydown,
  browseCommandHistoryUp,
  browseCommandHistoryDown,
  rerunHistoryCommand,
  clearCommandHistory,
  runCommand,
  cancelCommand,
} = useCommandDialog({
  showError, session, commandRunning, commandHistory, commandHistoryIndex, commandHistoryBackup, persistCommandHistory,
  commandHistoryTimes, pushTerminalCommandHistory, persistCommandHistoryTimes,
});

// 批量发送：状态/发送/跨工作台广播收口在 composables/useBatchSend。
const {
  batchBarOpen,
  batchTargetsOpen,
  batchLoading,
  batchSending,
  batchTargets,
  batchSelected,
  batchDraft,
  batchError,
  batchSummary,
  batchQuickOpen,
  batchQuickSearch,
  filteredBatchQuickCommands,
  groupedBatchQuickCommands,
  batchQuickPickName,
  batchSaveMode,
  batchSaveName,
  batchSaving,
  batchHistoryIndex,
  batchHistoryBackup,
  batchBarSourceId,
  toggleBatchBar,
  broadcastBatchBarState,
  applyRemoteBatchBarState,
  refreshBatchTargets,
  toggleBatchTargetsPopover,
  toggleBatchTargetId,
  pickBatchTargets,
  pickBatchQuick,
  browseBatchHistoryUp,
  browseBatchHistoryDown,
  batchSessionLabel,
  sendBatchCommand,
  dismissBatchResult,
  openBatchBarSave,
  confirmBatchBarSave,
  cancelBatchBarSave,
} = useBatchSend({
  showError,
  connected,
  session,
  localSession,
  quickCommands,
  commandHistory,
  panelSurface,
  sftpPaneOpen,
  transferPanelOpen,
  persistCommandHistory,
  pushTerminalCommandHistory,
  confirmRiskyPaste,
  terminalInputQueue,
  closeOtherPopovers: closeToolbarPopovers,
});

// 连接建立后刷新 AI 模式；断开时收起模式弹出层。
watch(connected, (value) => {
  if (value) {
    void refreshAgentMode();
  } else {
    agentModeOpen.value = false;
  }
});

// ---------------------------------------------------------------------------
// 连接信息面板（只读）

// ---------------------------------------------------------------------------

// 工具条弹层 toggle（quick/session/connection-info/columns/rdp/palette/batch/
// transfer）已随菜单状态迁 composables/useTerminalMenus；shell 选项视图切换
// 仍留在本地 shell 偏好域。
// 下拉内切到 shell 选项视图：同弹层视图切换，菜单不关，顺带懒加载 shell 列表。
function openSessionShellPrefs() {
  sessionMenuShellOpen.value = true;
  void openLocalShellPrefs();
}

// 认证方式：读取 ssh/sessions/list 当前会话行的 authMethod（只读方法名，
// 不含任何凭据材料）。失败时面板显示占位符，不影响其他信息。
// 连接信息面板：延迟探测/认证方式只读名称收口在 composables/useConnectionInfo。
const {
  connectionInfoOpen,
  connectionLatency,
  connectionLatencyBusy,
  connectionLatencyFailed,
  connectionAuthMethod,
  refreshConnectionAuthMethod,
  measureLatency,
} = useConnectionInfo({ t, session, connectionReadOnly });



// 录制目标会话：SSH 连接优先（与既有语义一致）；本地终端（运行中的 shell）
// 走 local/recording 家族——sidecar 侧与 ssh/recording 同形同目录。
// SSH 断开 / 本地 shell 退出时目标消失，录制态由 useRecording 内的 watch 复位。
const recordingTarget = computed<{ kind: "ssh" | "local"; sessionId: string } | undefined>(() => {
  if (connected.value && session.value) return { kind: "ssh", sessionId: session.value.sessionId };
  if (localSession.value) return { kind: "local", sessionId: localSession.value.sessionId };
  return undefined;
});

// 终端录制 + 回放（asciicast v2）：状态与交互收口在 composables/useRecording。
const {
  recordingActive,
    recordingsOpen,
    recordings,
    recordingsLoading,
    recordingDeleteTarget,
    recordingDeleteSubmitting,
    recordingExportingId,
    replayState,
    replayPlaying,
    replaySpeed,
    replayPlayheadMs,
    replayExporting,
    replayHost,
    recordCountdown,
    recordingStartedAt,
    recordingElapsedSec,
    beginRecordCountdown,
    cancelRecordCountdown,
    startRecordingClock,
    stopRecordingClock,
    startRecordingNow,
    toggleRecording,
    loadRecordings,
    toggleRecordings,
    revealRecording,
    exportRecordingTranscript,
    recordingsQuery,
    recordingHits,
    recordingSearchBusy,
    filteredRecordings,
    runRecordingSearch,
    clearRecordingsSearch,
    deleteRecording,
    confirmRecordingDelete,
    recordingClearAllOpen,
    recordingClearAllSubmitting,
    confirmRecordingClearAll,
    REPLAY_EVENT_CAP,
    replayDurationMs,
    loadReplayEvents,
    openReplay,
    closeReplay,
    stopReplayLoop,
    replayFrame,
    toggleReplayPlay,
    onReplaySeek,
    exportRecordingGif,
    exportReplayGif,
    exportRecordingFromList,
    formatDuration,
    formatRecordedAt,
} = useRecording({
  t, showNotice, showError, recordingTarget, appearance, terminalTheme, openTransferTarget, revealTransferTarget,
  terminalFontOverride, terminalAppearance,
  hostFontFamily: () => hostTerminalFontFamily(appearance.value),
  loadDownloadDir, loadDownloadUseDefaultDir, askDownloadTarget, applyChosenDirAsDefault, resolveDownloadConflictFor,
  probeLocalCapabilities, saveHostFile,
  closeMetrics: () => closeMetrics(),
});
// 后端事件分派：收口在 composables/usePluginEvents（30 个 method 分支整体
// 平移，method 键互斥、命中即 return，与原 if 链语义一致）。
const { handlePluginEvent } = usePluginEvents({
  t, session, terminalState, showError, showNotice,
  terminalDiag, batchBarSourceId, applyRemoteBatchBarState,
  hostKeyPrompt, rdpCertPrompt, rdpCertRemember, connectLog,
  scheduleSessionReconnect, reconnectPending,
  localSession, markLocalExited,
  telnetSession, telnetState, telnetError, markTelnetClosed,
  vncSession, vncState, vncError, markVncClosed,
  serialSession, markSerialClosed, serialUpload,
  rdpSession, rdpState, rdpSurface, rdpClipboardChunks,
  agentPromptQueue, agentRunning, recordingActive, startRecordingClock,
  handleWatchModified, uploadAckWaiters, updateTransfer,
});

// 宿主桥事件入口：env 事件由 composable 内窄化排除。
function handleEvent(event: DbxPluginEvent) {
  handlePluginEvent(event);
}


// 指标卡 + 进程管理（F2）：状态/轮询/采样收口在 composables/useMetricsPanel。
const {
  metricsOpen,
    processesOpen,
    processRows,
    processLoading,
    processSortKey,
    metrics,
    metricsLoading,
    metricsError,
    metricSamples,
    recordMetricSamples,
    metricsRxSparkline,
    metricsTxSparkline,
    metricsCpuSparkline,
    metricsMemSparkline,
    visibleDiskMounts,
    visibleNetworkInterfaces,
    metricsDistroBadge,
    refreshMetrics,
    openMetrics,
    closeMetrics,
    backfillMetricsHistory,
    toggleProcessPanel,
    refreshProcessList,
    sortedProcessRows,
    PROCESS_VISIBLE_LIMIT,
    visibleProcessRows,
    killProcessRow,
} = useMetricsPanel({ t, showNotice, showError, session, transferPanelOpen, recordingsOpen });

// 右侧栏指标视图快照：组件只接收解包后的普通值，避免把一整套 ref 当 props。
const metricsView = computed<ServerMetricsView>(() => ({
  metrics: metrics.value,
  loading: metricsLoading.value,
  error: metricsError.value,
  distroBadge: metricsDistroBadge.value,
  cpuSparkline: metricsCpuSparkline.value,
  memSparkline: metricsMemSparkline.value,
  rxSparkline: metricsRxSparkline.value,
  txSparkline: metricsTxSparkline.value,
  cpuLast: metricSamples.cpu.at(-1) ?? 0,
  memLast: metricSamples.mem.at(-1) ?? 0,
  disks: visibleDiskMounts.value,
  networks: visibleNetworkInterfaces.value,
  processesOpen: processesOpen.value,
  processRows: processRows.value,
  processLoading: processLoading.value,
  processSortKey: processSortKey.value,
  sortedProcessTotal: sortedProcessRows.value.length,
  visibleProcessRows: visibleProcessRows.value,
}));

function setMetricsProcessSort(key: MetricsProcessSortKey) {
  processSortKey.value = key;
}

// 指标轮询被外部收起（录制/传输互斥会调用 closeMetrics）时同步收起右栏，
// 否则会留下一个不再刷新的指标快照挡住终端。
watch(metricsOpen, (open) => {
  if (!open && rightPanelView.value === "metrics" && sftpPaneOpen.value) closeRightPanel();
});

// —— dock 面板左上角系统信息（CPU/内存/网速）：状态/轮询/采样收口在
// composables/useDockMetrics ——
const {
  localTerminalActive,
  refreshPanelMetrics,
  panelMetricsVisible,
  panelMetricsFailed,
  panelMetricsErrorTitle,
  panelCpuPercent,
  panelMemPercent,
  panelNetRates,
} = useDockMetrics({
  t, isLocalMode, localShellRestored, metrics, metricsError, metricsLoading,
  metricsOpen, connected, panelSurface, refreshMetrics,
});

async function refreshDiskUsage() {
  if (!session.value) return;
  const path = currentPath.value;
  const usage = await window.dbxPlugin
    .invoke<DiskUsage>("sftp/diskUsage", { sessionId: session.value.sessionId, path }, { timeoutMs: 30_000 })
    .catch(() => undefined);
  // fire-and-forget 无 epoch 守卫：慢响应/失败落回时目录可能已切换，
  // 按发起时的 path 口径校验，过期快照不回写页脚。
  if (path !== currentPath.value) return;
  diskUsage.value = usage;
}

// 权限矩阵换算与 chmod 对话框状态/提交：收口在 composables/useSftpChmod
// （PERM_ROLES/PERM_COLUMNS 与八进制草稿换算同文件）。
const {
  chmodTarget,
  chmodDraft,
  chmodSubmitting,
  toggleChmodPerm,
  toggleAttrsPerm,
  beginChmod,
  confirmChmod,
} = useSftpChmod<SftpEntry>({
  canWrite: () => canWrite.value,
  session: () => session.value,
  sudoMode: () => sudoMode.value,
  attrsMode,
  closeFileMenu: () => { fileMenu.value = undefined; },
  loadDirectory: () => loadDirectory(),
  clearSftpDetailCache: () => clearSftpDetailCache(),
  showNotice: (message) => showNotice(message),
  showError: (cause) => showError(cause),
  t: (key) => t(key),
});

function openSettings() {
  settingsOpen.value = true;
  void probeLocalCapabilities();
}


/**
 * 工具栏弹出层互斥族统一收口（round2：收敛五处 + 三处模板内联的手抄互斥清单）。
 * 打开任一同族弹出层前调用，先关掉全部兄弟弹出层与右键菜单，再由各 toggle
 * 设定自身状态。族成员 = 模板 class="popover" 的九个弹出层（quick-commands /
 * agent-mode / highlight-rules / connection-info / columns / transfer /
 * batch-targets / bookmark-save / path-history）。语义差异说明：右侧栏的
 * 指标/容器视图（closeRightPanel 会停掉指标轮询并卸载容器面板）与批量保存态
 * （batchSaveMode，cancelBatchBarSave 带草稿清理）不属于本族，仍由 Esc 链收口；
 * batch-targets 弹层另有 mousedown-capture 点空白收起，此处再关一次幂等无害。
 */
function closeToolbarPopovers() {
  fileMenu.value = undefined;
  terminalMenuOpen.value = false;
  transferHistoryMenu.value = undefined;
  transferPanelOpen.value = false;
  columnsOpen.value = false;
  pathHistoryOpen.value = false;
  quickMenuOpen.value = false;
  connectionInfoOpen.value = false;
  agentModeOpen.value = false;
  bookmarkSaveOpen.value = false;
  batchTargetsOpen.value = false;
  sessionMenuOpen.value = false;
  sessionMenuShellOpen.value = false;
  sessionMenuShellPickOpen.value = false;
  localShellSurfaceOpen.value = false;
  panelPaletteOpen.value = false;
  rdpPasteOpen.value = false;
  rdpResolutionOpen.value = false;
  batchQuickOpen.value = false;
  batchTargetsOpen.value = false;
}

/**
 * 弹层焦点管理（P1-2）：打开时焦点进入弹层首控件、关闭后归还触发元素。
 * 原生 autofocus 在 Vue 动态插入时不生效，改为显式驱动；Tab 圈定已移交
 * reka Dialog 的 FocusScope（Phase 6），Esc 关闭链沿用下方
 * onDocumentKeydown 的分层退出。
 */
// 触发元素栈：与弹层嵌套深度同步 push/pop。右键菜单项这类"打开弹层后自身
// 随菜单卸载"的触发元素无法承接归还焦点，逐层弹出时跳过已断连元素。
const modalTriggerStack: HTMLElement[] = [];
// 弹层外最近聚焦的稳定元素：右键菜单项属瞬态控件（点击打开弹层后随菜单
// 卸载，无法承接归还焦点），归还目标回退到菜单打开前的焦点宿主；
// 弹层内聚焦不覆盖该记录。
let lastStableFocus: HTMLElement | null = null;
// 幽灵点击守卫（R3-P1-1）：焦点归还后短窗内拦截无 mousedown 前驱的合成
// click；决策逻辑走 ghostClickGuard 纯模块（有单测），真实鼠标点击放行。
const ghostClickGuard = createGhostClickGuard();
function onDocumentMouseDownCapture(event: MouseEvent) {
  ghostClickGuard.noteMouseDown();
  // 批量目标/高亮规则两个 popover 的点空白收起已移交 reka DismissableLayer
  // （pointerdown-outside → update:open(false)），capture 手动清扫移除。
}
function onDocumentClickCapture(event: MouseEvent) {
  if (!ghostClickGuard.shouldSuppress()) return;
  event.preventDefault();
  event.stopPropagation();
}
function trackStableFocus(event: FocusEvent) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  if (target.closest('[data-slot="dialog-content"]') || target.closest('[data-slot="context-menu-content"]')) return;
  lastStableFocus = target;
}
// 与 Esc 关闭链同源的弹层在开状态（hostKey/agent 审批属安全弹窗：
// 参与聚焦与 Tab 陷阱，但不参与 Esc 关闭）。按模板出现顺序排列，
// 计数变化驱动聚焦/归还；同层互斥由交互保证。
const modalOpenStates = computed(() => [
  localOpenConfirmOpen.value,
  telnetDialogOpen.value,
  serialDialogOpen.value,
  serialUploadDialogOpen.value,
  vncDialogOpen.value,
  rdpDialogOpen.value,
  rdpCertPrompt.value !== null,
  folderPickerTarget.value !== null,
  // SFTP 工具栏工具弹窗（OTP / 导入会话），模板顺序在预览弹窗之前。
  otpPanelOpen.value,
  importPanelOpen.value,
  previewOpen.value,
  pasteConfirm.value,
  // 通用确认弹窗（SSH-H1）：模板顺序在 pasteConfirm 之后、dropUpload 之前。
  !!pendingConfirmDialog.value,

  dropUploadPrompt.value,
  downloadPrompt.value !== null,
  downloadConflictPrompt.value !== null,
  uploadDuplicatePrompt.value !== null,
  attrsTarget.value,
  deleteTarget.value,
  batchDeleteOpen.value,
  recordingDeleteTarget.value !== null,
  recordingClearAllOpen.value,
  transferHistoryClearOpen.value,
  chmodTarget.value,
  newFileDialog.value,
  operationDialog.value,
  symlinkDialog.value !== null,
  watchModifiedQueue.value.length > 0,
  // 自定义编辑器命令弹窗（编辑器映射增强），模板顺序在 watch 确认弹窗之后。
  customEditorOpen.value,
  commandOpen.value,
  profilesOpen.value,
  auditOpen.value,
  auditClearOpen.value,
  settingsOpen.value,
  alertTriageOpen.value,
  hostKeyPrompt.value,
  agentPromptHead.value,
  spawnSessionDialogOpen.value,
]);
const modalOpenCount = computed(() => modalOpenStates.value.filter(Boolean).length);

/** 当前最顶层弹层容器；无弹层时返回 null（嵌套时取首个命中即最外层——焦点回落语义）。 */
function topModalContainer(): HTMLElement | null {
  if (!modalOpenCount.value) return null;
  return document.querySelector<HTMLElement>('[data-slot="dialog-content"]');
}

function focusTopModal() {
  pickModalFocusTarget(topModalContainer())?.focus({ preventScroll: true });
}

watch(modalOpenCount, (count, previous) => {
  if (count > previous) {
    // 打开：整组从无到有时记录触发元素供关闭归还；嵌套打开（如命令
    // 对话框上叠粘贴确认）逐层入栈。触发元素优先取"弹层外稳定焦点"，
    // 避免抓到已随右键菜单卸载的菜单项。
    for (let i = previous; i < count; i++) {
      modalTriggerStack.push(lastStableFocus ?? document.body);
    }
    void nextTick(focusTopModal);
    return;
  }
  if (!count) {
    // 全部关闭：焦点归还触发按钮；触发元素已随右键菜单等卸载时逐层回退，
    // 找不到任何在档元素则落回 BODY（无焦点宿主可还）。
    while (modalTriggerStack.length) {
      const trigger = modalTriggerStack.pop()!;
      if (trigger.isConnected) {
        // 幽灵点击守卫（R3-P1-1）：键盘 Enter 提交后归还焦点的瞬间，浏览器
        // 会在刚聚焦的按钮上派发一次无 mousedown 的合成 click 并重开弹层；
        // 短窗内拦截该 click，纯键盘流一次 Enter 即成功关闭。
        ghostClickGuard.arm();
        trigger.focus({ preventScroll: true });
        break;
      }
    }
    return;
  }
  // 内层弹层关闭、外层仍在：焦点回落外层弹层首控件。
  void nextTick(focusTopModal);
});

/**
 * Esc 关闭链（一次按键关一层）：预览 > 对话框 > 右键菜单 > 工具栏弹出层。
 * 逐层 if-return：无内容打开时按键穿透，不影响终端内 vim 等自身 Esc 语义。
 */
function onDocumentKeydown(event: KeyboardEvent) {
  // 编辑器保存快捷键：仅当焦点在编辑器面板内且存在页签时接管 Cmd/Ctrl+S；
  // 终端内不抢（保留用户既有终端/远端程序语义）。
  if (
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    event.key.toLowerCase() === "s" &&
    hasEditorTabs.value &&
    editorPaneContainsFocus()
  ) {
    event.preventDefault();
    void saveActive();
    return;
  }
  // Tab 焦点陷阱已移交 reka Dialog 的 FocusScope（每个弹层独立圈定，嵌套时顶层
  // 生效）；此处不再拦截 Tab。注意 FocusScope 的 keydown 挂在内容元素上，冒泡先于
  // 本 document 处理器，若恢复自制陷阱会与 reka 双步进，勿回退。
  if (event.key === "Tab") return;
  if (event.key !== "Escape") return;
  // reka Select/ContextMenu/Popover 弹层打开时 Esc 由 reka 消费（关弹层），不落入下方
  // 弹窗关闭链。本处理器注册早于 DismissableLayer，先跑时弹层 DOM 仍在。
  if (document.querySelector('[data-slot="select-content"], [data-slot="context-menu-content"], [data-slot="popover-content"]')) return;
  // 录制倒计时优先取消（遮罩在终端区，不属于弹层体系）。
  if (isCountdownActive(recordCountdown.value)) {
    cancelRecordCountdown();
    return;
  }
  if (previewOpen.value) {
    closePreview();
    return;
  }
  if (otpPanelOpen.value) {
    otpPanelOpen.value = false;
    return;
  }
  if (importPanelOpen.value) {
    importPanelOpen.value = false;
    return;
  }
  // ---- 对话框（安全取消语义；hostKey/agent 审批等安全弹窗不在此列）----
  if (folderPickerTarget.value) {
    folderPickerTarget.value = null;
    return;
  }
  if (pasteConfirm.value) {
    resolvePasteConfirm(false);
    return;
  }
  if (pendingConfirmDialog.value) {
    resolvePendingConfirmDialog(false);
    return;
  }
  if (dropUploadPrompt.value) {
    resolveDropUpload("cancel");
    return;
  }
  if (downloadPrompt.value) {
    resolveDownloadPrompt(undefined);
    return;
  }
  if (downloadConflictPrompt.value) {
    resolveDownloadConflict(undefined);
    return;
  }
  if (uploadDuplicatePrompt.value) {
    resolveUploadDuplicate(undefined);
    return;
  }
  if (attrsTarget.value) {
    closeAttributes();
    return;
  }
  if (deleteTarget.value) {
    deleteTarget.value = undefined;
    return;
  }
  if (batchDeleteOpen.value) {
    if (!batchDeleteSubmitting.value) batchDeleteOpen.value = false;
    return;
  }
  if (recordingDeleteTarget.value) {
    if (!recordingDeleteSubmitting.value) recordingDeleteTarget.value = null;
    return;
  }
  if (recordingClearAllOpen.value) {
    if (!recordingClearAllSubmitting.value) recordingClearAllOpen.value = false;
    return;
  }
  if (transferHistoryClearOpen.value) {
    transferHistoryClearOpen.value = false;
    return;
  }
  if (chmodTarget.value) {
    chmodTarget.value = undefined;
    return;
  }
  if (newFileDialog.value) {
    newFileDialog.value = false;
    return;
  }
  if (operationDialog.value) {
    operationDialog.value = null;
    return;
  }
  if (commandOpen.value) {
    commandOpen.value = false;
    return;
  }
  if (alertTriageOpen.value) {
    alertTriageOpen.value = false;
    return;
  }
  if (profilesOpen.value) {
    profilesOpen.value = false;
    return;
  }
  if (auditOpen.value) {
    if (auditClearOpen.value) {
      if (!auditClearSubmitting.value) auditClearOpen.value = false;
      return;
    }
    auditOpen.value = false;
    return;
  }
  if (settingsOpen.value) {
    // 内联 profile 管理（设置弹窗内）沿用 profilesOpen→settingsOpen 的逐层
    // 退出语义：先关编辑表单，再收起配置档 section，最后关弹窗
    // （表单/section 状态在 SettingsDialog 内部，经组件实例询问是否已消费）。
    if (settingsDialog.value?.consumeInlineEsc()) return;
    settingsOpen.value = false;
    return;
  }
  // 右键菜单与九个工具栏 popover 均已迁移 reka（ContextMenu/Popover）：Esc 与外点
  // 由 reka 自行消费（见上方 content 守卫），不再占 Esc 链一层。本层只剩
  // 指标/录制、容器右侧栏与批量保存态（带草稿清理）。
  if (metricsOpen.value || recordingsOpen.value || batchSaveMode.value || dockerPanelOpen.value) {
    if (batchSaveMode.value) cancelBatchBarSave();
    if (metricsOpen.value || dockerPanelOpen.value) closeRightPanel();
    if (recordingsOpen.value) recordingsOpen.value = false;
  }
}

function openTransferPanel() {
  // 右键菜单项打开：菜单项自身随后卸载，互斥族统一收口后再开面板。
  closeToolbarPopovers();
  transferPanelOpen.value = true;
}





function formatModified(value?: number) {
  if (!value) return "";
  return new Intl.DateTimeFormat(locale.value, { dateStyle: "short", timeStyle: "short" }).format(new Date(value * 1000));
}


function readU64(bytes: Uint8Array, offset: number) {
  return Number(new DataView(bytes.buffer, bytes.byteOffset + offset, 8).getBigUint64(0, false));
}

function writeU64(bytes: Uint8Array, offset: number, value: number) {
  new DataView(bytes.buffer, bytes.byteOffset + offset, 8).setBigUint64(0, BigInt(value), false);
}

async function waitForHostApi(timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs;
  while (!window.dbxPlugin && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
  if (!window.dbxPlugin) throw new Error(t("hostApiUnavailable"));
  return window.dbxPlugin;
}

async function initialize() {
  const api = await waitForHostApi();
  hostContext.value = await Promise.any([
    api.ready,
    api.request<Record<string, unknown>>("host.getContext"),
  ]);
  transportReuseState = createSessionTransportReuseState(hostContext.value);
  locale.value = api.locale || "zh-CN";
  // UI 状态恢复必须等 pluginStore 水合（宿主 storage 白名单键异步灌入）：
  // 否则 workbenchUi 镜像与 setup 期的 sftp-pane-open 等键全部读到空档，
  // 布局偏好整组"不持久"（issue #178 的半个根因，另一半是宿主缺桥）。
  await pluginStore.ready.catch(() => undefined);
  restoreUiState();
  const appearanceAppliedAtBoot = Boolean(api.appearance || api.theme);
  // 外观偏好的 CSS 部分（终端内边距变量）与宿主是否推送 appearance 无关，
  // 开机先落一次，否则用户设了内边距要等下次主题推送才生效。
  applyTerminalPaddingVars();
  // FIG wave-1：legacy 动态 fs provider 随 legacy spec 目录整体退役；
  // generator 动态候选在批次 2 经 completion/execute 目标机执行接线
  // （前端不直连 shell）。
  if (api.appearance) applyAppearance(api.appearance);
  else if (isDbxPluginTheme(api.theme)) applyAppearance(themeToAppearance(api.theme));
  // 宿主可能在 init 前先应答 host.getContext（如重推连接期间 init 被延迟）：
  // 此时 api.locale/appearance 仍是 bootstrap 默认值。init 落地后重读一次，
  // 否则会话会停在默认英文/默认主题。ready 已解决时 then 立即执行，是幂等重读。
  void api.ready.then(() => {
    if (api.locale) locale.value = api.locale;
    if (!appearanceAppliedAtBoot) {
      if (api.appearance) applyAppearance(api.appearance);
      else if (isDbxPluginTheme(api.theme)) applyAppearance(themeToAppearance(api.theme));
    }
  });
  // X-P2/P3/P4 收敛：env（locale/theme）+ context + appearance 订阅统一走
  // shared/frontend/hostThemeRuntime 单点（真桥无 onLocaleChange/onContextChange
  // 幽灵 API；onContext 旧桥回退 onContextChange；appearance 契约缺失时经
  // theme 通道兜底，两套不同时挂）。
  unsubscribeEnvironment = subscribeHostEnvironment<DbxPluginAppearance, DbxPluginTheme>(api, {
    onLocale: (nextLocale) => (locale.value = nextLocale || "zh-CN"),
    onContext: (context) => {
      hostContext.value = context;
    },
    onAppearance: applyAppearance,
    onTheme: (theme) => applyAppearance(themeToAppearance(theme)),
  }, { themeChannel: onHostThemeChange });
  unsubscribeEvent = api.onEvent(handleEvent);
  unsubscribeBinary = api.onBinary(handleBinary);
  unsubscribeFileDrag = api.fileTransfer?.onDragState((active) => {
    // 新一次拖拽从“无目标”开始；结束事件不能清目标——部分宿主先发
    // active=false，再回调 onDrop，提前清掉会让上传落回当前目录。
    if (active) sftpDragDropTarget.value = "";
    dragActive.value = active;
  });
  unsubscribeFileDrop = api.fileTransfer?.onDrop((files, drop) => {
    // 宿主目录展开撞上限（如单文件夹 2000 文件）只交付前缀：明确提示而不是
    // 静默丢文件；完整上传指引工具栏「上传文件夹」选择器（浏览器枚举，无此限）。
    if (drop?.truncated) showNotice(t("hostDrop.truncated"));
    // 宿主 onDrop 不带光标下的目标行：取树行拖拽悬停路径，缺省回当前目录。
    const target = sftpDragDropTarget.value || undefined;
    sftpDragDropTarget.value = "";
    void handleHostFileDrop(files, target);
  });
  // 分屏 PTY 不随 workbenchState 恢复。webview 重载后旧分屏会话仍在 sidecar
  // 中，先按独立 workbenchId 后缀清掉，避免孤儿 PTY 泄漏（主会话不受影响）。
  void closeOrphanSplitSessions();
  // §8.3 面板加载生命周期：探活与终端创建并行。探活只是一次 sidecar 往返，
  // 串行执行会把真正耗时的 openSession 压到整个引导的最后。
  const reattachLookup = readPluginMode(hostContext.value) !== "local-terminal" && connectionId.value && workbenchId.value && !restored.value
    ? findReattachSession()
    : Promise.resolve("");
  await nextTick();
  createTerminal();
  // P0 connectionless local-terminal passthrough (HOST_PLUGIN_UI_SPEC §4/§7.1): when the host opens this workbench with
  // the workbench is opened with the command context (plugin.mode="local-terminal") (command
  // panel / toolbar entry / self-open bridge), the SSH connection flow is skipped and the local terminal opens directly.
  if (readPluginMode(hostContext.value) === "local-terminal") {
    if (!workbenchId.value) throw new Error(t("errors.hostBridgeMissing"));
    await hydratePrefs();
    // Bottom dock / webview rebuild reopen: first reattach the live shell still bound to this workbenchId in the
    // live shell (reopening never leaks a new PTY; spec §10 leaves no PTY behind on close).
    if (await reattachLocalSession()) return;
    // A4 restore semantics (spec §7.6/§8.4): restoring is not re-running the command — a restored tab
    // no automatic shell — just the exit shell until the user explicitly hits "Reopen".
    if (restored.value) {
      localSession.value = null;
      localState.value = "exited";
      localShellRestored.value = true;
      return;
    }
    // Dock "+" creates with the selected shell type (context.plugin.shell); when unset the preference applies.
    await startLocalTerminal(readPluginShell(hostContext.value) || undefined);
    return;
  }
  if (!connectionId.value || !workbenchId.value) throw new Error(t("errors.hostBridgeMissing"));
  const state = initialState();
  if (restored.value) {
    terminalState.value = "disconnected";
    terminalError.value = t("restartDisconnected");
    return;
  }
  // Webview 重建必须先按 workbenchId 接回所有非 SSH 协议会话；这些会话不
  // 写 SSH workbench state，若先走 openSession 会重新创建 Telnet/VNC/RDP/Serial。
  if (await reattachProtocolSession()) return;
  if (typeof state.sessionId === "string" && state.sessionId) await attachSession(state.sessionId);
  else {
    // 宿主切 tab / 左侧菜单重开可能整体重建工作台 webview。只恢复
    // 同一 workbench 的 live session；不能按 connectionId 复用任意会话，
    // 否则打开同一连接的新 Tab 会接管已有 Tab 的 PTY。
    const reattach = await reattachLookup;
    if (reattach) await attachSession(reattach, reattach);
    // 上一轮本地终端还活着（webview 重建但 sidecar 未退出）：接回并补发，
    // 避免孤儿 shell 挂在 sidecar 里。
    else if (await reattachLocalSession()) {
      // 本地模式接管终端。
    }
    // 上来直接连（§8.3 面板加载生命周期）：宿主已在点击创建条目时
    // ensureConnected 预拨（connectionPreconnected 旗标），这里跳过 force
    // 重开（force 会复位共享连接），直接 openSession——拨号未完成时由
    // preconnect 短间隔轮询自愈（250ms 固定节奏，不再是 2s 退避梯子），
    // 拨号失败/永久错误走既有分类报错。
    // 旧宿主无旗标：保留原有 force 重开路径。
    else {
      if (panelSurface.value && !hostContext.value.connectionPreconnected) await requestHostReopenConnection();
      await openSession(false, true);
    }
  }
}

/**
 * Asks the sidecar for the live session bound to this workbench and connection
 * (sidecar `ssh/sessions/list`); "" when none — caller dials a fresh session.
 * Failures degrade to a fresh open instead of blocking the workbench.
 */
async function findReattachSession(): Promise<string> {
  try {
    const result = await window.dbxPlugin.invoke<{ sessions?: SessionSummary[] }>("ssh/sessions/list", {}, { timeoutMs: 10_000 });
    return pickLiveSessionForReattach(result?.sessions, { connectionId: connectionId.value, workbenchId: workbenchId.value });
  } catch {
    return "";
  }
}

/** 清理 webview 重载遗留的分屏 PTY（workbenchId 带 #split 后缀）。 */
async function closeOrphanSplitSessions(): Promise<void> {
  if (!workbenchId.value) return;
  const splitWorkbenchId = `${workbenchId.value}#split`;
  try {
    const result = await window.dbxPlugin.invoke<{ sessions?: Array<{ sessionId?: string; workbenchId?: string }> }>("ssh/sessions/list", {}, { timeoutMs: 10_000 });
    const orphans = (result?.sessions ?? []).filter((entry) => entry.workbenchId === splitWorkbenchId && entry.sessionId);
    await Promise.allSettled(orphans.map((entry) => window.dbxPlugin.invoke("ssh/session/close", { sessionId: entry.sessionId })));
  } catch {
    // 旧 sidecar / 无会话列表时静默；没有分屏就没有孤儿。
  }
}

type ProtocolSessionKind = "telnet" | "vnc" | "rdp" | "serial";
interface ProtocolSessionSummary extends SessionSummary {
  host?: string;
  port?: number;
  baudRate?: number;
  binaryInput?: boolean;
}

async function reattachProtocolSession(): Promise<boolean> {
  const lists: Array<{ kind: ProtocolSessionKind; method: string }> = [
    { kind: "telnet", method: "telnet/list" },
    { kind: "vnc", method: "vnc/list" },
    { kind: "rdp", method: "rdp/list" },
    { kind: "serial", method: "serial/list" },
  ];
  const candidates = await Promise.all(lists.map(async ({ kind, method }) => {
    try {
      const result = await window.dbxPlugin.invoke<{ sessions?: ProtocolSessionSummary[] }>(method, {}, { timeoutMs: 10_000 });
      const sessionId = pickProtocolSessionForReattach(result?.sessions, workbenchId.value);
      const session = result?.sessions?.find((entry) => entry?.sessionId === sessionId);
      return session ? { kind, session } : null;
    } catch {
      return null;
    }
  }));
  const live = candidates
    .filter((candidate): candidate is { kind: ProtocolSessionKind; session: ProtocolSessionSummary } => candidate !== null)
    .sort((left, right) => (right.session.createdAt ?? 0) - (left.session.createdAt ?? 0))[0];
  if (!live) return false;

  if (live.kind === "telnet") {
    telnetSession.value = { sessionId: live.session.sessionId, host: live.session.host || "", port: live.session.port || 23 };
    telnetState.value = "running";
    telnetLastSequence.value = 0;
    telnetPendingFrames.clear();
    await window.dbxPlugin.invoke<ReplayResult>("telnet/replay", { sessionId: live.session.sessionId, afterSequence: 0 });
  } else if (live.kind === "vnc") {
    await reattachVncSession(live.session);
  } else if (live.kind === "rdp") {
    rdpSession.value = { sessionId: live.session.sessionId, host: live.session.host || "", port: live.session.port || 3389 };
    rdpState.value = { state: "running", error: "", errorKind: "", attempt: 0, maxAttempts: 0 };
    await window.dbxPlugin.invoke("rdp/replay", { sessionId: live.session.sessionId });
  } else {
    serialSession.value = {
      sessionId: live.session.sessionId,
      port: live.session.port ? String(live.session.port) : "",
      baudRate: live.session.baudRate || 115_200,
    };
    serialState.value = "running";
    serialLastSequence.value = 0;
    serialPendingFrames.clear();
    serialBinaryInput.value = live.session.binaryInput !== false;
    await window.dbxPlugin.invoke<ReplayResult>("serial/replay", { sessionId: live.session.sessionId, afterSequence: 0 });
  }
  await nextTick();
  scheduleFit();
  return true;
}

watch([splitRatio, paneOrder, paneDirection, sftpPaneOpen, followDirectory, sudoMode, visibleColumns, sftpShowHidden, sort], persistState, { deep: true });

// SFTP 自动连接关闭时的惰性加载：面板从收起到打开的瞬间才补发首次列目录
// 并初始化单栏树（连接时已打开的面板由 afterSessionConnected 触发；本会话
// 已加载/树已展开时不重复，见 shouldLazyLoadSftpListing 与 initializeSftpTree）。
watch(sftpPaneOpen, (open) => {
  if (!open) return;
  // 跟随开启时即使树已加载也要重走一遍：面板重开会让 SftpTreePane 重新挂载，
  // scrollTop 归零，需要重新揭示并把活动终端的目录滚到顶部。
  if (shouldLazyLoadSftpListing({ paneOpen: open, sessionLive: !!session.value, listingLoaded: sftpListingLoaded.value }) || !sftpTreeRoot.value.loaded || followDirectory.value) {
    void initializeSftpTree();
  }
});

// ---------------------------------------------------------------------------
// MCP 设置镜像（存储迁移批 1，IMPL_PLAN_STORAGE_SYNC）：sidecar 在 MCP serve
// 时直接消费 mcp-settings.json，pluginStore 里只存「云同步载荷」。启动时：
// store 缺失 → 从 sidecar persisted 值播种；存在但与 sidecar 不一致（云同步
// 恢复后）→ 把 store 推回 sidecar。set 是部分更新，localTransferRoot 由
// sidecar 保留，不进镜像。保存侧的双写在 SettingsDialog.saveMcpSettings。
// ---------------------------------------------------------------------------

async function syncMcpSettingsMirror() {
  const stored = loadMcpSettingsMirror();
  try {
    const remote = extractMcpSettingsMirror(await window.dbxPlugin.invoke<unknown>("mcp/settings/get", {}));
    if (stored === null) {
      persistMcpSettingsMirror(remote);
      return;
    }
    if (!mcpSettingsMirrorEquals(stored, remote)) {
      await window.dbxPlugin.invoke("mcp/settings/set", {
        maxReadBytes: stored.maxReadBytes,
        maxUploadBytes: stored.maxUploadBytes,
        maxDownloadBytes: stored.maxDownloadBytes,
        execPermissionMode: stored.execPermissionMode,
        connectionScope: stored.connectionScope,
      });
    }
  } catch {
    // sidecar 不可用（旧版/降级）：跳过，下次启动重试。
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClickCloseMenus);
  // AI 桥能力位（Warp AI 对齐批）：桥 bootstrap 与 UI 挂载同批完成，挂载后
  // 探一次供设置页横幅展示；运行时调用点各做新鲜探测（aiBridgeOk）。
  aiBridgeReady.value = aiBridgeOk();
  // 文件夹上传能力探测（issue #78）：webkitdirectory 非标准属性，缺失环境
  // （老 webview）隐藏入口，文件级上传不受影响。
  const folderProbe = document.createElement("input");
  folderProbe.type = "file";
  folderUploadSupported.value = "webkitdirectory" in folderProbe;
  document.addEventListener("click", onDocumentClickCapture, true);
  document.addEventListener("mousedown", onDocumentMouseDownCapture, true);
  document.addEventListener("keydown", onDocumentKeydown);
  document.addEventListener("focusin", trackStableFocus);
  document.addEventListener("mouseover", onTooltipOver);
  document.addEventListener("mouseout", onTooltipOut);
  document.addEventListener("focusin", onTooltipFocusIn);
  document.addEventListener("focusout", onTooltipFocusOut);
  document.addEventListener("pointerdown", hideTooltip, true);
  document.addEventListener("wheel", hideTooltip, true);
  window.addEventListener("dbx:docker-open-in-terminal", handleDockerOpenInTerminal);
  window.addEventListener("dragleave", resetDragHighlightOnWindowLeave);
  hostFontObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
  void hydrateQuickCommands();
  void hydrateHighlightRules();
  void syncMcpSettingsMirror();
  void hydratePrefs();
  void initialize().catch((cause) => {
    terminalState.value = "error";
    showError(cause, "terminal");
  });
});

onBeforeUnmount(() => {
  disposed = true;
  window.removeEventListener("dbx:docker-open-in-terminal", handleDockerOpenInTerminal);
  window.removeEventListener("dragleave", resetDragHighlightOnWindowLeave);
  document.removeEventListener("mouseover", onTooltipOver);
  document.removeEventListener("mouseout", onTooltipOut);
  document.removeEventListener("focusin", onTooltipFocusIn);
  document.removeEventListener("focusout", onTooltipFocusOut);
  document.removeEventListener("pointerdown", hideTooltip, true);
  document.removeEventListener("wheel", hideTooltip, true);
  hostFontObserver.disconnect();
  hideTooltip();
  persistThrottle.cancel(); // 卸载兜底：丢弃挂起定时器后直接落盘（保留既有语义）
  void writeWorkbenchState();
  window.clearTimeout(resizeTimer);
  window.clearTimeout(reconnectTimer.value);
  window.clearInterval(reconnectCountdownTimer.value);
  window.clearInterval(terminalDiagTimer);
  disposePromptHints();
  stopCommandMarkerTick();
  stopAgentPromptTimer();
  resolvePasteConfirm(false);
  resolvePendingConfirmDialog(false);
  if (terminalHost.value) detachTerminalHost(terminalHost.value);
  document.removeEventListener("click", onDocumentClickCloseMenus);
  document.removeEventListener("click", onDocumentClickCapture, true);
  document.removeEventListener("mousedown", onDocumentMouseDownCapture, true);
  document.removeEventListener("keydown", onDocumentKeydown);
  document.removeEventListener("focusin", trackStableFocus);
  unsubscribeEvent?.();
  unsubscribeBinary?.();
  unsubscribeEnvironment?.();
  resizeObserver?.disconnect();
  osc52Disposable?.dispose();
  osc52Disposable = undefined;
  for (const disposable of oscFeedDisposables) disposable.dispose();
  oscFeedDisposables = [];
  disposeInput?.dispose();
  disposeWebkitInputFallback?.();
  disposeWebkitInputFallback = undefined;
  disposeSelectionCopy?.dispose();
  disposeTerminalBell?.dispose();
  disposeTerminalBell = undefined;
  terminalWriteThrottle.dispose();
  // 终端重建/会话关闭：在途写入回调整体作废，保护态复位，避免旧积压误判。
  outputInFlightBytes = 0;
  outputGate.reset();
  detachHighlightRender();
  detachActionLinks();
  detachGutterListeners();
  terminal?.dispose();
  for (const waiter of uploadAckWaiters.values()) {
    window.clearTimeout(waiter.timer);
    waiter.reject(new Error(t("errors.workbenchDetached")));
  }
  for (const waiter of downloadChunkWaiters.values()) {
    window.clearTimeout(waiter.timer);
    waiter.reject(new Error(t("errors.workbenchDetached")));
  }
});

// 空提示符快捷键引导条（TerminalPromptHints）：状态/显隐门/消散收口在
// composables/usePromptHints；声明位置收在 setup 尾部（依赖链上有后段才
// 声明的 const，放前面必踩 TDZ）。
// 提升的薄包装：早期配置对象（aiSearch onActivate / 补全配置）直接引用
// dismissPromptHints 函数名，实体在下方 composable 调用后接线，保持既有
// 引用时机（调用都发生在 setup 结束之后）。
let dismissPromptHintsImpl: (() => void) | undefined;
function dismissPromptHints() {
  dismissPromptHintsImpl?.();
}
const {
  promptHintsEnabled,
  promptHintsLineEmpty,
  promptHintsAnchor,
  markTerminalOutputActive,
  promptHintsHistoryKey,
  promptHintsCompletionsKey,
  promptHintsVisible,
  setPromptHintsEnabled,
  dismissPromptHints: dismissPromptHintsNow,
  interactivePromptGatePending,
  resetPromptHintsQuiet,
  disposePromptHints,
} = usePromptHints({
  terminal: () => terminal,
  isLocalMode, session, commandRunning, terminalTransferBusy,
  suggestionOpen, completionOpen, historyPanelOpen, quickSelectOpen, searchOpen,
  aiSearchActive: () => aiSearchState.value.active,
  hostKeyPrompt, terminalHotkeys, applePlatform,
  readTerminalSuggestionAnchor,
});
dismissPromptHintsImpl = dismissPromptHintsNow;

// 命令历史作用域切换（连接切换 / SSH↔本地 / 本地↔串口）：命令弹窗的历史
// 浏览态随环换桶作废，避免 ↑↓ 指向另一作用域的下标/草稿。放 setup 尾部：
// watch 创建即求值 historyScope，引用的 connectionId/serialSession 在前段。
watch(historyScope, () => {
  commandHistoryIndex.value = -1;
  commandHistoryBackup.value = "";
});
</script>

<template>
  <main class="workbench" :class="{ 'panel-surface': panelSurface }">
    <!-- dock 工具条隐藏后：右上角低透明度微钮还原（偏好落 pluginStore）。 -->
    <button v-if="panelSurface && panelToolbarHidden" type="button" class="panel-toolbar-reveal" :title="t('panel.showToolbar')" :aria-label="t('panel.showToolbar')" @click="togglePanelToolbar"><ChevronDown /></button>
    <header v-if="!panelSurface || !panelToolbarHidden" class="toolbar" :style="toolbarStyle">
      <!-- 连接信息入口：Info 图标按钮紧跟标识区（状态徽章右侧），弹层左对齐锚定。
           Dock 面板表面整体换用下方 panel-actions 精简组（宿主 chrome 已有标题/关闭）。 -->
      <div v-if="!panelSurface" class="identity-side">
        <div class="identity">
          <span v-if="connection.color" class="connection-color" :style="{ backgroundColor: connection.color }" />
          <!-- 主标题跟随 ConnectingCard 的口径：连接名优先，未命名回退 user@host:port；
               hover 仍可看到技术标识（UUID 只在名称与 host 全缺失时兜底出现）。 -->
          <strong :title="connectionIdentity">{{ connection.name || connectionIdentity }}</strong>
          <span v-if="connection.readOnly || connectionReadOnly" class="read-only-badge">{{ t("readOnly") }}</span>
          <span class="session-pill" :class="`session-${sessionStatus}`"><span class="session-dot" aria-hidden="true" />{{ sessionPillText }}<span v-if="sessionStatus === 'reconnecting' && reconnectCountdown" class="session-pill-countdown mono">{{ t("sessionStatus.reconnectCountdown", { seconds: reconnectCountdown.seconds, attempt: reconnectCountdown.attempt }) }}</span></span>
        </div>
        <ConnectionInfoPopover
          :connection-info-open="connectionInfoOpen"
          :connection="connection"
          :metrics-distro-badge="metricsDistroBadge"
          :connection-auth-method-label="connectionAuthMethodLabel"
          :connection-read-only="connectionReadOnly"
          :connection-latency="connectionLatency"
          :connection-latency-busy="connectionLatencyBusy"
          :connection-latency-failed="connectionLatencyFailed"
          :connected="connected"
          :t="t"
          @update:connectionInfoOpen="(value) => { if (!value) connectionInfoOpen = false; }"
          @toggle="toggleConnectionInfo"
          @measure="measureLatency"
        />
      </div>
      <div v-if="!panelSurface" class="toolbar-actions">
        <!-- —— SSH 工具组·视图段（分栏/字号）：连接协议门控（!isRdpConnection）
             在组级判一次，组内按钮只保留各自其余条件（localUiMode/panelSurface）；
             RDP 连接下整组不渲染，不产生孤儿按钮。 -->
        <template v-if="!isRdpConnection">
          <button class="icon-button icon-neutral" :title="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" :aria-label="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" :disabled="panelSurface" @click="togglePaneOrder"><ArrowLeftRight /></button>
          <!-- Local terminal UI hides SSH-only actions outright (not disabled): the local
               shell has no SSH session to act on. -->
          <button v-if="!localUiMode" class="icon-button icon-cyan" :class="{ 'is-active': sftpPaneOpen && sftpViewActive }" :title="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-label="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-pressed="sftpPaneOpen && sftpViewActive" :disabled="panelSurface" @click="toggleSftpPane"><FolderOpen v-if="!(sftpPaneOpen && sftpViewActive)" /><PanelRightClose v-else /></button>
          <button class="icon-button" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
          <button class="icon-button" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
        </template>
        <!-- —— RDP 独立工具组（isRdpMode 会话激活期）：缩放三档 + 桌面分辨率 +
             大文本粘贴 + 断开。与 SSH 组互斥、各判一次门控；SSH 工具整类隐藏后，
             这是远程桌面自己需要的操作面。 -->
        <template v-if="isRdpMode">
          <div class="rdp-scale-group" role="group" :aria-label="t('rdp.scaleMode')">
            <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'fit' }" :title="t('rdp.scaleMode.fit')" :aria-label="t('rdp.scaleMode.fit')" :aria-pressed="rdpScaleMode === 'fit'" @click="rdpScaleMode = 'fit'"><Scan /></button>
            <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'actual' }" :title="t('rdp.scaleMode.actual')" :aria-label="t('rdp.scaleMode.actual')" :aria-pressed="rdpScaleMode === 'actual'" @click="rdpScaleMode = 'actual'"><span class="font-step-label" aria-hidden="true">1:1</span></button>
            <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'stretch' }" :title="t('rdp.scaleMode.stretch')" :aria-label="t('rdp.scaleMode.stretch')" :aria-pressed="rdpScaleMode === 'stretch'" @click="rdpScaleMode = 'stretch'"><StretchHorizontal /></button>
          </div>
          <div>
            <Popover :open="rdpResolutionOpen" @update:open="(open) => { if (!open) rdpResolutionOpen = false; }">
              <PopoverAnchor as-child>
                <button class="icon-button icon-cyan" :class="{ 'is-active': rdpResolutionOpen }" :title="t('rdp.resolution.title')" :aria-label="t('rdp.resolution.title')" :aria-expanded="rdpResolutionOpen" @pointerdown.stop @click.stop="toggleRdpResolutionMenu"><Proportions /></button>
              </PopoverAnchor>
              <PopoverContent class="popover rdp-resolution-popover" align="start" :side-offset="5">
                <h3>{{ t("rdp.resolution.title") }}</h3>
                <div class="rdp-resolution-presets">
                  <button v-for="preset in RDP_RESOLUTION_PRESETS" :key="`${preset.width}x${preset.height}`" class="rdp-resolution-item" :class="{ 'is-active': isRdpResolutionActive(preset.width, preset.height) }" :disabled="rdpResizing" @click="applyRdpResolution(preset.width, preset.height)">
                    {{ rdpResolutionLabel(preset.width, preset.height) }}
                  </button>
                  <button class="rdp-resolution-item" :disabled="rdpResizing" @click="applyRdpResolution(rdpWindowProportionPreset().width, rdpWindowProportionPreset().height)">{{ t("rdp.resolution.window") }}</button>
                </div>
                <div class="rdp-resolution-custom">
                  <input v-model="rdpCustomWidth" class="mono" type="number" min="640" max="3840" :placeholder="t('rdp.width')" :aria-label="t('rdp.width')" />
                  <span aria-hidden="true">×</span>
                  <input v-model="rdpCustomHeight" class="mono" type="number" min="480" max="2160" :placeholder="t('rdp.height')" :aria-label="t('rdp.height')" />
                  <button class="primary-button" :disabled="rdpResizing || !rdpCustomWidth || !rdpCustomHeight" @click="applyRdpCustomResolution">{{ t("rdp.resolution.apply") }}</button>
                </div>
                <p v-if="rdpDesktopSize" class="muted rdp-paste-hint">{{ t("rdp.resolution.current", { width: rdpDesktopSize.width, height: rdpDesktopSize.height }) }}</p>
                <p class="muted rdp-paste-hint">{{ t("rdp.resolution.note") }}</p>
              </PopoverContent>
            </Popover>
          </div>
          <div>
            <Popover :open="rdpPasteOpen" @update:open="(open) => { if (!open) rdpPasteOpen = false; }">
              <PopoverAnchor as-child>
                <button class="icon-button icon-cyan" :class="{ 'is-active': rdpPasteOpen }" :title="t('rdp.paste.title')" :aria-label="t('rdp.paste.title')" :aria-expanded="rdpPasteOpen" @pointerdown.stop @click.stop="toggleRdpPasteMenu"><ClipboardPaste /></button>
              </PopoverAnchor>
              <PopoverContent class="popover rdp-paste-popover" align="start" :side-offset="5">
                <h3>{{ t("rdp.paste.title") }}</h3>
                <textarea v-model="rdpPasteText" class="rdp-paste-text mono" :placeholder="t('rdp.paste.placeholder')" rows="8" spellcheck="false" />
                <div class="rdp-paste-actions">
                  <button class="primary-button" :disabled="!rdpPasteText" @click="rdpWriteRemoteClipboard">{{ t("rdp.paste.toClipboard") }}</button>
                  <button class="link-button" :disabled="!rdpPasteText || rdpTypingBusy" @click="rdpTypeAsKeystrokes">{{ t("rdp.paste.type") }}</button>
                </div>
                <p class="muted rdp-paste-hint">{{ t("rdp.paste.toClipboardHint") }}</p>
                <p v-if="rdpTypingTotal > 0" class="muted rdp-paste-hint">{{ rdpTypingBusy ? t("rdp.paste.typing", { sent: rdpTypingSent, total: rdpTypingTotal }) : t("rdp.paste.typeDone", { count: rdpTypingSent }) }}</p>
              </PopoverContent>
            </Popover>
          </div>
          <button class="icon-button icon-emerald" :title="t('rdp.disconnect')" :aria-label="t('rdp.disconnect')" @click="closeRdpSession"><PlugZap /></button>
        </template>
        <!-- 工具条语义分组分隔线：两侧（视图段/会话菜单）在 SSH 与 RDP 两种形态
             下都各有一侧可见，永远不会成为孤儿分隔符；其余分隔线全部收进 SSH 组。 -->
        <span class="toolbar-separator" aria-hidden="true" />
        <!-- 会话下拉：新建 / 复制 / 命令会话 / 本地终端 / Shell 选项收敛为单入口，
             工具条不再平铺五个会话类按钮（视觉走查）。Shell 选项为同弹层视图切换
             （数据与 local/shells/list 懒加载逻辑不变）。 -->
        <div>
          <Popover :open="sessionMenuOpen" @update:open="(open) => { if (!open) sessionMenuOpen = false; }">
            <PopoverAnchor as-child>
              <button class="icon-button icon-emerald" :class="{ 'is-active': sessionMenuOpen }" :title="t('sessionMenu.title')" :aria-label="t('sessionMenu.title')" :aria-expanded="sessionMenuOpen" @pointerdown.stop @click.stop="toggleSessionMenu"><ChevronDown /></button>
            </PopoverAnchor>
            <PopoverContent class="popover session-menu-popover" :class="{ 'session-menu-shell': sessionMenuShellOpen || sessionMenuShellPickOpen }" align="start" :side-offset="5">
              <template v-if="!sessionMenuShellOpen && !sessionMenuShellPickOpen">
                <h3>{{ t("sessionMenu.title") }}</h3>
                <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId" @click="sessionMenuOpen = false; openNewSessionTab();"><SquarePlus />{{ t("newSessionTab") }}</button>
                <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId || !connected" @click="sessionMenuOpen = false; openCopiedSessionTab();"><Copy />{{ t("copySessionTab") }}</button>
                <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId || !connected" @click="sessionMenuOpen = false; openCommandSessionTab();"><TerminalIcon />{{ t("spawnSessionTab") }}</button>
                <template v-if="!localUiMode"><hr class="session-menu-separator" /></template>
                <!-- 本地终端：sidecar 所在机器的登录 shell。与 SSH 会话互斥展示，
                     已连接时经确认先关 SSH；退出态由终端覆盖层提供重开出口。
                     Telnet/VNC/RDP/串口模式下为对应的断开动作。 -->
                <button class="session-menu-item" @click="sessionMenuOpen = false; toggleLocalTerminal();"><TerminalIcon />{{ isRdpMode ? t("rdp.disconnect") : isVncMode ? t("vnc.disconnect") : isSerialMode ? t("serial.disconnect") : isTelnetMode ? t("telnet.disconnect") : localUiMode && !localShellRestored ? t("localTerminal.close") : t("localTerminal.open") }}</button>
                <button v-if="!isTelnetMode && !isSerialMode && !isVncMode && !isRdpMode" class="session-menu-item" @click="openSessionShellPick"><ChevronDown />{{ t("localTerminal.pickShell") }}</button>
                <button v-if="!isTelnetMode && !isSerialMode && !isVncMode && !isRdpMode" class="session-menu-item" @click="openSessionShellPrefs"><ChevronDown />{{ t("localTerminal.settings") }}</button>
              </template>
              <template v-else-if="sessionMenuShellPickOpen">
                <header class="session-shell-head">
                  <button class="icon-button compact" :title="t('cancel')" :aria-label="t('cancel')" @click="sessionMenuShellPickOpen = false"><ArrowLeft /></button>
                  <h3>{{ t("localTerminal.pickShell") }}</h3>
                </header>
                <p class="muted local-shell-hint">{{ t("localTerminal.pickShellHint") }}</p>
                <div v-if="localShellsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
                <template v-else-if="localShells.length">
                  <!-- dock「+」同款列表语义：点击即以该 shell 一次性打开/重启，
                       不改 localShell 默认偏好；首项 = 偏好/自动探测链。 -->
                  <button class="shell-surface-item" @click="pickLocalShell()">
                    <TerminalIcon class="h-3.5 w-3.5" />{{ t("localTerminal.autoShell") }}
                  </button>
                  <button v-for="entry in localShells" :key="entry.program" class="shell-surface-item" @click="pickLocalShell(entry.program)">
                    <TerminalIcon class="h-3.5 w-3.5" />{{ entry.name }}<span class="mono local-shell-program">{{ entry.program }}</span>
                    <span v-if="entry.isDefault" class="local-shell-badge">{{ t("localTerminal.defaultBadge") }}</span>
                    <span v-if="entry.isUserShell" class="local-shell-badge">{{ t("localTerminal.userShellBadge") }}</span>
                  </button>
                </template>
                <p v-else class="muted local-shell-hint">{{ t("localTerminal.shellsUnavailable") }}</p>
                <footer class="local-shell-footer">
                  <button class="primary-button" :disabled="localState === 'starting'" @click="sessionMenuOpen = false; sessionMenuShellPickOpen = false; pickLocalShell()">
                    {{ localUiMode ? t("localTerminal.restart") : t("localTerminal.open") }}
                  </button>
                </footer>
              </template>
              <template v-else>
                <header class="session-shell-head">
                  <button class="icon-button compact" :title="t('cancel')" :aria-label="t('cancel')" @click="sessionMenuShellOpen = false"><ArrowLeft /></button>
                  <h3>{{ t("localTerminal.settings") }}</h3>
                </header>
                <p class="muted local-shell-hint">{{ t("localTerminal.settingsHint") }}</p>
                <div v-if="localShellsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
                <template v-else-if="localShells.length">
                  <label v-for="entry in localShells" :key="entry.program" class="agent-mode-option">
                    <input type="radio" name="local-shell" :checked="localShellPref ? localShellPref === entry.program : entry.isDefault" @change="setLocalShellPref(entry.program)" />
                    <span class="local-shell-row">
                      <strong>{{ entry.name }}</strong>
                      <span class="mono local-shell-program">{{ entry.program }}</span>
                      <span v-if="entry.isDefault" class="local-shell-badge">{{ t("localTerminal.defaultBadge") }}</span>
                      <span v-if="entry.isUserShell" class="local-shell-badge">{{ t("localTerminal.userShellBadge") }}</span>
                    </span>
                  </label>
                </template>
                <p v-else class="muted local-shell-hint">{{ t("localTerminal.shellsUnavailable") }}</p>
                <label class="agent-mode-option" :title="selectedShellInjectable === false ? t('localTerminal.injectionUnavailable') : ''">
                  <input type="checkbox" name="local-shell-integration" :checked="localShellIntegrationPref" :disabled="selectedShellInjectable === false" @change="setLocalShellIntegrationPref(($event.target as HTMLInputElement).checked)" />
                  <span>{{ t("localTerminal.injection") }}</span>
                </label>
                <!-- 启动目录 + 环境变量（localShellCwd/localShellEnv 偏好）：
                     change 时落库，草稿行（未填名）只留在 UI 不持久化。 -->
                <div class="local-shell-field">
                  <label class="local-shell-field-label" for="local-shell-cwd">{{ t("localTerminal.cwdLabel") }}</label>
                  <input id="local-shell-cwd" :value="localShellCwdPref" class="mono local-shell-input" :placeholder="t('localTerminal.cwdPlaceholder')" spellcheck="false" @change="setLocalShellCwd(($event.target as HTMLInputElement).value)" />
                  <p class="muted local-shell-hint">{{ t("localTerminal.cwdHint") }}</p>
                </div>
                <div class="local-shell-field">
                  <div class="local-shell-env-head">
                    <span class="local-shell-field-label">{{ t("localTerminal.envLabel") }}</span>
                    <button class="icon-button compact" :title="t('localTerminal.envAdd')" :aria-label="t('localTerminal.envAdd')" @click="addLocalShellEnvRow"><Plus /></button>
                  </div>
                  <div v-for="(_, index) in localShellEnvPref" :key="index" class="local-shell-env-row">
                    <input v-model="localShellEnvPref[index].name" class="mono local-shell-input" :placeholder="t('localTerminal.envName')" :aria-label="t('localTerminal.envName')" spellcheck="false" @change="setLocalShellEnv()" />
                    <input v-model="localShellEnvPref[index].value" class="mono local-shell-input" :placeholder="t('localTerminal.envValue')" :aria-label="t('localTerminal.envValue')" spellcheck="false" @change="setLocalShellEnv()" />
                    <button class="icon-button compact" :title="t('localTerminal.envRemove')" :aria-label="t('localTerminal.envRemove')" @click="removeLocalShellEnvRow(index)"><Trash2 /></button>
                  </div>
                  <p class="muted local-shell-hint">{{ t("localTerminal.envHint") }}</p>
                </div>
                <footer class="local-shell-footer">
                  <!-- 本地模式中按钮保持可用：restart 语义（关当前 → 按新偏好重开）。
                       仅 starting 期间禁用防双击。 -->
                  <button
                    v-if="canOpenLocalTab"
                    class="local-tab-button"
                    :title="t('localTerminal.openInNewTab')" :aria-label="t('localTerminal.openInNewTab')"
                    @click="openLocalTerminalTab"
                  ><SquarePlus /></button>
                  <Popover :open="localShellSurfaceOpen" @update:open="(open) => (localShellSurfaceOpen = open)">
                    <PopoverAnchor as-child>
                      <button
                        v-if="canOpenLocalTab"
                        class="local-tab-button"
                        :title="t('localTerminal.openShellSurface')" :aria-label="t('localTerminal.openShellSurface')"
                        @pointerdown.stop @click.stop="openLocalShellSurfaceMenu"
                      ><ListPlus /></button>
                    </PopoverAnchor>
                    <PopoverContent class="popover" align="end" :side-offset="5">
                      <button class="shell-surface-item" @click="openLocalShellSurface()">
                        <TerminalIcon class="h-3.5 w-3.5" />{{ t("localTerminal.autoShell") }}
                      </button>
                      <!-- 已配置默认 Shell 时隐藏发现列表（与 dock「+」launch-options
                           同规则）：autoShell 项已按偏好打开，无需再选。 -->
                      <template v-if="!localShellPref">
                        <button v-for="entry in localShells" :key="entry.program" class="shell-surface-item" @click="openLocalShellSurface(entry.program)">
                          <TerminalIcon class="h-3.5 w-3.5" />{{ entry.name }}<span class="mono local-shell-program">{{ entry.program }}</span>
                        </button>
                      </template>
                      <template v-if="dockConnections.length">
                        <p class="shell-surface-header">{{ t("localTerminal.connectionTerminals") }}</p>
                        <button v-for="connection in dockConnections" :key="`conn-${connection.id}`" class="shell-surface-item" @click="openConnectionSurface(connection)">
                          <TerminalIcon class="h-3.5 w-3.5" />{{ connection.name }}
                        </button>
                      </template>
                    </PopoverContent>
                  </Popover>
                  <button class="primary-button" :disabled="localState === 'starting'" @click="sessionMenuOpen = false; sessionMenuShellOpen = false; localShellRestored || isLocalMode ? restartLocalTerminal() : requestLocalTerminal()">
                    {{ localUiMode ? t("localTerminal.restart") : t("localTerminal.open") }}
                  </button>
                </footer>
              </template>
            </PopoverContent>
          </Popover>
        </div>
        <!-- —— SSH 工具组·运维/命令/记录/SFTP 工具段：组级 !isRdpConnection 判
             一次，段内三条语义分隔线随组渲染，RDP 连接下不会再出现孤儿分隔符。
             组内按钮只保留 localUiMode/isSerialMode/panelSurface 等自身条件。 -->
        <template v-if="!isRdpConnection">
        <!-- 串口文件上传入口（NyaTerm 对齐 P0-3）：仅串口模式可用；传输中禁发。 -->
        <button v-if="isSerialMode" class="icon-button icon-emerald" :title="t('serial.upload.open')" :aria-label="t('serial.upload.open')" :disabled="serialUploadBusy" @click="serialUploadDialogOpen = true"><FileUp /></button>
        <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('reconnect')" :aria-label="t('reconnect')" :disabled="terminalState === 'connecting' && !reconnectPending" @click="reconnectNow"><PlugZap /></button>
        <!-- 一键 sudo -v：向当前 PTY 写入命令刷新 sudo 凭据缓存；quick sudo 自动应答
             是否启用由连接设置决定（设置弹窗），工作台不再提供开关。 -->
        <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('sudoRefresh.title')" :aria-label="t('sudoRefresh.title')" :disabled="!connected" @click="sendSudoRefresh"><ShieldCheck /></button>
        <!-- quick sudo profiles 管理入口（M32-A）已归位设置·sudo：工具条不再放
             管理类按钮，sudoRefresh 保留为会话内即时动作。 -->
        <button v-if="!localUiMode" class="icon-button icon-cyan" :title="t('alertTriage.title')" :aria-label="t('alertTriage.title')" @click="openAlertTriage"><Siren /></button>
        <!-- main 新增的端口转发入口同属 SSH 专属：沿用 A4 惯例在本地模式整体隐藏。 -->
        <button v-if="!localUiMode" class="icon-button icon-cyan" :title="t('forwards.title')" :aria-label="t('forwards.title')" :disabled="!connectionId" @click="forwardsOpen = true"><Network /></button>
        <!-- 容器管理右侧栏视图（鲸鱼 logo 独立入口）：与 SFTP/指标互斥切换；
             SFTP 面板关闭时也可达；local 模式保留——本机 daemon（Docker
             Desktop/OrbStack）场景照常可用。面板自治，切走即卸载停轮询。 -->
        <button class="icon-button icon-blue" :class="{ 'is-active': dockerPanelOpen }" :title="t('docker.toolbarTitle')" :aria-label="t('docker.toolbarTitle')" :aria-pressed="dockerPanelOpen" @click.stop="toggleDockerPanel"><DockerWhaleLogo /></button>
        <span class="toolbar-separator" aria-hidden="true" />
        <button v-if="!localUiMode" class="icon-button icon-neutral" :title="t('commandTitle')" :aria-label="t('commandTitle')" :disabled="!connected" @click="openCommandDialog"><SquareTerminal /></button>
        <button v-if="!localUiMode && !panelSurface" class="icon-button icon-neutral" :class="{ 'is-active': batchBarOpen }" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" :aria-pressed="batchBarOpen" :disabled="!connected" @click="toggleBatchBar"><ListChecks /></button>
        <div>
          <Popover :open="quickMenuOpen" @update:open="(open) => { if (!open) quickMenuOpen = false; }">
            <PopoverAnchor as-child>
              <button class="icon-button icon-amber" :title="t('quickCommands')" :aria-label="t('quickCommands')" :disabled="!connected" @pointerdown.stop @click.stop="toggleQuickMenu"><Zap /></button>
            </PopoverAnchor>
            <PopoverContent class="popover quick-commands-popover" align="end" :side-offset="5">
            <!-- M32-A3 后工具条只留"列表 + 搜索 + 执行"（Termius Snippets 式）：
                 新建/编辑/导入管理归位设置·终端（QuickCommandsSection）。 -->
            <h3>{{ t("quickCommands") }}</h3>
            <p class="quick-command-global-hint">{{ t("quickCommandsGlobalHint") }}</p>
            <div v-if="quickCommands.length" class="quick-search">
              <Search />
              <input v-model="quickSearch" :placeholder="t('quickCommandsSearch')" spellcheck="false" />
            </div>
            <div v-if="!quickCommands.length" class="empty compact">{{ t("quickCommandsEmpty") }}</div>
            <div v-else-if="!filteredQuickCommands.length" class="empty compact">{{ t("quickCommandsNoMatch") }}</div>
            <template v-else-if="quickHasRealGroups">
              <section v-for="group in groupedQuickCommands" :key="group.id || '__ungrouped__'" class="quick-group">
                <button class="quick-group-head" :aria-expanded="!quickCollapsedGroups.includes(group.id)" @click="toggleQuickGroupCollapse(group.id)">
                  <ChevronDown v-if="!quickCollapsedGroups.includes(group.id)" class="quick-group-chevron" />
                  <ChevronRight v-else class="quick-group-chevron" />
                  <span class="quick-group-name">{{ quickGroupName(group) }}</span>
                  <span class="quick-group-count">{{ group.commands.length }}</span>
                </button>
                <template v-if="!quickCollapsedGroups.includes(group.id)">
                  <div v-for="item in group.commands" :key="item.id" class="quick-command-row quick-card" :class="{ expanded: quickExpandedId === item.id }">
                    <button class="quick-card-main" :title="item.command" :aria-label="item.command" @click="toggleQuickExpand(item.id)">
                      <Braces class="quick-card-icon" />
                      <span class="quick-card-text">
                        <strong>{{ item.name }}</strong>
                        <span class="mono">{{ item.command }}</span>
                      </span>
                    </button>
                    <div class="quick-card-actions">
                      <button class="quick-action" :disabled="!connected" @click="sendQuickCommand(item)">{{ t("quickCommandRun") }}</button>
                      <button class="quick-action" :disabled="!connected" @click="pasteQuickCommand(item)">{{ t("quickCommandPaste") }}</button>
                    </div>
                    <div v-if="quickExpandedId === item.id" class="quick-card-full mono">{{ item.command }}</div>
                  </div>
                </template>
              </section>
            </template>
            <div v-for="item in filteredQuickCommands" v-else :key="item.id" class="quick-command-row quick-card" :class="{ expanded: quickExpandedId === item.id }">
              <button class="quick-card-main" :title="item.command" :aria-label="item.command" @click="toggleQuickExpand(item.id)">
                <Braces class="quick-card-icon" />
                <span class="quick-card-text">
                  <strong>{{ item.name }}</strong>
                  <span class="mono">{{ item.command }}</span>
                </span>
              </button>
              <div class="quick-card-actions">
                <button class="quick-action" :disabled="!connected" @click="sendQuickCommand(item)">{{ t("quickCommandRun") }}</button>
                <button class="quick-action" :disabled="!connected" @click="pasteQuickCommand(item)">{{ t("quickCommandPaste") }}</button>
              </div>
              <div v-if="quickExpandedId === item.id" class="quick-card-full mono">{{ item.command }}</div>
            </div>
            <footer class="quick-command-footer quick-command-footer--readonly">
              <span class="quick-command-limit">{{ quickCommandGroupSummary }}</span>
              <span class="quick-command-manage-hint">{{ t("quickCommands.manageHint") }}</span>
            </footer>
            </PopoverContent>
          </Popover>
        </div>
        <div>
          <Popover :open="agentModeOpen" @update:open="(open) => { if (!open) agentModeOpen = false; }">
            <PopoverAnchor as-child>
              <button class="icon-button" :class="agentMode === 'off' ? 'icon-neutral' : 'icon-emerald is-active'" :title="t('agentTerminalQuickHint')" :aria-label="t('agentTerminalQuickHint')" :aria-pressed="agentMode !== 'off'" :disabled="!connected" @pointerdown.stop @click.stop="toggleAgentModeMenu"><Bot /></button>
            </PopoverAnchor>
            <PopoverContent class="popover agent-mode-popover" align="end" :side-offset="5">
            <h3>{{ t("agentTerminalSection") }}</h3>
            <label v-for="mode in AGENT_MODES" :key="mode" class="agent-mode-option">
              <input type="radio" name="agent-mode" :checked="agentMode === mode" :disabled="agentModeBusy" @change="applyAgentMode(mode)" />
              <span>{{ t(`agentTerminal${mode === "off" ? "Off" : mode === "auto" ? "Auto" : "Strict"}`) }}</span>
            </label>
            <p class="muted agent-mode-note">{{ agentModeHint }}</p>
            </PopoverContent>
          </Popover>
        </div>
        <!-- 高亮规则管理（M32-A2）已归位设置·终端（HighlightRulesSection）：
             工具条不再放配置编辑器，渲染扫描（compiledHighlightRules）仍在。 -->
        <span class="toolbar-separator" aria-hidden="true" />
        <button class="icon-button icon-emerald" :class="{ 'is-active': sftpPaneOpen && rightPanelView === 'metrics' }" :title="t('metrics')" :aria-label="t('metrics')" :aria-pressed="sftpPaneOpen && rightPanelView === 'metrics'" :disabled="!connected" @click="toggleMetricsPanel"><Gauge /></button>
        <button class="icon-button" :class="{ 'is-recording': recordingActive }" :title="recordingActive ? t('recordingStop') : t('recordingTitle')" :aria-label="recordingActive ? t('recordingStop') : t('recordingTitle')" :disabled="!recordingTarget" @click="toggleRecording"><Disc /></button>
        <button class="icon-button" :class="{ 'is-active': recordingsOpen }" :title="t('recordingsTitle')" :aria-label="t('recordingsTitle')" :aria-pressed="recordingsOpen" @click="toggleRecordings"><Film /></button>
        <!-- 设置弹窗承载 SSH/SFTP/终端专属分节（sudo/agent/安全/命名编码…），
             RDP 连接下整类不适用——入口随之隐藏（RDP 实验开关在非 RDP tab 的
             设置·终端里仍可达）；审计日志只记录 SSH/SFTP 操作，RDP 从不入审计。 -->
        <button class="icon-button icon-violet" :title="t('settings')" :aria-label="t('settings')" @click="openSettings"><Settings /></button>
        <button class="icon-button icon-amber" :title="t('auditLog.title')" :aria-label="t('auditLog.title')" @click="openAuditLog"><FileText /></button>
        <span class="toolbar-separator" aria-hidden="true" />
        <div>
          <Popover :open="columnsOpen" @update:open="(open) => { if (!open) columnsOpen = false; }">
            <PopoverAnchor as-child>
              <button class="icon-button icon-violet" :title="t('sftpPane.preferences')" :aria-label="t('sftpPane.preferences')" :disabled="panelSurface" @pointerdown.stop @click.stop="toggleColumnsMenu"><SlidersHorizontal /></button>
            </PopoverAnchor>
            <PopoverContent class="popover columns-popover" align="end" :side-offset="5">
            <!-- 单栏树不再有表头多列：这里只保留 SFTP 面板的默认开合/自动
                 连接偏好，不再声称可自定义列（visibleColumns 仅作持久化兼容）。 -->
            <label :title="t('sftpPane.defaultOpenHint')"><input type="checkbox" :checked="sftpPaneDefaultOpen" @change="toggleSftpPaneDefaultOpen" />{{ t("sftpPane.defaultOpen") }}</label>
            <label :title="t('sftpPane.autoConnectHint')"><input type="checkbox" :checked="sftpAutoConnect" @change="toggleSftpAutoConnect" />{{ t("sftpPane.autoConnect") }}</label>
            </PopoverContent>
          </Popover>
        </div>
        <div>
          <!-- 历史卡右键菜单（ContextMenu）打开时忽略弹层的外点关闭请求：
               菜单项 pointerdown 相对弹层是"外部"，不加守卫会在 select 前把宿主弹层
               连同菜单一起卸载，动作丢失。 -->
          <button class="icon-button icon-blue" :title="t('transfers')" :aria-label="t('transfers')" :disabled="panelSurface" @pointerdown.stop @click.stop="toggleTransferPanel"><ArrowUpDown /><span v-if="activeTransfers" class="activity-dot" /></button>

        </div>
        </template>
      </div>
      <!-- dock 面板左组：系统信息带（CPU/内存/网速，5s 轮询），flex:1 把
           右侧按钮组推到右缘；数据未就绪/不支持时整条隐藏不占位。 -->
      <div v-if="panelSurface" class="panel-toolbar-left">
        <DockMetricsStrip
          :panel-metrics-visible="panelMetricsVisible"
          :panel-metrics-failed="panelMetricsFailed"
          :panel-cpu-percent="panelCpuPercent"
          :panel-mem-percent="panelMemPercent"
          :panel-net-rates="panelNetRates"
          :panel-metrics-error-title="panelMetricsErrorTitle"
          :format-rate="formatRate"
          :t="t"
        />
      </div>
      <!-- Dock 面板表面（底部栏）的精简工具区：宿主 dock chrome 已带标题与
           关闭/收起/最大化，这里只留终端快捷操作——字号、录屏、传输、配色快切、
           设置与工具条收起，密度对齐底部栏（紧凑 30px 工具条 + 透明 ghost 按钮）。 -->
      <div v-if="panelSurface" class="toolbar-actions panel-actions">
        <!-- 在 tab 打开当前连接（宿主 openWorkbench → 新 tab，panel 自己保持轻量）。 -->
        <button v-if="!localUiMode && !isRdpConnection" class="icon-button compact" :title="t('newSessionTab')" :aria-label="t('newSessionTab')" :disabled="!connectionId" @click="openNewSessionTab"><ExternalLink /></button>
        <!-- SFTP 文件面板开关：dock 默认不开，手动开合（连接后列表已就绪）。 -->
        <button v-if="!localUiMode && !isRdpConnection" class="icon-button compact" :class="{ 'is-active': sftpPaneOpen && sftpViewActive }" :title="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-label="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-pressed="sftpPaneOpen && sftpViewActive" @click="toggleSftpPane"><FolderOpen v-if="!(sftpPaneOpen && sftpViewActive)" /><PanelRightClose v-else /></button>
        <button v-if="!isRdpConnection" class="icon-button compact" :class="{ 'is-recording': recordingActive }" :title="recordingActive ? t('recordingStop') : t('recordingTitle')" :aria-label="recordingActive ? t('recordingStop') : t('recordingTitle')" :disabled="!recordingTarget" @click="toggleRecording"><Disc /></button>
        <button v-if="!isRdpConnection" class="icon-button compact" :title="t('transfers')" :aria-label="t('transfers')" @pointerdown.stop @click.stop="toggleTransferPanel"><ArrowUpDown /><span v-if="activeTransfers" class="activity-dot" /></button>
        <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
        <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
        <Popover v-if="!isRdpMode" :open="panelPaletteOpen" @update:open="(open) => { if (!open) panelPaletteOpen = false; }">
          <PopoverAnchor as-child>
            <button class="icon-button compact icon-violet" :class="{ 'is-active': panelPaletteOpen }" :title="t('terminalAppearance.schemeSection')" :aria-label="t('terminalAppearance.schemeSection')" :aria-expanded="panelPaletteOpen" @pointerdown.stop @click.stop="togglePanelPaletteMenu"><Palette /></button>
          </PopoverAnchor>
          <PopoverContent class="popover panel-palette-popover" align="end" :side-offset="4">
            <h3>{{ t("terminalAppearance.schemeSection") }}</h3>
            <ul class="panel-palette-list" role="listbox">
              <li>
                <button type="button" class="panel-palette-row" :class="{ selected: panelActiveSchemeId === null }" role="option" :aria-selected="panelActiveSchemeId === null" @click="pickPanelScheme(null)">
                  <span class="panel-palette-name">{{ t("terminalAppearance.modeFollowHost") }}</span>
                </button>
              </li>
              <li v-for="scheme in panelSchemeChoices" :key="scheme.id">
                <button type="button" class="panel-palette-row" :class="{ selected: panelActiveSchemeId === scheme.id }" role="option" :aria-selected="panelActiveSchemeId === scheme.id" :title="scheme.name" :aria-label="scheme.name" @click="pickPanelScheme(scheme.id)">
                  <span class="panel-palette-name">{{ scheme.name }}</span>
                  <span class="panel-palette-swatch" aria-hidden="true"><i v-for="(color, index) in scheme.colors.slice(0, 6)" :key="index" :style="{ background: color }" /></span>
                </button>
              </li>
            </ul>
            <footer class="panel-palette-footer">
              <button type="button" class="link-button" @click="panelPaletteOpen = false; openSettings()"><Settings />{{ t("terminalAppearance.moreSettings") }}</button>
            </footer>
          </PopoverContent>
        </Popover>
        <button v-if="!isRdpConnection" class="icon-button compact" :title="t('settings')" :aria-label="t('settings')" @click="openSettings"><Settings /></button>
        <button class="icon-button compact" :title="t('panel.hideToolbar')" :aria-label="t('panel.hideToolbar')" :aria-pressed="panelToolbarHidden" @click="togglePanelToolbar"><ChevronUp /></button>
      </div>
      <!-- 传输弹层单实例（tab/panel 两表面共用）：Popover 挂在 header 层，锚点是
           工具条右缘下的隐形定位点（.transfer-popover-anchor，固定定位不随表面
           工具条高度变化）；tab 工具条与 dock panel-actions 里的按钮都只是
           toggleTransferPanel 的普通开关——panel 没有这组 tab 工具条实例，
           共享锚点让同一个弹层在两个表面都能弹出（内容单拷贝零复制）。 -->
      <TransferPopover
        :transfer-panel-open="transferPanelOpen"
        :transfer-list="transferList"
        :transfer-history="transferHistory"
        :resumable-tasks="resumableTasks"
        :folder-upload-progress="folderUploadProgress"
        :file-batch-progress="fileBatchProgress"
        :paused-task-ids="pausedTaskIds"
        :transfer-speeds="transferSpeeds"
        :transfer-history-menu="transferHistoryMenu"
        :transfer-history-loading="transferHistoryLoading"
        :transfer-history-failed="transferHistoryFailed"
        :resumable-loading="resumableLoading"
        :folder-upload-percent="folderUploadPercent"
        :batch-upload-percent="batchUploadPercent"
        :format-bytes="formatBytes"
        :transfer-percent="transferPercent"
        :transfer-bar-value="transferBarValue"
        :transfer-shown-bytes="transferShownBytes"
        :transfer-bytes-title="transferBytesTitle"
        :transfer-pausable="transferPausable"
        :t="t"
        @update:transferPanelOpen="(value) => { if (!value && !transferHistoryMenu) transferPanelOpen = false; }"
        @close-history-menu="transferHistoryMenu = undefined"
        @show-history-menu="showTransferHistoryMenu"
        @toggle-pause="toggleTransferPause"
        @cancel="cancelTransfer"
        @reveal="revealTransferTarget"
        @open-file="openTransferTarget"
        @refresh="refreshTransferPanel"
        @open-clear="transferHistoryClearOpen = true"
        @begin-resume="beginResumeUpload"
        @resume-picked="onResumeFilePicked"
      />
    </header>

    <div v-if="tooltip" ref="tooltipBubble" class="app-tooltip" :class="{ 'app-tooltip-above': tooltip.above }" :style="{ left: `${tooltip.x}px`, top: `${tooltip.y}px`, '--arrow-offset': `${tooltip.arrowOffset}px` }" role="tooltip">{{ tooltip.text }}</div>
    <ToastProvider :label="t('notification')">
      <ToastRoot v-if="notice" :key="noticeKey" :open="noticeOpen" :duration="noticeActions.length ? 8000 : 3500" class="notice" @update:open="onNoticeOpenChange">
        <span>{{ notice }}</span>
        <ToastAction v-for="action in noticeActions" :key="action.label" :alt-text="action.label" class="notice-action" @click="action.run()">{{ action.label }}</ToastAction>
      </ToastRoot>
      <ToastViewport class="notice-viewport" />
    </ToastProvider>
    <ToastProvider :label="t('notification')">
      <ToastRoot v-if="sftpError" :key="sftpErrorKey" :open="sftpErrorOpen" :duration="8000" class="error-banner" @update:open="onSftpErrorOpenChange">
        <span>{{ sftpError }}</span>
        <ToastAction v-if="sftpErrorRetry" :alt-text="t('retry')" class="notice-action" @click="sftpErrorRetry()">{{ t("retry") }}</ToastAction>
        <ToastClose :title="t('close')"><X /></ToastClose>
      </ToastRoot>
      <ToastViewport class="error-viewport" />
    </ToastProvider>

    <section ref="paneContainer" :class="orderedPaneClass" :style="paneSplitStyle">
      <!-- 终端栈：编辑器页签在上、终端在下。SFTP 分栏比（terminalBasis）施加
           在栈容器上，内层终端 pane 只负责填满栈内剩余高度；无页签时编辑器
           与把手不渲染，终端回到整栈高度。 -->
      <section ref="terminalStackRef" class="terminal-stack" :style="terminalBasis">
      <EditorPane
        v-if="hasEditorTabs"
        ref="editorPaneRef"
        :style="{ flexBasis: `${editorSplitRatio}%` }"
        :tabs="editorTabs"
        :active-id="activeEditorId"
        :t="t"
        :locale="locale"
        :appearance="appearance"
        @activate="activateEditorTab"
        @close="closeEditorTab"
        @save="saveEditorTab"
        @update:text="({ id, text }) => updateEditorText(id, text)"
      />
      <div
        v-if="hasEditorTabs"
        class="editor-divider"
        role="separator"
        aria-orientation="horizontal"
        :aria-valuenow="Math.round(editorSplitRatio)"
        aria-valuemin="25"
        aria-valuemax="75"
        tabindex="0"
        @pointerdown="startEditorDividerDrag"
        @keydown="onEditorDividerKeydown"
      />
      <div
        class="terminal-workspace"
        @dragenter.prevent="onTerminalDragEnter"
        @dragover.prevent
        @dragleave="onTerminalDragLeave"
        @drop.prevent="onTerminalDrop($event)"
      >
      <TerminalLayoutTree
        :node="terminalLayout"
        :connection-id="connectionId || ''"
        :workbench-id="workbenchId"
        :main-session-id="session?.sessionId || ''"
        :theme="splitTerminalTheme"
        :font-family="splitTerminalFont.fontFamily"
        :font-size="splitTerminalFont.fontSize"
        :t="t"
        :locale="locale"
        :can-split="canSplitTerminal"
        :can-create-tab="canOpenShellTab"
        :engines="ctxSearchEngines"
        :connected="connected"
        :can-write="canWrite"
        :transfer-busy="terminalTransferBusy"
        :notify="showNotice"
        @opened="onShellTabOpened"
        @activate-tab="activatePaneTab"
        @close-tab="closePaneTab"
        @new-tab="createPaneTab"
        @close-pane="closePane"
        @split="splitTerminalPane"
        @search-online="searchTextOnline"
        @cwd="onShellTabCwd"
        @activate-pane="onPaneActivated"
      >
        <template #main>
      <ContextMenu :open="terminalMenuOpen" @update:open="(open) => { if (!open) terminalMenuOpen = false; }">
        <ContextMenuTrigger as-child>
      <section class="terminal-pane" :class="{ 'drag-active': terminalDragActive, 'marker-visible': commandMarker.installed, 'gutter-visible': gutterPaneVisible, 'wallpaper-active': wallpaperActive, ...transferBusyPaneClass(terminalTransferBusy) }" :style="gutterPaneStyle" @contextmenu="showTerminalMenu">
        <!-- P2-9 背景图层：pointer-events:none 垫底（DOM 序先于 terminal-host），
             透明度 0.1-0.9 由设置页滑杆控制；开启期间强制 DOM 渲染器透出本层。 -->
        <div v-if="wallpaperActive" class="terminal-wallpaper" :style="{ backgroundImage: `url(${wallpaperDataUrl})`, opacity: wallpaperOpacity / 100 }" aria-hidden="true" />
        <!-- P1-3 行号/时间戳 gutter：绝对定位覆盖左缘 padding 环带（z-index 1，
             低于浮层 z-index 2），xterm 左 padding 随 --dbx-gutter-width 加宽，
             不遮文本；drop-overlay/搜索面板/诊断浮层定位不受影响。 -->
        <TerminalGutter v-if="gutterVisible" :rows="gutterRows" :width="gutterWidth" />
        <div ref="terminalHost" class="terminal-host" :class="{ 'bell-flash': terminalBellFlash }" @mousedown.middle="handleTerminalMiddleClick" />
        <!-- VNC 画布（nyaterm-parity P2 2d）：盖在 xterm 之上（z-index 2），
             帧从 vnc/frame/{id} 二进制通道进入；键盘/鼠标由画布采集后经
             keysym 映射发 vnc/input。连接态/退出覆盖层沿用 terminal-overlay。 -->
        <VncSurface v-if="isVncMode" ref="vncSurface" class="vnc-surface" :scale-mode="vncScaleMode" @input="sendVncInput" @clipboard-out="sendVncClipboard" />
        <!-- RDP 画布（nyaterm-parity P3-4）：盖在 xterm 之上（z-index 2），帧从
             rdp/frame/{id} 二进制通道进入（与 vnc/frame 同一 44 字节 patch 头）；
             键盘经扫描码映射、鼠标/滚轮原样映射发 rdp/input，服务端光标形状
             （rdp/pointer）落到画布 CSS cursor。连接态/退出覆盖层沿用
             terminal-overlay。 -->
        <RdpSurface v-if="isRdpMode" ref="rdpSurface" class="vnc-surface" :scale-mode="rdpScaleMode" @input="sendRdpInput" @clipboard-out="sendRdpClipboard" @desktop-size="(size) => (rdpDesktopSize = size)" />
        <!-- P1-2 动作链接命令预览浮签：悬停 / Alt+点击时显示建议命令文本。 -->
        <div v-if="actionLinkHint" class="action-link-hint mono" :style="{ left: `${actionLinkHint.x}px`, top: `${actionLinkHint.y}px` }">{{ actionLinkHint.text }}</div>
        <!-- #33/#71 快速输入丢失诊断浮层：Ctrl/Cmd+Shift+D 切换。keys=onData
             路由到 PTY 的按键、sends=提交宿主桥的帧、acks=sidecar 确认的帧、
             errors=桥拒绝、swallowed=传输路由吞键。三者对不上即锁定丢失层。 -->
        <div v-if="terminalDiagVisible" class="terminal-diag-overlay" @dblclick="terminalDiagVisible = false">
          keys {{ terminalDiag.keys }} · sends {{ terminalDiag.sends }} · acks {{ terminalDiag.acks }} · errors {{ terminalDiag.errors }} · swallowed {{ terminalDiag.swallowed }}
        </div>
        <!-- 空提示符快捷键引导条（引导 Warp 对齐键位）：空行静置时浮在光标行
             上方，用过任一功能或 ✗ 永久消散（lib/terminalPromptHints.ts 门）；
             定位与建议浮层同一光标锚点坐标系，底边贴光标行顶不遮输入行。 -->
        <TerminalPromptHints
          v-if="promptHintsVisible"
          :locale="locale"
          :ai-search-available="aiBridgeOk() && aiSettingsState.search"
          :history-key-label="promptHintsHistoryKey"
          :completions-key-label="promptHintsCompletionsKey"
          :anchor="promptHintsAnchor"
          :viewport="suggestionViewport"
          @dismiss="dismissPromptHints"
        />
        <!-- 命令模糊建议浮层（P1-1）：锚点为光标像素坐标，读不到时贴终端底部；
             键盘（↑↓/Tab/Enter/Esc）由 handleTerminalKey 在浮层开启时优先消费。 -->
        <CommandSuggestions
          v-if="suggestionOpen && suggestionItems.length"
          :items="suggestionItems"
          :active-index="suggestionActiveIndex"
          :anchor="suggestionAnchor"
          :viewport="suggestionViewport"
          :t="t"
          @activate="(index) => (suggestionActiveIndex = index)"
          @fill="fillSuggestion"
          @ignore="ignoreSuggestion"
        />
        <!-- 结构化补全浮层（FIG wave-1 最终架构；批次 2-1 两段渲染）：fig 引擎
             候选（CompletionItem）直用展示，keyboard.ts 规则表由
             handleCompletionKey 消费，点击回传 item 由 controller 执行 edit；
             generator 在途且无静态候选时显示 loading 占位行（Tab/Enter 放行
             shell，keyboard.ts loading 态）。 -->
        <CompletionMenu
          v-if="completionOpen && (completionItems.length || completionLoading)"
          :items="completionItems"
          :active-index="completionActiveIndex"
          :anchor="completionAnchor"
          :viewport="suggestionViewport"
          :loading="completionLoading"
          :t="t"
          @activate="(index) => (completionActiveIndex = index)"
          @accept="acceptCompletionRow"
        />
        <!-- 终端行内 ghost 自动建议（对标 Warp/fish）：灰色剩余文本盖在光标右侧，
             接受键可配置（批 4e，默认 →；Ctrl+→ 逐词），handleTerminalKey 消费；
             #138 批 2 起与结构化补全菜单同屏共存。overlay DOM 而非 xterm
             decoration 的理由见 ghost 函数块注释。 -->
        <div
          v-if="ghostMatch && ghostAnchor"
          class="terminal-ghost mono"
          :style="{ left: `${ghostAnchor.x}px`, top: `${ghostAnchor.y}px` }"
        >
          <span aria-hidden="true">{{ ghostMatch.remainder }}</span>
          <!-- 键位胶囊（Warp accept-autosuggestion-hint 同位）：pointer-events
               独立放开供点击改绑菜单；ghost 容器其余区域仍不可交互。 -->
          <button
            v-if="!suggestionOpen"
            type="button"
            class="terminal-ghost-keycap"
            :title="t('terminalGhost.changeKey')" :aria-label="t('terminalGhost.changeKey')"
            @click.stop.prevent="ghostKeycapMenuOpen = !ghostKeycapMenuOpen"
            @mousedown.stop.prevent
          >{{ ghostAcceptKeyLabel }}</button>
          <div v-if="ghostKeycapMenuOpen" class="terminal-ghost-keymenu">
            <button type="button" class="terminal-ghost-keyoption" :class="{ active: !ghostTabAccept }" @click.stop.prevent="setGhostTabAccept(false)">→</button>
            <button type="button" class="terminal-ghost-keyoption" :class="{ active: ghostTabAccept }" @click.stop.prevent="setGhostTabAccept(true)">Tab</button>
            <div class="terminal-ghost-keymenu-note">{{ t("terminalGhost.keysSummary") }}</div>
          </div>
        </div>
        <!-- # AI 命令搜索模式提示（Warp AI Command Search 同位）：激活后 onData
             全部改道本地状态机、字节不进 PTY，提示条浮在光标行上方展示 query 与
             键位（底边贴光标行顶，不遮输入行）；回车发起宿主 AI 会话（ask），
             结果由用户复制回填，插件不代执行。 -->
        <div
          v-if="(aiSearchState.active || aiSearchPending) && aiSearchAnchor"
          ref="aiSearchBarEl"
          class="terminal-ai-search mono"
          :class="{ pending: aiSearchPending }"
          :style="aiSearchBarStyle"
          role="status"
        >
          <span class="terminal-ai-search-badge" aria-hidden="true">#</span>
          <template v-if="aiSearchPending">
            <span class="terminal-ai-search-query">{{ aiSearchPendingQuery }}</span>
            <span class="terminal-ai-search-keys terminal-ai-search-pending"><Loader2 class="spinning" aria-hidden="true" /> {{ t("aiSearch.pendingHint") }}<span aria-hidden="true"> · </span><kbd>Esc</kbd> {{ t("aiSearch.pendingEscHint") }}</span>
          </template>
          <template v-else>
            <span class="terminal-ai-search-query" :class="{ placeholder: !aiSearchState.query }">{{ aiSearchState.query || t("aiSearch.hintPlaceholder") }}</span>
            <span class="terminal-ai-search-keys"><kbd>Enter</kbd> {{ t("aiSearch.hintSubmit") }}<span aria-hidden="true"> · </span><kbd>Esc</kbd> {{ t("aiSearch.hintCancel") }}</span>
          </template>
        </div>
        <!-- 失败命令修复条（Warp Fix-with-AI 同位）：非弹窗不抢焦点，贴触发光标
             行顶；新命令开始/Esc 关闭/会话切换即消失。「用 AI 修复」首次发送过
             快照预览确认，输出已截断+脱敏（prepareAiOutputSnapshot）。 -->
        <div v-if="aiFixBar" class="terminal-ai-fix mono" :style="aiFixBarStyle" role="status">
          <span class="terminal-ai-fix-exit">✗ {{ aiFixBar.exitCode }}</span>
          <span class="terminal-ai-fix-command">{{ aiFixBar.command }}</span>
          <button type="button" class="terminal-ai-fix-btn" @click="aiFixFromBar">{{ t("aiFix.barAction") }}</button>
          <button type="button" class="terminal-ai-fix-close" :title="t('aiFix.barClose')" :aria-label="t('aiFix.barClose')" @click="aiFixBar = null; clearFixRecommendation()">✕</button>
        </div>
        <div v-if="!isRdpConnection && !localUiMode && (terminalDragActive || (dragActive && !sftpViewVisible))" class="drop-overlay"><FileUp /><strong>{{ t("terminalDrop.hint") }}</strong></div>
        <TerminalSearchPanel
          v-if="searchOpen"
          :locale="locale"
          :initial-query="searchSeedQuery"
          :initial-options="searchSeedOptions"
          :match-state="searchMatchState"
          :result-index="searchResultIndex"
          :result-count="searchResultCount"
          @find-next="(query, options) => runTerminalSearch(query, options, 'next')"
          @find-previous="(query, options) => runTerminalSearch(query, options, 'prev')"
          @clear="clearTerminalSearch"
          @close="closeTerminalSearch"
        />
        <!-- Quick Select Mode（WT-1）：注册表 quick-select 动作唤起，逐项复制命中；
             Esc/↑↓/Enter 由 handleTerminalKey 的浮层分支消费，焦点不离开终端。 -->
        <TerminalQuickSelectPanel
          v-if="quickSelectOpen"
          :locale="locale"
          :hits="quickSelectHits"
          :active-index="quickSelectActive"
          @activate="(index) => (quickSelectActive = index)"
          @copy="copyQuickSelectHit"
          @close="closeQuickSelect"
        />
        <!-- Warp 式 history 面板（↑ 唤起）：commandHistory 可视化快速回填，
             选中（打开/↑↓/点击/搜索过滤）即实时回填输入行（仅回填不执行），
             悬停只浏览不抢高亮不改输入；面板内搜索框聚焦过滤，↑↓/Enter/Tab/
             Esc 经 panel-key 转发给 handleHistoryPanelKey（焦点留在输入框，
             回填后回终端），锚点定位与翻转语义同建议浮层。 -->
        <TerminalHistoryPanel
          v-if="historyPanelOpen"
          :locale="locale"
          :entries="historyPanelEntries"
          :active-index="historyPanelActiveIndex"
          :anchor="historyPanelAnchor"
          :query="historyPanelQuery"
          :viewport="suggestionViewport"
          :focus-search-on-mount="historyPanelFocusSearch"
          @update:query="(value: string) => { historyPanelQuery = value; updateHistoryPanelFilter(true); }"
          @panel-key="handleHistoryPanelPanelKey"
          @select="selectHistoryEntry"
          @close="closeHistoryPanel"
        />
        <div v-if="reconnectPending" class="reconnect-banner" role="status">
          <Loader2 class="spinning" />
          <span class="reconnect-text">{{ t("reconnectBanner.label") }}</span>
          <span v-if="reconnectCountdown" class="reconnect-countdown mono">{{ t("sessionStatus.reconnectCountdown", { seconds: reconnectCountdown.seconds, attempt: reconnectCountdown.attempt }) }}</span>
          <span v-if="reconnectCountdown" class="reconnect-attempt">{{ t("reconnectBanner.attempt", { attempt: reconnectCountdown.attempt }) }}</span>
          <progress v-if="reconnectCountdown" :value="reconnectCountdown.percent" max="100" />
          <button class="reconnect-now" @click="reconnectNow">{{ t("reconnectNow") }}</button>
        </div>
        <!-- 录制中悬浮控制条：底部居中，红色呼吸点 + 时长 + 停止 -->
        <div v-if="recordingActive" class="recording-float" role="status">
          <Disc class="recording-float-dot" />
          <span class="recording-elapsed mono">{{ formatDuration(recordingElapsedSec) }}</span>
          <button class="recording-stop" @click="toggleRecording"><Square />{{ t("recordingStop") }}</button>
        </div>
        <!-- 录制开始倒计时遮罩：大数字 3→2→1（:key 重触发放缩动画），Esc/点击取消 -->
        <div v-if="recordCountdown !== null" class="record-countdown-overlay" role="status" @click="cancelRecordCountdown">
          <span class="record-countdown-number" :key="recordCountdown">{{ recordCountdown }}</span>
          <span class="record-countdown-hint">{{ t("recordingCountdownHint") }}</span>
        </div>
        <!-- SSH 连接卡片：本地/串口/Telnet/VNC 会话占用的终端视图不再叠 SSH-only
             卡片（互斥展示）。Dock panel surface 不再显示瞬态连接遮罩（宿主
             预拨号、面板直出终端区域），但 FAILED/disconnected 是死路态——
             连接卡（错误文案 + 重连出口）在面板内同样要呈现，否则面板读作空白。 -->
        <div
          v-if="(!panelSurface || terminalState === 'error' || terminalState === 'disconnected') && !isLocalMode && !localShellRestored && !isSerialMode && !isTelnetMode && !isVncMode && !isRdpConnection && !isRdpMode && terminalState !== 'connected' && !reconnectPending"
          class="terminal-overlay"
        >
          <ConnectingCard
            :locale="locale"
            :name="connection.name || connectionIdentity"
            :identity="connectionIdentity"
            :state="connectCardState"
            :status-text="inactiveWaiting ? t('connectCard.waitingReopen') : ''"
            :error-text="terminalErrorFriendly || terminalError || t('disconnected')"
            :error-detail="terminalErrorDetail"
            :logs-open="connectLogsOpen"
            :logs="connectLogEntries"
            @cancel="cancelConnect"
            @reconnect="reconnect"
            @connect="startConnect"
            @toggle-logs="connectLogsOpen = !connectLogsOpen"
          />
        </div>
        <!-- 本地终端启动中覆盖层：spawn 正常在数百毫秒内返回；卡在 starting
             （宿主桥丢响应、慢盘冷启动）必须有可见反馈，否则就是零信息黑屏。
             超时/失败由 startLocalTerminal 的 catch 接管转 exited + showError。 -->
        <div v-if="(isLocalMode || localShellRestored) && localState === 'starting'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <Loader2 class="local-exit-icon spinning" />
            <strong>{{ t("localTerminal.starting") }}</strong>
          </div>
        </div>
        <!-- 本地终端退出态覆盖层（含 A4 恢复外壳：restored tab 未起 shell 时
             也由它承担外壳态，spec §8.4）：显示退出码（sidecar 未上报时不显
             示），给重开与关闭两个出口（VS Code 式终端退出体验）。 -->
        <div v-if="(isLocalMode || localShellRestored) && localState === 'exited'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <TerminalIcon class="local-exit-icon" />
            <strong>{{ t("localTerminal.exited") }}</strong>
            <span v-if="localExitCode !== null" class="mono local-exit-code">{{ t("localTerminal.exitCode", { code: localExitCode }) }}</span>
            <div class="local-exit-actions">
              <button class="primary-button" @click="restartLocalTerminal()">{{ t("localTerminal.restart") }}</button>
              <button @click="localShellRestored ? dismissRestoredLocalShell() : closeLocalTerminal()">{{ t("localTerminal.close") }}</button>
            </div>
          </div>
        </div>
        <!-- Telnet 退出覆盖层（连接失败/对端断开）：给出关闭出口，展示
             sidecar 带回的原因文本（明文协议告警在连接弹窗里）。 -->
        <div v-if="isTelnetMode && telnetState === 'closed'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <TriangleAlert class="local-exit-icon" />
            <strong>{{ t("telnet.closed") }}</strong>
            <span v-if="telnetError" class="mono local-exit-code">{{ telnetError }}</span>
            <div class="local-exit-actions">
              <button @click="closeTelnetSession">{{ t("telnet.close") }}</button>
            </div>
          </div>
        </div>
        <!-- 串口退出覆盖层（读线程 IO 失败/设备拔线）：给出关闭出口，展示原因。 -->
        <div v-if="isSerialMode && serialState === 'closed'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <TriangleAlert class="local-exit-icon" />
            <strong>{{ t("serial.closed") }}</strong>
            <span v-if="serialError" class="mono local-exit-code">{{ serialError }}</span>
            <div class="local-exit-actions">
              <button @click="closeSerialSession">{{ t("serial.close") }}</button>
            </div>
          </div>
        </div>
        <!-- VNC 退出覆盖层（认证失败/服务端强制 Tight/对端断开）：给出关闭
             出口，展示 sidecar 带回的原因文本；明文/经典认证告警在连接弹窗。 -->
        <div v-if="isVncMode && vncState === 'closed'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <TriangleAlert class="local-exit-icon" />
            <strong>{{ t("vnc.closed") }}</strong>
            <span v-if="vncError" class="mono local-exit-code">{{ vncError }}</span>
            <div class="local-exit-actions">
              <button @click="closeVncSession">{{ t("vnc.close") }}</button>
            </div>
          </div>
        </div>
        <!-- RDP 重连状态条（退避梯子进行中）：非阻塞展示进度，梯子跑完由
             closed 覆盖层接管（协议：证书/认证/协商失败永不自动重试）。 -->
        <div v-if="isRdpMode && rdpState.state === 'reconnecting'" class="zmodem-status" role="status">
          <Loader2 class="spinning" />
          <span>{{ rdpState.maxAttempts > 0 ? t("rdp.reconnectingAttempt", { attempt: rdpState.attempt, max: rdpState.maxAttempts }) : t("rdp.reconnecting") }}</span>
        </div>
        <!-- RDP 退出覆盖层（graceful close / 终态错误）：errorKind 友好文案为
             主行、sidecar 原因文本为细节行；给出 Reconnect（rdp/reconnect）与
             关闭双出口（协议：服务端主动断开不自动重连，由用户决定）。 -->
        <div v-if="isRdpMode && rdpState.state === 'closed'" class="terminal-overlay">
          <div class="local-exit-card" role="status">
            <TriangleAlert class="local-exit-icon" />
            <strong>{{ rdpClosedTitle }}</strong>
            <span v-if="rdpState.error" class="mono local-exit-code">{{ rdpState.error }}</span>
            <div class="local-exit-actions">
              <button class="primary-button" @click="reconnectRdpSession">{{ t("rdp.reconnect") }}</button>
              <!-- 认证失败是永久错误：重连只会用同一套凭据再撞一次（协议：
                   认证失败永不自动重试），把用户带回可改凭据的连接对话框。 -->
              <button v-if="rdpState.errorKind === 'authentication'" @click="rdpDialogOpen = true">{{ t("rdp.editCredentials") }}</button>
              <button @click="closeRdpSession">{{ t("rdp.close") }}</button>
            </div>
          </div>
        </div>
        <div v-if="commandMarker.installed" class="terminal-command-marker" :class="{ active: commandMarker.active, failed: !commandMarker.active && commandMarker.exitCode !== null && commandMarker.exitCode !== 0 }" :title="commandMarkerDetails" @click="terminal?.focus()">
          <Loader2 v-if="commandMarker.active" class="spinning" />
          <TriangleAlert v-else-if="commandMarker.exitCode" />
          <Info v-else />
          <span v-if="commandMarker.active" class="marker-text">{{ t("terminalCommand.running", { command: commandMarker.command || "…" }) }}</span>
          <span v-if="commandMarker.active && commandMarkerElapsed !== null" class="marker-elapsed mono">{{ formatCommandDuration(commandMarkerElapsed) }}</span>
          <span v-else-if="commandMarker.exitCode !== null" class="marker-text">{{ t("terminalCommand.finished", { code: commandMarker.exitCode, duration: formatCommandDuration(commandMarker.durationMs) }) }}</span>
          <span v-else class="marker-text">{{ t("terminalCommand.hint") }}</span>
          <span v-if="commandMarker.cwd" class="marker-cwd mono">{{ commandMarker.cwd }}</span>
        </div>
        <div v-if="agentRunning" class="agent-run-banner" role="status">
          <Loader2 class="spinning" />
          <span class="agent-run-text">{{ t("agentRunningBanner") }}</span>
          <code class="agent-run-command mono" :title="agentRunning.command">{{ agentRunning.command }}</code>
          <button class="agent-interrupt" @click="interruptAgentRun">{{ t("agentInterrupt") }}</button>
        </div>
        <div v-if="zmodemBusy" class="zmodem-status trzsz-status" role="status">
          <Loader2 class="spinning" />
          <span class="trzsz-label">{{ zmodemStatusLabel }}</span>
          <progress v-if="zmodemState === 'uploading' || zmodemState === 'receiving'" :value="zmodemPercent" max="100" />
          <span v-if="(zmodemState === 'uploading' || zmodemState === 'receiving') && zmodemFileCount > 1" class="trzsz-count mono">{{ zmodemFileIndex }}/{{ zmodemFileCount }}</span>
          <span v-if="zmodemSpeed">{{ formatBytes(zmodemSpeed) }}/s</span>
          <button class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="cancelZmodemUpload"><X /></button>
        </div>
        <div v-if="trzszOverlayVisible" class="zmodem-status trzsz-status" role="status" :class="{ 'trzsz-done': trzszPhase === 'success', 'trzsz-failed': trzszPhase === 'failed' }">
          <Loader2 v-if="trzszPhase === 'waiting' || trzszPhase === 'transferring'" class="spinning" />
          <TriangleAlert v-else-if="trzszPhase === 'failed'" />
          <span class="trzsz-label">{{ trzszStatusLabel }}</span>
          <progress v-if="trzszPhase === 'transferring'" :value="trzszPercent" max="100" />
          <span v-if="trzszPhase === 'transferring' && trzszFileCount > 1" class="trzsz-count mono">{{ trzszFileIndex }}/{{ trzszFileCount }}</span>
          <span v-if="trzszPhase === 'transferring' && trzszSpeed" class="trzsz-speed">{{ formatBytes(trzszSpeed) }}/s</span>
          <button v-if="trzszBusy" class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="cancelTrzszTransfer"><X /></button>
        </div>
        <!-- 串口文件上传进度（NyaTerm 对齐 P0-3）：running 吞键入 + 可取消；
             complete/failed 保留展示，由用户点 × 收起。 -->
        <div v-if="serialUploadOverlayVisible" class="zmodem-status trzsz-status" role="status" :class="{ 'trzsz-done': serialUpload.phase === 'complete', 'trzsz-failed': serialUpload.phase === 'failed' }">
          <Loader2 v-if="serialUploadBusy" class="spinning" />
          <TriangleAlert v-else-if="serialUpload.phase === 'failed'" />
          <span class="trzsz-label">{{ serialUploadStatusLabel }}</span>
          <progress v-if="serialUploadBusy" :value="serialUploadPercentValue" max="100" />
          <span class="trzsz-count mono">{{ serialUpload.protocol.toUpperCase() }}</span>
          <button v-if="serialUploadBusy" class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="cancelSerialUpload"><X /></button>
          <button v-else class="trzsz-cancel" :title="t('close')" :aria-label="t('close')" @click="serialUpload = initialSerialUploadState()"><X /></button>
        </div>
        <!-- 录制记录浮条：列出 .cast 录制，可回放/导出/删除；
             M14 增加名称+内容搜索（命中行摘录）与 transcript 导出。 -->
        <section v-if="recordingsOpen" class="metrics-float recordings-float">
          <header>
            <h2>{{ t("recordingsTitle") }}</h2>
            <button v-if="recordings.length" class="icon-button recording-delete" :title="t('recordingsClear')" :aria-label="t('recordingsClear')" :disabled="recordingClearAllSubmitting" @click="recordingClearAllOpen = true"><Trash2 /></button>
            <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="toggleRecordings"><X /></button>
          </header>
          <div class="recordings-search-row" v-if="recordings.length">
            <Search class="recordings-search-icon" />
            <input v-model="recordingsQuery" class="recordings-search" type="search" :placeholder="t('recordingsSearchPlaceholder')" :aria-label="t('recordingsSearchPlaceholder')" />
            <button v-if="recordingsQuery" class="icon-button compact" :title="t('close')" :aria-label="t('close')" @click="clearRecordingsSearch"><X /></button>
          </div>
          <div class="metrics-float-body">
            <div v-if="recordingsLoading && !recordings.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
            <div v-else-if="!recordings.length" class="empty compact">{{ t("recordingsEmpty") }}</div>
            <div v-else-if="!filteredRecordings.length" class="empty compact">{{ t("recordingsNoMatch") }}</div>
            <template v-for="item in filteredRecordings" :key="item.recordingId">
              <article class="recording-card">
                <Disc class="recording-icon" />
                <div class="recording-text">
                  <span class="recording-host">{{ item.host || item.recordingId }}</span>
                  <span class="recording-meta">{{ formatRecordedAt(item.startedAt) }}<template v-if="item.bytes"> · {{ formatBytes(item.bytes) }}</template></span>
                  <span v-for="(hit, hitIndex) in recordingHits[item.recordingId] ?? []" :key="hitIndex" class="recording-hit mono">{{ hit }}</span>
                </div>
                <span class="recording-duration mono">{{ formatDuration(item.durationSecs ?? 0) }}</span>
                <div class="recording-actions">
                  <button class="icon-button compact" :title="t('replayOpen')" :aria-label="t('replayOpen')" @click="openReplay(item)"><Play /></button>
                  <button class="icon-button compact" :title="t('replayExportGif')" :aria-label="t('replayExportGif')" :disabled="replayExporting" @click="exportRecordingFromList(item)"><Loader2 v-if="recordingExportingId === item.recordingId" class="spinning" /><ImagePlay v-else /></button>
                  <button class="icon-button compact" :title="t('recordingExportTranscript')" :aria-label="t('recordingExportTranscript')" :disabled="replayExporting" @click="exportRecordingTranscript(item)"><FileText /></button>
                  <button v-if="localCanSave" class="icon-button compact" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click="revealRecording(item)"><FolderOpen /></button>
                  <button class="icon-button compact recording-delete" :title="t('recordingDelete')" :aria-label="t('recordingDelete')" @click="deleteRecording(item)"><Trash2 /></button>
                </div>
              </article>
            </template>
          </div>
        </section>
        <!-- 回放弹窗：xterm 重放 + 倍速/进度/GIF 导出 -->
        <div v-if="replayState" class="replay-overlay" @click.self="closeReplay">
          <section class="replay-modal">
            <header>
              <h2>{{ t("replayTitle") }}<span class="metrics-host"> · {{ replayState.summary.host || replayState.summary.recordingId }}</span></h2>
              <button class="icon-button" :title="t('replayClose')" :aria-label="t('replayClose')" @click="closeReplay"><X /></button>
            </header>
            <div class="replay-terminal-wrap">
              <div ref="replayHost" class="replay-terminal"></div>
              <div v-if="replayDurationMs <= 0" class="replay-empty">{{ t("replayEmpty") }}</div>
            </div>
            <div class="replay-controls">
              <button class="icon-button" :title="t(replayPlaying ? 'replayPause' : 'replayPlay')" :aria-label="t(replayPlaying ? 'replayPause' : 'replayPlay')" @click="toggleReplayPlay"><Pause v-if="replayPlaying" /><Play v-else /></button>
              <Select :model-value="String(replaySpeed)" @update:model-value="(v) => (replaySpeed = Number(v))">
                <SelectTrigger size="xs" class="replay-speed" :title="t('replaySpeed')">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="0.5">0.5×</SelectItem>
                  <SelectItem value="1">1×</SelectItem>
                  <SelectItem value="2">2×</SelectItem>
                  <SelectItem value="4">4×</SelectItem>
                </SelectContent>
              </Select>
              <input class="replay-seek" type="range" min="0" :max="Math.max(1, replayDurationMs)" :value="replayPlayheadMs" step="100" @input="onReplaySeek" />
              <span class="mono replay-time">{{ formatDuration(replayPlayheadMs / 1000) }} / {{ formatDuration(replayDurationMs / 1000) }}</span>
              <button class="link-button" :disabled="replayExporting" @click="exportReplayGif">{{ replayExporting ? t("replayExporting") : t("replayExportGif") }}</button>
            </div>
          </section>
        </div>
      </section>
        </ContextMenuTrigger>
        <!-- P2-8：Copy/Paste/Search online/Close 由 TerminalContextMenu 承载；
             插件自有菜单项经默认插槽保持在原有位置。 -->
        <TerminalContextMenu
          :open="terminalMenuOpen"
          :has-selection="terminal?.hasSelection() ?? false"
          :can-paste="connected && !terminalTransferBusy"
          :engines="ctxSearchEngines"
          :t="t"
          @copy="copyTerminalSelection"
          @paste="pasteTerminal"
          @search="searchSelectionOnline"
          @close="terminalMenuOpen = false"
        >
          <!-- 本地终端最近命令（VS Code Run Recent Command 简化版）：
               依赖 shell integration 注入的 633;E 命令行。 -->
          <template v-if="isLocalMode && localRecentCommands.length">
            <ContextMenuItem @select="rerunLocalCommand(localRecentCommands[0])"><History />{{ t("localTerminal.rerunLast") }}</ContextMenuItem>
            <ContextMenuItem v-for="command in localRecentCommands.slice(0, 5)" :key="command" @select="rerunLocalCommand(command)"><span class="mono local-rerun-command">{{ command }}</span></ContextMenuItem>
            <ContextMenuSeparator />
          </template>
          <ContextMenuItem @select="selectAllTerminal"><TextSelect />{{ t("terminalSelectAll") }}</ContextMenuItem>
          <ContextMenuItem @select="openTerminalSearch"><Search />{{ t("terminalSearch.open") }}</ContextMenuItem>
          <ContextMenuItem @select="clearTerminal"><Eraser />{{ t("terminalClear") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!connected" @select="sendSudoRefresh"><ShieldCheck />{{ t("sudoRefresh.title") }}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem :disabled="!canSplitTerminal" @select="openTerminalSplit('row')"><ArrowLeftRight />{{ t("terminalSplit.leftRight") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!canSplitTerminal" @select="openTerminalSplit('column')"><ArrowUpDown />{{ t("terminalSplit.topBottom") }}</ContextMenuItem>
          <ContextMenuItem v-if="terminalHasSplit" @select="closeTerminalSplit"><X />{{ t("terminalSplit.close") }}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseZmodem"><FileUp />{{ t("zmodemUpload") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseTrzszUpload"><FileUp />{{ t("trzszUpload") }}</ContextMenuItem>
        </TerminalContextMenu>
      </ContextMenu>
        </template>
      </TerminalLayoutTree>
        <!-- 批量发送结果浮条：显示在命令条上方，可手动关闭。 -->
        <div v-if="connected && batchBarOpen && !isRdpConnection && (batchSummary || batchError)" class="batch-bar-status" role="status">
          <template v-if="batchSummary">
            <p :class="batchSummary.failed ? 'task-error' : 'muted'">
              {{ batchSummary.failed ? t("batchSendPartial", { sent: batchSummary.sent, failed: batchSummary.failed }) : t("batchSendSent", { count: batchSummary.sent }) }}
            </p>
            <div v-if="batchSummary.failed" class="batch-result-list">
              <span v-for="row in batchSummary.rows.filter((item) => !item.success)" :key="row.sessionId" class="batch-result-row mono">
                {{ batchSessionLabel(row.sessionId) }} · {{ row.error || t("batchSendFailed") }}
              </span>
            </div>
          </template>
          <p v-else class="task-error">{{ batchError }}</p>
          <button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="dismissBatchResult"><X /></button>
        </div>
        <!-- 批量发送命令条（Electerm quick-command bar）：贴终端底部，回车即发送；
             目标选择/快速命令切换/保存为快速命令均在条上完成。 -->
        <BatchBar
          v-if="connected && batchBarOpen && !isRdpConnection"
          v-model:batchTargetsOpen="batchTargetsOpen"
          v-model:batchQuickOpen="batchQuickOpen"
          v-model:batchQuickSearch="batchQuickSearch"
          v-model:batchSaveName="batchSaveName"
          v-model:batchDraft="batchDraft"
          :batch-loading="batchLoading"
          :batch-sending="batchSending"
          :batch-targets="batchTargets"
          :batch-selected="batchSelected"
          :filtered-batch-quick-commands="filteredBatchQuickCommands"
          :grouped-batch-quick-commands="groupedBatchQuickCommands"
          :batch-quick-pick-name="batchQuickPickName"
          :batch-save-mode="batchSaveMode"
          :batch-saving="batchSaving"
          :quick-commands="quickCommands"
          :quick-has-real-groups="quickHasRealGroups"
          :quick-collapsed-groups="quickCollapsedGroups"
          :quick-command-group-summary="quickCommandGroupSummary"
          :quick-group-name="quickGroupName"
          :session-id="session?.sessionId || ''"
          :t="t"
          @toggle-targets="toggleBatchTargetsPopover"
          @pick-targets="pickBatchTargets"
          @refresh-targets="refreshBatchTargets"
          @toggle-target="toggleBatchTargetId"
          @toggle-quick="toggleBatchQuickPopover"
          @pick-quick="pickBatchQuick"
          @toggle-group="toggleQuickGroupCollapse"
          @broadcast="broadcastBatchBarState"
          @browse-history="(direction) => direction === 'up' ? browseBatchHistoryUp() : browseBatchHistoryDown()"
          @send="sendBatchCommand"
          @open-save="openBatchBarSave"
          @confirm-save="confirmBatchBarSave"
          @cancel-save="cancelBatchBarSave"
        />
      </div>
      </section>

      <div
        v-if="sftpPaneOpen && !isRdpConnection && (rightPanelView === 'docker' || !localUiMode)"
        class="divider"
        role="separator"
        :aria-orientation="paneLayout.direction === 'column' ? 'horizontal' : 'vertical'"
        :aria-valuenow="Math.round(paneLayout.ratio)"
        aria-valuemin="35"
        aria-valuemax="80"
        :title="dividerResizeHint"
        tabindex="0"
        @pointerdown="startDividerDrag"
        @keydown="onDividerKeydown($event, nudgeSplitDivider)"
      />

      <section v-if="sftpPaneOpen && !isRdpConnection && (rightPanelView === 'docker' || !localUiMode)" ref="sftpPane" class="sftp-pane right-panel" tabindex="-1" :class="{ 'drag-active': dragActive && sftpViewActive }" @pointerdown="focusSftpPaneOnPointerDown" @paste.capture="sftpViewActive && onSftpClipboardPaste($event)" @dragenter.prevent="sftpViewActive && onSftpDragEnter($event)" @dragover.prevent @dragleave="sftpViewActive && onSftpDragLeave($event)" @drop.prevent="sftpViewActive && onDrop($event)">
        <SftpWorkspacePanel
          v-if="rightPanelView === 'sftp'"
          v-model:currentPath="currentPath"
          v-model:pathBarEditing="pathBarEditing"
          v-model:bookmarkSaveOpen="bookmarkSaveOpen"
          v-model:bookmarkLabelDraft="bookmarkLabelDraft"
          v-model:pathHistoryOpen="pathHistoryOpen"
          v-model:sftpSearchDraft="sftpSearchDraft"
          v-model:sftpTypeFilter="sftpTypeFilter"
          :t="t"
          :locale="locale"
          :connected="connected"
          :loading-files="loadingFiles"
          :panel-surface="panelSurface"
          :follow-directory="followDirectory"
          :path-crumbs="pathCrumbs"
          :bookmark-saving="bookmarkSaving"
          :current-path-history="currentPathHistory"
          :sftp-bookmarks="sftpBookmarks"
          :quick-paths="SFTP_QUICK_PATHS"
          :can-write="canWrite"
          :sftp-clipboard="sftpClipboard"
          :paste-busy="pasteBusy"
          :sudo-mode="sudoMode"
          :sftp-show-hidden="sftpShowHidden"
          :sftp-filters-active="sftpFiltersActive"
          :selected-entries="selectedEntries"
          :batch-progress="batchProgress"
          :archive-busy="archiveBusy"
          :batch-delete-submitting="batchDeleteSubmitting"
          :sftp-soft-error="sftpSoftError"
          :file-menu="fileMenu"
          :file-menu-protected="fileMenuProtected"
          :sftp-tree-rows="sftpTreeRows"
          :selected-uri-set="selectedUriSet"
          :selected-path="selectedPath"
          :renaming-path="renamingPath"
          :rename-draft="renameDraft"
          :rename-submitting="renameSubmitting"
          :load-sftp-details="loadSftpDetails"
          :drag-active="dragActive"
          :sftp-drag-drop-target="sftpDragDropTarget"
          :sftp-tree-entries="sftpTreeEntries"
          :sftp-tree-total-rows="sftpTreeTotalRows"
          :loaded-tree-entries="loadedTreeEntries"
          :disk-usage="diskUsage"
          :folder-upload-supported="folderUploadSupported"
          :local-can-save="localCanSave"
          :available-external-editors="availableExternalEditors"
          :editor-config="editorConfig"
          :is-archive-name="isArchiveName"
          :mapped-editor-id-for="mappedEditorIdFor"
          :set-path-bar-input-el="bindSftpPathBarInput"
          :on-tree-pane="bindSftpWorkspaceTreePane"
          @update:fileMenu="fileMenu = $event"
          @update:batchDeleteOpen="batchDeleteOpen = true"
          @update:deleteTarget="deleteTarget = $event"
          @update:renamingPath="renamingPath = $event"
          @update:sftpDragDropTarget="sftpDragDropTarget = $event"
          @update:otpPanelOpen="otpPanelOpen = true"
          @update:importPanelOpen="importPanelOpen = true"
          @go-parent="goParent"
          @load-home="loadHome"
          @load-directory="loadDirectory()"
          @set-directory-tracking="setDirectoryTracking"
          @submit-path-input="submitPathInput"
          @cancel-path-bar-edit="cancelPathBarEdit"
          @begin-path-bar-edit="beginPathBarEdit"
          @go-to-path="goToPath"
          @toggle-bookmark-save="toggleBookmarkSave"
          @confirm-bookmark-save="confirmBookmarkSave"
          @toggle-path-history-menu="togglePathHistoryMenu"
          @remove-bookmark="removeBookmark"
          @paste-clipboard="pasteClipboard"
          @toggle-sudo-mode="toggleSudoMode"
          @clear-sftp-search="clearSftpSearch"
          @toggle-sftp-show-hidden="toggleSftpShowHidden"
          @batch-archive="batchArchive"
          @clear-row-selection="clearRowSelection"
          @batch-download="batchDownload"
          @copy-selected-paths="copySelectedPaths"
          @file-area-context-menu="onFileAreaContextMenu"
          @toggle-sftp-tree-node="toggleSftpTreeNode"
          @select-tree-entry="selectTreeEntry"
          @open-sftp-tree-node="openSftpTreeNode"
          @sftp-tree-drop="onSftpTreeDrop"
          @show-file-menu="showFileMenu"
          @commit-tree-rename="commitTreeRename"
          @file-row-keydown="onFileRowKeydown"
          @folder-menu-action="folderMenuAction"
          @open-entry="openEntry"
          @download-entry="downloadEntry"
          @refresh-sftp-folder="refreshSftpFolder"
          @ensure-external-editors="ensureExternalEditors"
          @open-with-system-entry="openWithSystemEntry"
          @open-with-known-entry="openWithKnownEntry"
          @open-with-custom-entry="openWithCustomEntry"
          @begin-custom-editor-command="beginCustomEditorCommand"
          @begin-symlink-edit="beginSymlinkEdit"
          @begin-rename="beginRename"
          @copy-selected-entries="copySelectedEntries"
          @copy-text-to-clipboard="copyTextToClipboard"
          @begin-chmod="beginChmod"
          @open-attributes="openAttributes"
          @archive-entry="archiveEntry"
          @extract-entry="extractEntry"
        />
        <template v-else-if="rightPanelView === 'metrics'">
          <header class="side-panel-head">
            <h2>
              {{ t("metrics") }}
              <span v-if="metricsDistroBadge" class="distro-badge" :style="{ backgroundColor: metricsDistroBadge.color }" :title="metricsDistroBadge.name">{{ metricsDistroBadge.label }}</span>
              <span v-if="metrics?.hostname" class="metrics-host" :title="metrics.hostname"> · {{ metrics.hostname }}</span>
            </h2>
            <span class="side-panel-head-actions">
              <button class="icon-button compact" :title="t('refresh')" :aria-label="t('refresh')" :disabled="metricsLoading" @click="refreshMetrics"><RefreshCw :class="{ spinning: metricsLoading }" /></button>
              <button class="icon-button compact" :title="t('close')" :aria-label="t('close')" @click="closeRightPanel"><X /></button>
            </span>
          </header>
          <ServerMetricsPanel
            :view="metricsView"
            :t="t"
            :locale="locale"
            @refresh="refreshMetrics"
            @toggle-processes="toggleProcessPanel"
            @refresh-processes="refreshProcessList"
            @kill-process="(row, signal) => killProcessRow(row, signal)"
            @update:sort="setMetricsProcessSort"
          />
        </template>
        <template v-else>
          <header class="side-panel-head">
            <h2>{{ t("docker.toolbarTitle") }}</h2>
            <span class="side-panel-head-actions">
              <button class="icon-button compact" :title="t('close')" :aria-label="t('close')" @click="closeRightPanel"><X /></button>
            </span>
          </header>
          <DockerPanel class="right-panel-docker" :t="t" :session-id="session?.sessionId ?? ''" :connection-id="connectionId" />
        </template>
      </section>
    </section>

    <!-- SFTP 工具栏工具弹窗：OTP 验证码 / 会话导入（组件自带内部弹窗与状态）。 -->
    <Dialog :open="otpPanelOpen" @update:open="(open) => { if (!open) otpPanelOpen = false; }">
      <DialogContent class="modal sftp-tool-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("otpPanel.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="otpPanelOpen = false"><X /></button>
        </header>
        <OtpPanel :t="t" />
      </DialogContent>
    </Dialog>

    <Dialog :open="importPanelOpen" @update:open="(open) => { if (!open) importPanelOpen = false; }">
      <DialogContent class="modal sftp-tool-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("importWizard.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="importPanelOpen = false"><X /></button>
        </header>
        <ImportWizard :t="t" />
      </DialogContent>
    </Dialog>

    <!-- 弹层统一迁移 reka Dialog（Strategy B）：portal/遮罩/焦点陷阱/外点关闭由 reka
         承担；Esc 仍由 onDocumentKeydown 分层链独占（内容上一律 @escape-key-down.prevent），
         外点关闭经 update:open(false) 路由到各弹窗的语义取消函数。 -->
    <Dialog :open="previewOpen" @update:open="(open) => { if (!open) closePreview(); }">
      <DialogContent class="modal preview-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>
            {{ previewTitle }}
            <span v-if="previewDirty" class="preview-dirty"><span class="preview-dirty-dot" />{{ t("editSave.unsaved") }}</span>
            <span v-else-if="previewTruncated" class="preview-truncated-badge">{{ t("previewDialog.truncated", { limit: formatBytes(MAX_INLINE_PREVIEW_BYTES), size: formatBytes(previewSize) }) }}</span>
          </DialogTitle>
          <div v-if="previewEditableAllowed" class="preview-actions">
            <template v-if="!previewEditable">
              <button :title="t('editSave.edit')" :aria-label="t('editSave.edit')" @click="beginPreviewEdit"><Pencil />{{ t("editSave.edit") }}</button>
            </template>
            <template v-else>
              <button :title="t('cancel')" :aria-label="t('cancel')" @click="cancelPreviewEdit">{{ t("cancel") }}</button>
              <button :title="t('editSave.save')" :aria-label="t('editSave.save')" :disabled="previewSaving" @click="savePreview"><Loader2 v-if="previewSaving" class="spinning" /><Save v-else />{{ t("editSave.save") }}</button>
            </template>
          </div>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="closePreview"><X /></button>
        </header>
        <div v-if="previewLoading" class="empty"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="previewMode === 'image'" class="preview-image-stage">
          <img class="preview-image" :class="{ 'preview-image--full': previewImageZoomed }" :src="previewImageUrl" :alt="previewTitle" :title="previewImageZoomed ? t('imagePreview.zoomOut') : t('imagePreview.zoomIn')" @click="previewImageZoomed = !previewImageZoomed" />
        </div>
        <!-- JSON 候选且非编辑态走格式化预览面板（invalid/too-large 时面板内部降级为
             原始视图 + 提示条）；其余文件与编辑态保持既有 TextPreview。 -->
        <JsonPreviewPanel v-else-if="!previewEditable && jsonPreviewState.kind !== 'unavailable'" :state="jsonPreviewState" :text="previewText" :file-name="previewTitle" :locale="locale" :appearance="appearance" />
        <TextPreview v-else :text="previewText" :file-name="previewTitle" :appearance="appearance" :editable="previewEditable" @change="previewDraft = $event" />
      </DialogContent>
    </Dialog>

    <!-- 端口映射管理（-L/-R/-D）：与连接右键弹框复用预设、列表和操作。
         以 connectionId 创建独立映射，关闭终端不停止端口转发。 -->
    <PortForwardDialog
      :locale="locale"
      :open="forwardsOpen"
      :connection-id="connectionId"
      :session-id="null"
      independent
      @update:open="forwardsOpen = $event"
      @error="showError($event, 'terminal')"
    />

    <WorkbenchSftpDialogs
      v-model:operationDialog="operationDialog"
      v-model:operationDraft="operationDraft"
      v-model:symlinkDialog="symlinkDialog"
      v-model:symlinkDraft="symlinkDraft"
      v-model:symlinkTargetDraft="symlinkTargetDraft"
      :symlink-submitting="symlinkSubmitting"
      :watch-modified-queue="watchModifiedQueue"
      :watch-modified-prompt="watchModifiedPrompt"
      :external-edit-busy="externalEditBusy"
      v-model:customEditorOpen="customEditorOpen"
      v-model:customEditorName="customEditorName"
      v-model:customEditorCommand="customEditorCommand"
      v-model:customEditorRemember="customEditorRemember"
      :custom-editor-pattern="customEditorPattern"
      v-model:commandOpen="commandOpen"
      v-model:commandDraft="commandDraft"
      :command-running="commandRunning"
      :command-history="commandHistory"
      v-model:commandUseSudo="commandUseSudo"
      :command-result="commandResult"
      :command-output-text="commandOutputText"
      :command-error="commandError"
      v-model:chmodTarget="chmodTarget"
      v-model:chmodDraft="chmodDraft"
      :chmod-submitting="chmodSubmitting"
      v-model:deleteTarget="deleteTarget"
      :delete-submitting="deleteSubmitting"
      v-model:batchDeleteOpen="batchDeleteOpen"
      :selected-entries="selectedEntries"
      :batch-progress="batchProgress"
      :batch-delete-submitting="batchDeleteSubmitting"
      v-model:downloadPrompt="downloadPrompt"
      :local-can-save="localCanSave"
      :download-conflict-prompt="downloadConflictPrompt"
      :upload-duplicate-prompt="uploadDuplicatePrompt"
      v-model:uploadDuplicateApplyAll="uploadDuplicateApplyAll"
      v-model:newFileDialog="newFileDialog"
      v-model:newFileDraft="newFileDraft"
      :new-file-submitting="newFileSubmitting"
      :attrs-target="attrsTarget"
      :attrs-loading="attrsLoading"
      :attrs-info="attrsInfo"
      v-model:attrsMode="attrsMode"
      :attrs-submitting="attrsSubmitting"
      :attrs-owner-display="attrsOwnerDisplay"
      :can-write="canWrite"
      :perm-roles="PERM_ROLES"
      :perm-columns="PERM_COLUMNS"
      :perm-bit="permBit"
      :batch-progress-percent="batchProgressPercent"
      :format-modified="formatModified"
      :t="t"
      @create-directory="createDirectory"
      @commit-symlink="commitSymlink"
      @dismiss-watch-modified="dismissWatchModified"
      @upload-watched-always="uploadWatchedFileAlways"
      @upload-watched-single="uploadWatchedFileOnce"
      @commit-custom-editor="commitCustomEditor"
      @command-input-keydown="handleCommandInputKeydown"
      @clear-command-history="clearCommandHistory"
      @rerun-history-command="rerunHistoryCommand"
      @cancel-command="cancelCommand"
      @run-command="runCommand"
      @confirm-chmod="confirmChmod"
      @confirm-delete="confirmDelete"
      @confirm-batch-delete="confirmBatchDelete"
      @resolve-download-prompt="resolveDownloadPrompt"
      @resolve-download-conflict="resolveDownloadConflict"
      @resolve-upload-duplicate="resolveUploadDuplicate"
      @open-folder-picker="folderPickerTarget = 'prompt'"
      @create-new-file="createNewFile"
      @close-attributes="closeAttributes"
      @toggle-chmod-perm="toggleChmodPerm"
      @toggle-attrs-perm="toggleAttrsPerm"
      @save-attributes-permissions="saveAttributesPermissions"
    />




    <Dialog :open="!!recordingDeleteTarget" @update:open="(open) => { if (!open) recordingDeleteTarget = null; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="recordingDeleteTarget">
        <header><DialogTitle>{{ t("recordingDelete") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="recordingDeleteTarget = null"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("recordingDeleteConfirm", { host: recordingDeleteTarget.host || recordingDeleteTarget.recordingId }) }}</strong><p class="muted">{{ formatRecordedAt(recordingDeleteTarget.startedAt) }} · {{ formatDuration(recordingDeleteTarget.durationSecs ?? 0) }}</p></div></div>
        <footer><button @click="recordingDeleteTarget = null" :disabled="recordingDeleteSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="recordingDeleteSubmitting" @click="confirmRecordingDelete"><Loader2 v-if="recordingDeleteSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 录制一键清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
    <Dialog :open="recordingClearAllOpen" @update:open="(open) => { if (!open) recordingClearAllOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("recordingsClear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="recordingClearAllOpen = false"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("recordingsClearConfirm", { count: recordings.length }) }}</strong></div></div>
        <footer><button @click="recordingClearAllOpen = false" :disabled="recordingClearAllSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="recordingClearAllSubmitting" @click="confirmRecordingClearAll"><Loader2 v-if="recordingClearAllSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
      </DialogContent>
    </Dialog>

    <!-- 传输历史清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
    <Dialog :open="transferHistoryClearOpen" @update:open="(open) => { if (!open) transferHistoryClearOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("transfersHistory.clear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="transferHistoryClearOpen = false"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("transfersHistory.clearConfirm") }}</strong></div></div>
        <footer><button @click="transferHistoryClearOpen = false">{{ t("cancel") }}</button><button class="danger-button" @click="confirmTransferHistoryClear"><Trash2 />{{ t("delete") }}</button></footer>
      </DialogContent>
    </Dialog>



    <!-- 设置弹窗 + quick sudo 配置档管理弹窗：独立组件（设置域 UI/状态集中处；Esc 链与终端偏好仍留在 App）。 -->
    <SettingsDialog
      ref="settingsDialog"
      v-model:open="settingsOpen"
      v-model:profilesOpen="profilesOpen"
      :session-id="session?.sessionId"
      :connection-id="session?.connectionId"
      :terminal-font-size="terminalFontSize"
      :host-font-size="appearance.terminal.fontSize"
      :host-font-family="hostTerminalFontFamily(appearance)"
      :local-download-dir="localDownloadDir"
      :local-can-save="localCanSave"
      :webgl-enabled="webglEnabled"
      :local-shell="localShellPref"
      @set-local-shell="setLocalShellPref"
      :terminal-behavior="terminalBehavior"
      :pane-direction="paneDirection"
      @update:pane-direction="setPaneDirection"
      :terminal-hotkeys="terminalHotkeys"
      :apple-platform="applePlatform"
      :action-links="actionLinksSettings"
      :gutter="gutterSettings"
      :ctx-search-engines="ctxSearchEnginesText"
      :wallpaper-enabled="wallpaperEnabled"
      :wallpaper-opacity="wallpaperOpacity"
      :wallpaper-session-only="wallpaperSessionOnly"
      :appearance="terminalAppearanceState"
      :custom-themes="terminalAppearance.customThemes"
      :active-theme-id="activeAppearanceThemeId"
      :host-theme="hostTerminalTheme()"
      :host-color-scheme="appearance.colorScheme"
      :download-prefs="downloadPrefsAdapter"
      :transfer-prefs="transferPrefsAdapter"
      :editor-prefs="editorPrefsAdapter"
      :suggestion-prefs="suggestionPrefsAdapter"
      :highlight-rules="highlightRules"
      :highlight-saving="highlightSaving"
      :highlight-display-mode="highlightDisplayMode"
      :quick-commands="quickCommands"
      :quick-saving="quickSaving"
      :quick-importing="quickImportBusy"
      :t="t"
      @notice="showNotice"
      @error="showError"
      @save-highlight-rule="saveHighlightRule"
      @delete-highlight-rule="deleteHighlightRule"
      @toggle-highlight-rule="toggleHighlightRule"
      @update:highlight-display-mode="setHighlightDisplayMode"
      @save-quick-command="saveQuickCommand"
      @delete-quick-command="deleteQuickCommand"
      @import-quick-commands="importQuickCommands"
      @rename-quick-command-group="renameQuickCommandGroupById"
      @dissolve-quick-command-group="dissolveQuickCommandGroupById"
      @browse-download-dir="folderPickerTarget = 'settings'"
      @update:webgl="setWebglEnabled"
      @update-behavior="updateTerminalBehavior"
      @update-hotkeys="updateTerminalHotkeys"
      @update:action-links="updateActionLinksSettings"
      @update:gutter="updateGutterSettings"
      @update:ctx-search-engines="updateCtxSearchEngines"
      @update:ghost-suggest="setGhostEnabled"
      @update:history-panel-enabled="setHistoryPanelEnabled"
      @update:command-history-limit="applyCommandHistoryLimit"
      :ghost-tab-accept="ghostTabAccept"
      @update:ghost-tab-accept="setGhostTabAccept"
      :ai-settings="aiSettingsState"
      :ai-bridge-ready="aiBridgeReady"
      @update-ai-settings="updateAiSettings"
      @reset-ai-consent="resetAiFixConsent"
      :prompt-hints-enabled="promptHintsEnabled"
      @update:prompt-hints-enabled="setPromptHintsEnabled"
      @update:wallpaper-enabled="updateWallpaperEnabled"
      @update:wallpaper-opacity="updateWallpaperOpacity"
      @set-wallpaper-image="setWallpaperImage"
      @clear-wallpaper="clearWallpaperImage"
      @apply-font="(payload) => applyTerminalFontSettings(payload.family, payload.size)"
      @update-appearance="updateTerminalAppearance"
      @apply-theme="applyTerminalAppearanceTheme"
      @save-theme="saveTerminalAppearanceTheme"
      @delete-theme="deleteTerminalAppearanceTheme"
      @add-schemes="addImportedSchemes"
      @remove-scheme="removeImportedScheme"
    />

    <Dialog :open="auditOpen" @update:open="(open) => { if (!open) auditOpen = false; }">
      <DialogContent class="modal audit-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("auditLog.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="auditOpen = false"><X /></button></header>
        <div class="settings-body">
          <div class="audit-toolbar">
            <Select :model-value="auditKindFilter || SELECT_EMPTY_SENTINEL" @update:model-value="(v) => (auditKindFilter = v === SELECT_EMPTY_SENTINEL ? '' : String(v))">
              <SelectTrigger size="xs" class="audit-kind-select" :aria-label="t('auditLog.kindFilter')">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="SELECT_EMPTY_SENTINEL">{{ t("auditLog.kindAll") }}</SelectItem>
                <SelectItem v-for="kind in auditKindOptions(auditEntries)" :key="kind" :value="kind">{{ auditKindLabel(kind, t) }}</SelectItem>
              </SelectContent>
            </Select>
            <div class="audit-actions">
              <button class="icon-button" :title="t('refresh')" :aria-label="t('refresh')" :disabled="auditLoading" @click="loadAuditEntries"><RefreshCw :class="{ spinning: auditLoading }" /></button>
              <button class="icon-button" :title="t('auditLog.clear')" :aria-label="t('auditLog.clear')" :disabled="!auditEntries.length" @click="auditClearOpen = true"><Trash2 /></button>
            </div>
          </div>
          <div v-if="auditLoading && !auditEntries.length" class="empty compact audit-empty"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <div v-else-if="auditLoadFailed" class="empty compact audit-empty">
            <span>{{ t("auditLog.loadFailed") }}</span>
            <button class="link-button" @click="loadAuditEntries">{{ t("refresh") }}</button>
          </div>
          <div v-else-if="!visibleAuditEntries.length" class="audit-empty"><FileText /><span>{{ t("auditLog.empty") }}</span></div>
          <template v-else>
            <ul class="audit-list">
              <li v-for="(entry, index) in visibleAuditEntries" :key="`${entry.ts}-${entry.kind}-${index}`" class="audit-row">
                <span class="audit-time mono">{{ auditTime(entry.ts) }}</span>
                <span class="audit-kind-badge" :class="auditRowKindClass(entry.kind)">{{ auditKindLabel(entry.kind, t) }}</span>
                <span v-if="entry.connection || entry.sessionId" class="audit-connection mono" :title="entry.connection || entry.sessionId">{{ entry.connection || entry.sessionId }}</span>
                <span v-if="entry.command" class="audit-command mono" :title="entry.command">{{ entry.command }}</span>
                <span v-if="entry.output" class="audit-output mono" :title="entry.output">{{ entry.output }}</span>
                <span v-if="entry.gate" class="audit-gate mono">{{ entry.gate }}</span>
                <span v-if="auditOutcomeLabel(entry, t)" class="audit-outcome" :class="{ error: entry.outcome === 'error' || entry.decision === 'denied' || entry.decision === 'timeout', ok: entry.outcome === 'ok' || entry.decision === 'approved' }">{{ auditOutcomeLabel(entry, t) }}</span>
                <span v-if="entry.exitCode != null" class="audit-time">{{ t("auditLog.exitCode", { code: entry.exitCode }) }}</span>
              </li>
            </ul>
            <p v-if="auditTruncated" class="muted audit-truncated">{{ t("auditLog.truncated") }}</p>
          </template>
        </div>
        <footer><button @click="auditOpen = false">{{ t("close") }}</button></footer>
      </DialogContent>
    </Dialog>

    <!-- 审计日志清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
    <Dialog :open="auditClearOpen" @update:open="(open) => { if (!open) auditClearOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("auditLog.clear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="auditClearOpen = false"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("auditLog.clearConfirm") }}</strong></div></div>
        <footer><button @click="auditClearOpen = false" :disabled="auditClearSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="auditClearSubmitting" @click="confirmAuditClear"><Loader2 v-if="auditClearSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
      </DialogContent>
    </Dialog>






    <Dialog :open="!!dropUploadPrompt" @update:open="(open) => { if (!open) resolveDropUpload('cancel'); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="dropUploadPrompt">
        <header>
          <DialogTitle>{{ t("terminalDropPrompt.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="resolveDropUpload('cancel')"><X /></button>
        </header>
        <p class="muted">{{ t("terminalDropPrompt.summary", { count: dropUploadPrompt.files.length }) }}</p>
        <pre class="command-output mono drop-file-list">{{ dropUploadPrompt.files.map((file) => file.name).join("\n") }}</pre>
        <label class="drop-option">
          <input v-model="dropUploadTarget" type="radio" name="drop-upload-target" value="cwd" />
          <span>{{ t("terminalDropPrompt.toCurrent") }}</span>
          <code class="mono">{{ dropCwdTarget }}</code>
        </label>
        <label class="drop-option">
          <input v-model="dropUploadTarget" type="radio" name="drop-upload-target" value="custom" />
          <span>{{ t("terminalDropPrompt.toCustom") }}</span>
        </label>
        <input
          ref="dropUploadPathInputEl"
          v-model="dropUploadPathInput"
          class="drop-path-input mono"
          type="text"
          spellcheck="false"
          :placeholder="t('terminalDropPrompt.pathPlaceholder')"
          :disabled="dropUploadTarget !== 'custom'"
          @keydown.enter.prevent="confirmDropUpload"
        />
        <footer>
          <button @click="resolveDropUpload('cancel')">{{ t("cancel") }}</button>
          <button class="primary-button" :disabled="dropUploadTarget === 'custom' && !normalizeDropTargetDir(dropUploadPathInput)" @click="confirmDropUpload">{{ t("upload") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

    <WorkbenchSessionDialogs
      v-model:spawnSessionDialogOpen="spawnSessionDialogOpen"
      v-model:spawnSessionCommand="spawnSessionCommand"
      :host-key-prompt="hostKeyPrompt"
      v-model:rememberHostKey="rememberHostKey"
      :rdp-cert-prompt="rdpCertPrompt"
      v-model:rdpCertRemember="rdpCertRemember"
      :rdp-cert-remaining="rdpCertRemaining"
      :agent-prompt-head="agentPromptHead"
      v-model:agentPromptCommand="agentPromptCommand"
      v-model:agentPromptRemember="agentPromptRemember"
      :agent-prompt-remaining="agentPromptRemaining"
      v-model:alertTriageOpen="alertTriageOpen"
      v-model:alert-triage-payload="alertTriagePayload"
      :alert-triage-busy="alertTriageBusy"
      :alert-triage-error="alertTriageError"
      :alert-triage-result="alertTriageResult"
      :has-session="!!session"
      :paste-confirm="pasteConfirm"
      :pending-confirm-dialog="pendingConfirmDialog"
      v-model:localOpenConfirmOpen="localOpenConfirmOpen"
      :t="t"
      @confirm-command-session-tab="confirmCommandSessionTab"
      @resolve-host-key="resolveHostKey"
      @resolve-rdp-certificate="resolveRdpCertificate"
      @resolve-agent-prompt="resolveAgentPrompt"
      @run-alert-triage="runAlertTriage"
      @send-suggestion="sendSuggestionToTerminal"
      @copy-suggestions="copySuggestions"
      @resolve-paste-confirm="resolvePasteConfirm"
      @resolve-pending-confirm="resolvePendingConfirmDialog"
      @confirm-local-terminal="confirmLocalTerminal"
    />

    <!-- 目录选择器：DOM 末尾渲染，保证叠在设置弹窗/下载询问弹窗之上。
         不传 initialPath：默认从「此电脑」盘符页开始（macOS/Linux 无盘符概念，
         回退到默认下载目录），与 kimi-code-desktop 的选择器行为对齐。 -->
    <FolderPickerDialog
      v-if="folderPickerTarget"
      :locale="locale"
      @select="onFolderPicked"
      @close="folderPickerTarget = null"
    />

    <!-- Telnet 连接表单（P2-3）：host/port/回退格/回车 + Expect 自动应答。 -->
    <TelnetConnectDialog :locale="locale" :open="telnetDialogOpen" @update:open="(open) => (telnetDialogOpen = open)" @connect="startTelnetSession" />

    <!-- 串口连接表单（P3）：端口发现/波特率/数据位/校验/停止位/退格。 -->
    <SerialConnectDialog :locale="locale" :open="serialDialogOpen" @update:open="(open) => (serialDialogOpen = open)" @connect="startSerialSession" />
    <SerialUploadDialog :locale="locale" :open="serialUploadDialogOpen" :busy="serialUploadBusy" @update:open="(open) => (serialUploadDialogOpen = open)" @start="startSerialUpload" />

    <VncConnectDialog :locale="locale" :open="vncDialogOpen" @update:open="(open) => (vncDialogOpen = open)" @connect="startVncSession" />

    <!-- RDP 连接表单（P3-4）：host/port/NLA 凭据/分辨率/证书策略。 -->
    <RdpConnectDialog :locale="locale" :open="rdpDialogOpen" @update:open="(open) => (rdpDialogOpen = open)" @connect="connectViaRdpDialog" />

    <!-- RDP 确认/VNC 确认/Telnet 确认/串口确认（M32-A 移除）：工具条协议直开
         入口下线后，表单兜底从连接路由失败路径直接弹 ConnectDialog，不再有
         "占用终态先确认替换" 的流程。 -->


    <input ref="uploadInput" class="hidden" type="file" multiple @change="onUploadInput" />
    <!-- webkitdirectory 会自动携带多选语义；目录选择以相对路径（webkitRelativePath）回传。 -->
    <input v-if="folderUploadSupported" ref="folderUploadInput" class="hidden" type="file" multiple webkitdirectory @change="onFolderUploadInput" />
    <input ref="zmodemInput" class="hidden" type="file" multiple @change="onZmodemInput" />
    <input ref="trzszInput" class="hidden" type="file" multiple @change="onTrzszPickInput" @cancel="onTrzszPickCancel" />
  </main>
</template>

<style scoped src="./styles/app.css"></style>

