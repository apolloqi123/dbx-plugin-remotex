<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { ImageAddon } from "@xterm/addon-image";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon, type ISearchOptions } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import {
  Archive,
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Disc,
  Eye,
  EyeOff,
  Film,
  Pause,
  Play,
  ArrowLeftRight,
  ArrowUp,
  ArrowUpDown,
  Bot,
  Braces,
  ClipboardPaste,
  Scan,
  StretchHorizontal,
  Proportions,
  SlidersHorizontal,
  Copy,
  Download,
  Eraser,
  ExternalLink,
  File as FileIcon,
  FileInput,
  FilePlus,
  FileText,
  FileUp,
  KeyRound,
  Plus,
  Folder,
  FolderOpen,
  FolderPlus,
  FolderUp,
  Gauge,
  ImagePlay,
  History,
  Home,
  Info,
  Link2,
  ListChecks,
  Loader2,
  Lock,
  Network,
  PackageOpen,
  Palette,
  PanelRightClose,
  Pencil,
  PlugZap,
  RefreshCw,
  Save,
  Scissors,
  Search,
  Send,
  Settings,
  ShieldCheck,
  Siren,
  Square,
  SquarePlus,
  ListPlus,
  SquareTerminal,
  Star,
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
import { confirmDialog, useConfirmDialogHost } from "../shared/lib/confirmDialog";
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
  sanitizeSearchOptions,
  isApplePlatform,
  TERMINAL_SEARCH_OPTIONS_KEY,
  terminalSearchSeedFromSelection,
  normalizeDropTargetDir,
  resolveDropTargetDir,
  transferBusyPaneClass,
  type TerminalSearchOptions,
} from "../features/terminal/lib/terminalInteraction";
import { createTerminalWriteThrottle, type TerminalWriteThrottle } from "../features/terminal/lib/terminalWriteThrottle";
import { createOutputGate } from "../features/terminal/lib/terminalBackpressure";
import { createTerminalInputQueue, terminalInputChannel, TerminalInputTimeoutError } from "../features/terminal/lib/terminalInputQueue";
import { SERIAL_STREAM_STDIN, isKnownStreamTag, supportsBinaryInput } from "../features/serial/lib/serialTerminalFrames";
import { describeReconnectRestoredNotice, isConnectionInactiveError, isSessionGoneError, shouldReattachTerminal, terminalReconnectDelay, TERMINAL_RECONNECT_DELAYS } from "../features/terminal/lib/terminalReconnect";
import { TERMINAL_TRANSPORT_PROBE_TIMEOUT_MS, useTerminalTransportWatch } from "../features/terminal/composables/useTerminalTransportWatch";
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
import { friendlySftpError } from "../features/sftp/lib/sftpErrors";
import { paneDirectionClass, paneSplitCssVar, resolvePaneDirection, sanitizePaneDirection, terminalFlexBasis, type PaneDirectionPref } from "../shared/lib/paneLayout";
import { isCountdownActive } from "../features/recording/lib/recordingCountdown";
import { type SftpTypeFilter } from "../features/sftp/lib/sftpFileFilters";
import {
  defaultBookmarkLabel,
  listBookmarks,
  loadSftpBookmarksFromStore,
  persistSftpBookmarks,
  removeBookmark as removeBookmarkFromList,
  SFTP_BOOKMARKS_LIMIT,
  SFTP_BOOKMARK_LABEL_MAX_LENGTH,
  upsertBookmark,
  validateBookmarkInput,
  type SftpBookmark,
} from "../features/sftp/lib/sftpBookmarks";
import {
  extractMcpSettingsMirror,
  loadMcpSettingsMirror,
  mcpSettingsMirrorEquals,
  persistMcpSettingsMirror,
} from "../features/settings/lib/settingsModel";
import {
  clampSuggestionMaxChars,
  clampSuggestionMinChars,
  extractPreferencesMirror,
  loadPreferencesMirror,
  persistPreferencesMirror,
  preferencesMirrorEquals,
  sanitizeConflictPolicy,
  sanitizeTransferCompressMode,
  sanitizeTransferCompressThresholdMib,
  type SftpCompressMode,
} from "../shared/lib/preferencesMirror";
import { EnterLineTracker, acceptCommandWithContinuation, canCaptureEnterLine, echoConfirmsLine, extractDesyncedCommand, extractEchoedCommand } from "../features/terminal/lib/terminalEnterCapture";
import { applyLineEditControlChar } from "../features/terminal/lib/terminalLineModel";
import { loadGhostTabAccept, matchesGhostFullAccept, matchesGhostWordAccept, resolveCtrlFRoute, saveGhostTabAccept } from "../features/completion/lib/ghostAcceptKey";
import { isPasswordPromptLine, loadPromptHintsDismissed, loadPromptHintsEnabled, savePromptHintsDismissed, savePromptHintsEnabled, shouldShowPromptHints } from "../features/terminal/lib/terminalPromptHints";
import { readWorkbenchUiMirror, writeWorkbenchUiMirror } from "./lib/workbenchUiMirror";
import { createPersistThrottle } from "./lib/persistThrottle";
import { createPreviewAsyncComponent } from "../shared/lib/asyncPreview";
import { settingsPreferences } from "../features/settings/lib/settingsPreferences";
// 终端交互提示守卫（issue #150 及其反馈扩展）：远端询问输入/要求选择（MFA/
// 验证码/密码待输入、选择菜单、y/n 确认）时，↑/↓ 留给远端交互，不唤起历史
// 面板；键入应答不开建议/ghost 浮层。
import { interactivePromptNearCursor, isInteractivePromptPending } from "../features/terminal/lib/interactivePromptGuard";
import { cursorAbsoluteRow } from "../features/terminal/lib/terminalAnchor";
import { evaluateGhost } from "../features/terminal/lib/terminalGhostSuggest";
import { ARBITER_PRIORITIES, resolveKeyOwners, type ArbiterClaim, type ClaimKey, type KeyOwners, type OverlayId } from "../shared/lib/overlayArbiter";
// AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）：宿主 AI 通道封装 +
// `#` 命令搜索状态机 + 失败修复（输出采集/脱敏）+ 请求构造。插件零密钥，
// provider/模型全在宿主 Settings → AI；能力缺失全链路降级。
import { aiBridgeAvailable, aiCompletionAvailable, aiRecommendationsAvailable, clearAiRecommendations, generateAiText, listAiModels, openAiConversation, pickDefaultAiModel, setAiRecommendations } from "../features/agent/lib/aiBridge";
import { TailCapture } from "../features/agent/lib/aiCapture";
import { parseAiResultText } from "../features/agent/lib/aiResultParse";
import { buildAiAssistRequest, buildAiFixPrompt, buildAiFixRequest, buildAiSearchPrompt, buildAiSearchRequest } from "../features/agent/lib/aiRequests";
import { canActivateAiSearch, classifyAiSearchInput, createAiSearchState, nextAiSearchState, type AiSearchState } from "../features/agent/lib/aiSearchMode";
import { loadAiSettings, saveAiSettings, sanitizeAiSettings, type AiSettings } from "../features/agent/lib/aiSettings";
import { isExpectedNonZeroExit, shouldOfferAiFix } from "../features/agent/lib/aiFix";
import { prepareAiOutputSnapshot, stripAnsiEscapes } from "../shared/lib/outputRedaction";
// 结构化补全（FIG wave-1 最终架构）：唯一结构化补全来源 = fig 引擎
// （vendored amazon-q parser + 全量语料，经冻结接缝 FigCompletionSource 注入
// CompletionController）；legacy 补全目录已退役，无命中即
// pass-through（菜单关、Tab 交 shell），不造假候选。
import type { CompletionItem } from "../features/completion/lib/completion/core/types";
import { sanitizeNameEncoding, type SftpNameEncoding } from "../features/sftp/lib/sftpName";
import { batchUploadPercent, clampTransferConcurrency, clampTransferDownloadLimit, clampTransferMaxActive, runTransfers, sanitizeTransferDuplicatePolicy, type TransferDuplicatePolicy, type TransferTask,
} from "../features/transfers/lib/transferQueue";
import { filterQuickCommands, groupQuickCommands, quickCommandGroupCount, type QuickCommandGroupView, QUICK_COMMANDS_LIMIT } from "../features/quick-commands/lib/quickCommands";
import { flippedOverlayBottom, overlayBelowTop, type SuggestionAnchor } from "../shared/lib/overlayPlacement";
import { choosePromptHintsPlacement, PROMPT_HINTS_FALLBACK_BAR_HEIGHT } from "../features/terminal/lib/terminalPromptHints";
import { registerWatch } from "../features/sftp/lib/watchEdits";
import { batchTargetLabel } from "../features/terminal/lib/batchSend";
import { formatLatency, formatAuthMethodLabel, normalizeConnectionPort, normalizeConnectionText, type KnownAuthMethod } from "../features/connections/lib/connectionInfo";
import { readPluginMode, readPluginShell, resolveWorkbenchId } from "../shared/lib/pluginContext";
import { WORKBENCH_ID } from "../shared/lib/pluginIds";
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
import { purposeKeyLabel, sanitizeTriagePayload, severityClass } from "../features/security/lib/alertTriage";
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
import SftpTreePane from "../features/sftp/components/SftpTreePane.vue";
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
import SplitTerminalPane from "../features/terminal/components/SplitTerminalPane.vue";
import { useTerminalTabs, type TerminalTab } from "../features/terminal/composables/useTerminalTabs";
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
import { type TransferHistoryEntry } from "../features/transfers/lib/transferHistory";
import { useTransferHistory } from "../features/transfers/composables/useTransferHistory";
import { resolveSftpPaneOpen, sanitizeSftpPaneDefaultOpen, resolveDirectoryFollow, sanitizeDirectoryFollowPref, sanitizeSftpAutoConnectPref, shouldLazyLoadSftpListing, sanitizeWorkbenchLayoutPrefs, type SshWorkbenchPaneOrder } from "./lib/workbenchLayout";
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
import { decideFileRowAction } from "../features/sftp/lib/fileRowKeydown";
import { createCrossSurfaceStorageReader, subscribeStorageEvents } from "../shared/lib/appearanceSync";
import { attachWebglRenderer, syncWebglRenderer, type WebglRecoveryOptions, type WebglRendererLike } from "../features/terminal/lib/terminalWebgl";
import {
  activeProfileId,
  loadTerminalAppearance,
  terminalOptionPatch,
  type TerminalAppearanceState,
} from "../features/terminal/lib/terminalAppearance";
import { BUILTIN_TERMINAL_SCHEMES, schemeTone } from "../features/terminal/lib/terminalScheme";
import {
  loadTerminalBehavior,
  persistTerminalBehavior,
  resolveRightClickBehavior,
  sanitizeTerminalBehavior,
  terminalBehaviorOptionPatch,
  transformPasteText,
  type TerminalBehaviorSettings,
} from "../features/terminal/lib/terminalBehavior";
import {
  formatHotkeyDisplay,
  keyComboFromEvent,
  loadTerminalHotkeys,
  matchTerminalHotkey,
  persistTerminalHotkeys,
  sanitizeTerminalHotkeys,
  type TerminalHotkeyActionId,
  type TerminalHotkeyBindings,
} from "../features/terminal/lib/terminalHotkeys";
import { bridgeBinaryBytes } from "../../../shared/frontend/binaryEvent";
import {
  applyFileTreeChildren,
  createFileTreeRoot,
  findFileTreeNode,
  flattenVisibleTreeRows,
  isSftpTreeRequestCurrent,
  markCollapsedTreeNodesStale,
  markFileTreeStale,
  type SftpFileTreeNode,
} from "../features/sftp/lib/sftpFileTree";
import {
  filterSelectableSftpEntries,
  filterSelectableSftpUris,
  isSelectableSftpEntry,
  resolveSftpContextMenuSelection,
} from "../features/sftp/lib/sftpContextMenu";
import { sanitizeSftpEntries, type SftpSanitizedEntry } from "../features/sftp/lib/sftpEntries";
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
import { Switch } from "../shared/ui/switch/index";
import { defaultPatternFor, newCustomEditorId, resolveEditorForFile, sanitizeEditorConfig, type CustomEditor, type EditorConfig, type KnownEditor, type ResolvedEditor } from "../features/sftp/lib/editorRules";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../shared/ui/select/index";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuSub, ContextMenuSubContent, ContextMenuSubTrigger, ContextMenuTrigger } from "../shared/ui/context-menu/index";
import { Popover, PopoverAnchor, PopoverContent } from "../shared/ui/popover/index";
import { Dialog, DialogContent, DialogTitle } from "../shared/ui/dialog/index";
import SettingsDialog from "../features/settings/components/SettingsDialog.vue";
import TerminalContextMenu, {
  buildSearchUrl,
  DEFAULT_CTX_SEARCH_ENGINES_TEXT,
  parseCtxSearchEnginesText,
  type CtxSearchEngine,
} from "../features/terminal/components/TerminalContextMenu.vue";
import PortForwardDialog from "../features/forwarding/components/PortForwardDialog.vue";
import TelnetConnectDialog, { type TelnetConnectOptions } from "../features/telnet/components/TelnetConnectDialog.vue";
import SerialConnectDialog, { type SerialConnectOptions } from "../features/serial/components/SerialConnectDialog.vue";
// 串口文件上传（NyaTerm 对齐 P0-3）：弹窗 + overlay 状态机在 lib/serialUpload。
import SerialUploadDialog from "../features/serial/components/SerialUploadDialog.vue";
import {
  initialSerialUploadState,
  serialUploadActive,
  serialUploadPercent,
  streamSerialUploadFile,
  type SerialUploadProtocol,
  type SerialUploadUiState,
} from "../features/serial/lib/serialUpload";
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
  rdpCertStatusKey,
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

interface SftpStatInfo {
  path: string;
  kind: SftpEntry["kind"];
  size?: number;
  modifiedAt?: number;
  mode?: string;
  owner?: string;
  group?: string;
  ownerName?: string | null;
  ownerUid?: number | null;
  groupName?: string | null;
  groupGid?: number | null;
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
// SFTP 面板默认打开偏好：pluginStore 全局持久化（"false" = 新工作台仅终端）。
const SFTP_PANE_OPEN_KEY = "ssh-sftp-pane-open";
// 跨重启的布局/视图全局偏好（issue #178）：单键 JSON，记录最近一次的
// paneOrder / splitRatio / SFTP 面板开合 / 列显隐与列宽 / 显示隐藏文件 / 排序。
// writeWorkbenchState 写穿（persistState 防抖统一触发），restoreUiState 启动
// 水合为新工作台初始值；workbenchState 会话恢复字段存在时仍优先覆盖。
const WORKBENCH_LAYOUT_KEY = "ssh-workbench-layout";
// 目录跟随全局偏好：pluginStore 持久化（"true" = 新工作台初始即开启；per-tab
// 的 workbenchState.followDirectory 仍优先，恢复的 tab 不被全局值覆盖）。
const FOLLOW_DIRECTORY_KEY = "ssh-follow-directory";
// SFTP 自动连接偏好：pluginStore 持久化（"false" = 连接成功后不自动加载
// SFTP 目录；服务端未开 SFTP 时避免每次连接弹错误横幅，issue #179）。
const SFTP_AUTO_CONNECT_KEY = "ssh-sftp-auto-connect";
const DOWNLOAD_DIR_KEY = "ssh-download-directory";
// 是否默认下载到「下载保存目录」（默认开）；关闭则每次下载打开目录选择窗口。
const DOWNLOAD_USE_DEFAULT_KEY = "ssh-download-use-default-dir";
// 文件已存在时的处理策略：rename（自动重命名，默认）/ ask（询问我）/ overwrite（覆盖）。
const DOWNLOAD_CONFLICT_KEY = "ssh-download-conflict-policy";
// 上传并发（P1-5，1..10，默认 3）与重复目标策略（rename 默认）：sidecar
const SFTP_COMPAT_MODE_KEY = "ssh-sftp-compat-mode";
const SFTP_NAME_ENCODING_KEY = "ssh-sftp-name-encoding";
// 压缩传输（M33，gzip 混合方案）：策略三态（auto=智能综合判断含弱 CPU
// 门槛，默认 / on=始终尝试 / off=关闭）+ 生效阈值 MiB（默认 64，0=不限
// 下限）；sidecar preferences 同步，键名与后端白名单一致。
const TRANSFER_COMPRESS_MODE_KEY = "ssh-transfer-compress-mode";
const TRANSFER_COMPRESS_THRESHOLD_KEY = "ssh-transfer-compress-threshold-mib";
// 命令输入建议（P1-1）：开关 + 查询长度上下限；同一偏好链路持久化。
const SUGGESTIONS_ENABLED_KEY = "ssh-history-suggestions-enabled";
const SUGGESTIONS_MIN_CHARS_KEY = "ssh-history-suggestion-min-chars";
const SUGGESTIONS_MAX_CHARS_KEY = "ssh-history-suggestion-max-chars";
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

// 存储迁移批 3：传输/下载/建议偏好消毒器已下沉 lib/preferencesMirror
// （镜像 schema 同处、可单测），App 统一从那里导入。
// —— AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）——
// 设置权威值 pluginStore（单键 JSON），内存态供门控与设置页回显；bridge 能力
// 位挂载后再探一次（桥 bootstrap 与 UI 挂载同批完成），运行时调用点各做新鲜
// 探测（aiBridgeOk），不受挂载时序影响。
const aiSettingsState = ref<AiSettings>(loadAiSettings());
const aiBridgeReady = ref(false);
const aiSearchState = ref<AiSearchState>(createAiSearchState());
// `#` 直连生成的等待态（2026-10-02 反馈：发起后无任何提示，用户不知道在等
// AI 返回）：pending 期间输入冻结（全部吞键）、提示条切 loading 形态、Esc
// 本地放弃（放弃 = 过期令牌，迟到的结果不回填）；generateText 无取消桥
// （宿主扩展清单项），放弃只能做到「结果不插入」。
const aiSearchPending = ref(false);
const aiSearchPendingQuery = ref("");
let aiSearchToken = 0;
const aiSearchAnchor = ref<SuggestionAnchor | null>(null);
// 修复条状态：command/exitCode/output 已是脱敏后快照（触发时一次成型），
// anchorY 为触发光标行顶（相对 terminal-host），bottom 样式按它落位。
const aiFixBar = ref<{ command: string; exitCode: number; output: string; anchorY: number } | null>(null);
// 执行期输出采集（仅 fix 开启时有意义；采集与解码器随会话复位）。
const aiOutputCapture = new TailCapture();
let aiOutputDecoder = new TextDecoder("utf-8", { fatal: false });
function aiBridgeOk(): boolean {
  return aiBridgeAvailable(window.dbxPlugin);
}
function updateAiSettings(patch: Partial<AiSettings>) {
  aiSettingsState.value = sanitizeAiSettings({ ...aiSettingsState.value, ...patch });
  saveAiSettings(aiSettingsState.value);
}
function resetAiFixConsent() {
  updateAiSettings({ fixConsent: false });
  showNotice(t("aiSettings.resetConsentDone"));
}
/** AI 上下文公共位（多语言 / 多系统 / 多 shell，Warp AI 对齐批）：locale 让
 *  LLM 以终端用户语言回答；shell 语义是命令生成的关键——本地终端取 shell
 *  偏好（basename）与平台口径，SSH 只在提示符采样判出 PowerShell/cmd 时带
 *  （复用补全的采样，不为 AI 另写探测；远端 OS 刻意不发探测命令，AI 从输出
 *  推断——诚实省略优于猜测）。 */
function aiContextExtras() {
  const extras: Record<string, unknown> = { locale: locale.value };
  if (connectionId.value) extras.connectionId = connectionId.value;
  if (terminalCwd.value) extras.cwd = terminalCwd.value;
  if (isLocalMode.value) {
    const shellProgram = localShellPref.value || localShells.value.find((entry) => entry.isDefault)?.program;
    if (shellProgram) extras.shell = shellProgram.split(/[\\/]/).pop();
    const platform = typeof navigator !== "undefined" ? navigator.platform.toLowerCase() : "";
    extras.os = applePlatform ? "macos" : platform.includes("win") ? "windows" : "linux";
  } else {
    const sniffed = sniffTerminalShell();
    if (sniffed) extras.shell = sniffed;
  }
  return extras;
}

// Apple 平台判定（Cmd 为主修饰键）：既有的全选语义与新增的快捷键默认键位都要用，
// 因此在此单点声明，供后面的偏好初始值与终端选项复用。
const applePlatform = isApplePlatform();
// decoration 引擎护栏：全局在档 decoration 上限（超限停止本帧注册）。
const HIGHLIGHT_DECORATION_LIMIT = 400;
// rAF 节流目标：≤30fps（约 33ms 一帧）。
const HIGHLIGHT_SCAN_MIN_INTERVAL_MS = 33;

type TerminalSearchMatchState = "idle" | "match" | "no-match";

const terminalHost = ref<HTMLElement>();
const sftpPane = ref<HTMLElement>();
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
const terminalError = ref("");
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
const sftpError = ref("");
const sftpErrorRetry = ref<(() => void) | null>(null);
const sftpErrorOpen = ref(false);
const sftpErrorKey = ref(0);
const notice = ref("");
const noticeActions = ref<Array<{ label: string; run: () => void }>>([]);
const noticeOpen = ref(false);
const noticeKey = ref(0);
const session = ref<SessionInfo>();
const currentPath = ref("/");
const entries = ref<SftpEntry[]>([]);
const selectedPath = ref("");
const loadingFiles = ref(false);
const hostKeyPrompt = ref<HostKeyPrompt>();
const rememberHostKey = ref(true);
const splitRatio = ref(58);
const paneOrder = ref<SshWorkbenchPaneOrder>("terminal-left");
// 布局方向偏好（issue #187）：auto 沿用 CSS 断点（窄屏上下/宽屏左右）；
// vertical 始终上下（终端在上、SFTP 在下），horizontal 始终左右。持久化走
// WORKBENCH_LAYOUT_KEY 布局偏好档（与 paneOrder 同层，writeWorkbenchState 写穿）。
const paneDirection = ref<PaneDirectionPref>("auto");
// 右侧栏可见性：SFTP / 指标 / 容器三视图互斥，sftpPaneOpen 表示右侧栏
// 整体开合（沿用既有 workbenchState 持久化），rightPanelView 决定当前内容。
// 指标/容器视图不跨重启恢复，避免启动即触发轮询。
const sftpPaneOpen = ref(loadSftpPaneDefaultOpen());
const rightPanelView = ref<RightPanelView>("sftp");
const sftpViewActive = computed(() => rightPanelView.value === "sftp");
// 终端拖放的“面板抢落点”门控：只有 SFTP 树真的可见时才把拖入交给面板；
// 指标/容器视图不应阻止终端接收拖拽。
const sftpViewVisible = computed(() => sftpPaneOpen.value && sftpViewActive.value);
const sftpPaneDefaultOpen = ref(loadSftpPaneDefaultOpen());
// 本会话是否已成功加载过 SFTP 目录：自动连接关闭时，手动打开面板的瞬间
// 据此惰性补载（每会话一次，断开重置）。
const sftpListingLoaded = ref(false);
const sftpAutoConnect = ref(loadSftpAutoConnect());
// sftp/home 探测结果：quick tab 置顶展示（获取失败时该项隐藏）。
const sftpHomePath = ref("");
// 终端行为偏好（对标 Tabby「Terminal」页）：右键语义、剪贴板、响铃、渲染细项，
// 单键 pluginStore 持久化（宿主 host.storage → localStorage 降级）；选中复制
// 是其一个字段（旧键镜像保降级，见 LEGACY_SELECT_COPY_KEY）。
const terminalBehavior = ref<TerminalBehaviorSettings>(loadTerminalBehavior());
/** 选中复制（既有消费点：选区变更钩子与设置页开关）。 */
const termSelectCopy = computed(() => terminalBehavior.value.copyOnSelect);
// 终端快捷键绑定（对标 Tabby「Hotkeys」页）：平台默认 + 用户改写，单键持久化。
const terminalHotkeys = ref<TerminalHotkeyBindings>(loadTerminalHotkeys(applePlatform));
// 沙箱宿主读不到系统剪贴板：右键粘贴的降级链靠这份插件视图内的复制副本。
const terminalCopyCache = createTerminalCopyCache();
const followDirectory = ref(loadDirectoryFollowPref());
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
// 右键菜单由 reka ContextMenu 承载（定位/碰撞/Esc/外点关闭均交给 reka）；
// 这里只保留受控 open 状态与负载数据，坐标由 trigger 从原生事件捕获。
const terminalMenuOpen = ref(false);
// 行右键菜单：selection 为打开菜单瞬间的多选快照（>1 时切换为批量区）。
const fileMenu = ref<{ entry: SftpEntry; selection: string[] }>();
// 右键目标为根 "/" 或当前目录（文件夹）时置位：菜单保留浏览/复制/下载，
// 但改名/删除/压缩/切剪等破坏性动作禁用，防止把正在浏览的目录移走/删掉。
const fileMenuProtected = ref(false);
// 文件列表空白处右键：新建文件夹 / 新建文件 / 刷新（拦截浏览器默认菜单）。
const blankMenu = ref(false);
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
const quickMenuOpen = ref(false);
// Termius Snippets 式面板状态：搜索过滤 / 卡片展开。
const quickSearch = ref("");
const quickExpandedId = ref<string | null>(null);
const filteredQuickCommands = computed(() => filterQuickCommands(quickCommands.value, quickSearch.value));
// 分组视图（issue #201）：派生组 + 折叠态（会话内 UI 态，不持久化）。仅一条
// 未分组时不渲染组头（等同旧扁平视图，避免噪声）；有真实分组才分组展示。
const groupedQuickCommands = computed<QuickCommandGroupView[]>(() => groupQuickCommands(filteredQuickCommands.value));
const quickHasRealGroups = computed(() => quickCommandGroupCount(quickCommands.value) > 0);
const quickCollapsedGroups = ref<string[]>([]);
function toggleQuickGroupCollapse(groupId: string) {
  const index = quickCollapsedGroups.value.indexOf(groupId);
  if (index >= 0) quickCollapsedGroups.value.splice(index, 1);
  else quickCollapsedGroups.value.push(groupId);
}
function quickGroupName(group: QuickCommandGroupView): string {
  return group.id ? group.name : t("quickCommandsUngrouped");
}
const quickCommandGroupSummary = computed(() => t("quickCommandsGroupSummary", { groups: quickCommandGroupCount(quickCommands.value), count: quickCommands.value.length }));
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



function toggleQuickExpand(id: string) {
  quickExpandedId.value = quickExpandedId.value === id ? null : id;
}
// 批量发送命令条（Electerm quick-command bar 风格）：常驻贴在终端底部，回车
// 生效只读门禁（来自 ssh/sessions/list 行的 readOnly：表单 read_only ∥
// 宿主标准 read_only）。后端门禁为权威来源，前端据此禁用写操作。
const connectionReadOnly = ref(false);
const settingsOpen = ref(false);
// reka Select 不接受空串 option value（空串 = 未选中占位）；空值选项用哨兵值双向映射。
const SELECT_EMPTY_SENTINEL = "__empty__";
const auditOpen = ref(false);
const profilesOpen = ref(false);
const chmodTarget = ref<SftpEntry>();
const chmodDraft = ref("");
const chmodSubmitting = ref(false);
const diskUsage = ref<DiskUsage>();
const searchOpen = ref(false);
// 打开搜索面板时的种子状态：选区首行预填 + 持久化的选项开关（见 openTerminalSearch）。
const searchSeedQuery = ref("");
const searchSeedOptions = ref<TerminalSearchOptions>(sanitizeSearchOptions(null));
const searchMatchState = ref<TerminalSearchMatchState>("idle");
const searchResultIndex = ref(0);
const searchResultCount = ref(0);
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
// 背景图开启时强制回退 DOM 渲染器（见 rendererWebglEffective watch）。
const rendererWebglEffective = computed(() => webglEnabled.value && !wallpaperActive.value);
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
const selectedUris = ref<string[]>([]);
// 选中态热点查询走 Set：行 class 绑定与 selectedEntries 每次渲染对全部
// 可见条目各查一次，数组 includes 是 O(条目×选中数)（万行目录 × 多选
// 1000 项 ≈ 每帧千万次比较）；Set 写入仍走 selectedUris 数组。
const selectedUriSet = computed(() => new Set(selectedUris.value));
const lastClickedUri = ref("");
const sftpClipboard = ref<SftpClipboard>();
const pasteBusy = ref(false);
// SFTP 路径书签（全局清单，sftp-bookmarks.json）：路径栏星标收藏 + 路径弹层内跳转/删除。
const sftpBookmarks = ref<SftpBookmark[]>([]);
const bookmarkSaveOpen = ref(false);
const bookmarkSaving = ref(false);
const bookmarkLabelDraft = ref("");
const newFileDialog = ref(false);
const newFileDraft = ref("");
const newFileSubmitting = ref(false);
// OTP 验证码 / 会话导入工具弹窗（SFTP 路径工具栏入口）。
const otpPanelOpen = ref(false);
const importPanelOpen = ref(false);
const attrsTarget = ref<SftpEntry>();
const attrsInfo = ref<SftpStatInfo>();
/** WindTerm 风格属主显示：用户名:组名（uid:gid），名字 = uid 数字时只显示一次。 */
const attrsOwnerDisplay = computed(() => {
  const ai = attrsInfo.value;
  if (!ai) return "–";
  const name = ai.ownerName || ai.owner || "–";
  const gname = ai.groupName || ai.group || "–";
  const label = `${name}:${gname}`;
  if (ai.ownerUid == null || ai.groupGid == null) return label;
  // 如果 ownerName 为 null 且 owner 就是 uid 数字字符串，不重复括号
  const ownerIsUid = ai.ownerName == null && ai.owner === String(ai.ownerUid);
  const groupIsGid = ai.groupName == null && ai.group === String(ai.groupGid);
  if (ownerIsUid && groupIsGid) return label;
  return `${label} (${ai.ownerUid}:${ai.groupGid})`;
});
const attrsLoading = ref(false);
const attrsMode = ref("");
const attrsSubmitting = ref(false);
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
const serialUpload = ref<SerialUploadUiState>(initialSerialUploadState());
const serialUploadDialogOpen = ref(false);
let serialUploadAbortRequested = false;
const serialUploadBusy = computed(() => serialUploadActive(serialUpload.value));
const serialUploadOverlayVisible = computed(() => serialUpload.value.phase !== "idle");
const serialUploadPercentValue = computed(() => serialUploadPercent(serialUpload.value));
const serialUploadStatusLabel = computed(() => {
  const upload = serialUpload.value;
  if (upload.phase === "complete") return t("serial.upload.complete", { name: upload.fileName });
  if (upload.phase === "failed") return t("serial.upload.failed", { reason: upload.reason });
  return t("serial.upload.running", { name: upload.fileName, percent: serialUploadPercentValue.value });
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
// —— 本地终端偏好（sidecar preferences.json 持久化；iframe 沙箱无 localStorage）——
// shell 空串 = 跟随自动探测；integration 缺省开。
const localShellPref = ref("");
const localShellIntegrationPref = ref(true);
// 本地 Shell 启动目录/环境变量（localShellCwd/localShellEnv 偏好）。envPref
// 保留未填完的草稿行作为 UI 模型；持久化与 spawn 载荷都取净化后的副本。
const localShellCwdPref = ref("");
const localShellEnvPref = ref<Array<{ name: string; value: string }>>([]);
// 净化视图：名字非空的行（与 sidecar 的 sanitize_env_pair 边界对齐的前端版）。
const localShellEnvClean = computed(() =>
  localShellEnvPref.value
    .map((pair) => ({ name: pair.name.trim(), value: pair.value }))
    .filter((pair) => pair.name),
);
// shell 选择器菜单：打开 shell 选项视图时拉一次 local/shells/list。
// 会话下拉（新建/复制/命令会话/本地终端/Shell 选项）单入口状态；shell 视图
// 为同弹层内的视图切换（非独立 popover，Esc/外点由 reka 随弹层整体收口）。
const sessionMenuOpen = ref(false);
const sessionMenuShellOpen = ref(false);
// 「选择 Shell 打开」子视图：同弹层视图切换（dock「+」同款列表），语义是
// 一次性 override 打开，与设置子视图（radio = 默认偏好）刻意分开。
const sessionMenuShellPickOpen = ref(false);
// Dock 面板表面的配色快切弹层（底部栏精简工具条）：列出与当前宿主明暗档
// 同调性的方案，单击即写入对应槽位并启用 custom 来源；「跟随宿主」恢复 host。
const panelPaletteOpen = ref(false);
// dock 工具条隐藏偏好（pluginStore 单键，跨会话记忆）：隐藏后由右上角低透明度
// 微钮还原（ChevronDown），避免「藏了找不回」。
const panelToolbarHidden = ref(pluginStore.getItem("ssh-panel-toolbar-hidden") === "1");
function togglePanelToolbar() {
  panelToolbarHidden.value = !panelToolbarHidden.value;
  try {
    if (panelToolbarHidden.value) pluginStore.setItem("ssh-panel-toolbar-hidden", "1");
    else pluginStore.removeItem("ssh-panel-toolbar-hidden");
  } catch {
    // 存储不可用时仅本次会话生效。
  }
}
const panelPaletteSlot = computed<"darkSchemeId" | "lightSchemeId">(() =>
  appearance.value.colorScheme === "light" ? "lightSchemeId" : "darkSchemeId",
);
const panelSchemeChoices = computed(() => {
  const tone = panelPaletteSlot.value === "lightSchemeId" ? "light" : "dark";
  return [...BUILTIN_TERMINAL_SCHEMES, ...terminalAppearance.value.customSchemes].filter((scheme) => schemeTone(scheme) === tone);
});
const panelActiveSchemeId = computed(() => terminalAppearance.value.settings.schemeSource === "custom" ? terminalAppearance.value.settings[panelPaletteSlot.value] : null);
/** 配色快切：null = 跟随宿主（关闭 custom 来源）；非空写入当前明暗档槽位。 */
function pickPanelScheme(id: string | null) {
  panelPaletteOpen.value = false;
  updateTerminalAppearance(id == null
    ? { schemeSource: "host" }
    : { schemeSource: "custom", [panelPaletteSlot.value]: id });
}
const localShells = ref<Array<{ program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean }>>([]);
const localShellsLoading = ref(false);
// 上次本地会话跟踪到的 cwd：重开时继承（VS Code 新终端继承工作区目录惯例）。
const localLastCwd = ref("");
// 本地终端最近命令（633;E 收集， newest-first，cap 20）：右键菜单"重跑"用。
// 注入关闭时无命令边界，本功能静默缺席。
const localRecentCommands = ref<string[]>([]);
// 当前偏好 shell 是否支持注入（ksh/csh/cmd 等裸 shell 灰掉开关）。
const selectedShellInjectable = computed<boolean | undefined>(() => {
  if (!localShellPref.value) return undefined;
  return localShells.value.find((entry) => entry.program === localShellPref.value)?.injectable;
});

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
// 终端分屏：在同一个工作台内并排/上下再开一个独立 PTY，复用已认证 transport。
// row = 左右，column = 上下；null = 未分屏。宿主 openWorkbench 只支持 tab 落点，
// 所以分屏由插件内的 SplitTerminalPane 自行开 session。
type TerminalSplitOrientation = "row" | "column";
const terminalSplitOrientation = ref<TerminalSplitOrientation | null>(null);
const splitTerminalSessionId = ref("");
const canSplitTerminal = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);
const splitTerminalFont = computed(() =>
  resolveTerminalFont(terminalFontOverride.value, {
    fontFamily: hostTerminalFontFamily(appearance.value),
    fontSize: appearance.value.terminal.fontSize,
  }),
);

function openTerminalSplit(orientation: TerminalSplitOrientation) {
  if (!canSplitTerminal.value) return;
  // 已分屏时再次选择只切换排布方向，不重建第二个 PTY。
  terminalSplitOrientation.value = orientation;
}
function closeTerminalSplit() {
  terminalSplitOrientation.value = null;
  splitTerminalSessionId.value = "";
}
function onSplitSessionOpened(sessionId: string) {
  splitTerminalSessionId.value = sessionId;
}

// 工作台内终端页签：主 PTY + 额外 PTY，切页签不切宿主连接 tab。
const {
  tabs: terminalTabs,
  activeId: activeTerminalTabId,
  auxTabs: auxTerminalTabs,
  canAdd: canAddTerminalTab,
  create: createTerminalTabState,
  activate: activateTerminalTabState,
  close: closeTerminalTabState,
  reset: resetTerminalTabs,
  tabLabelKey,
} = useTerminalTabs();
const auxTerminalSessionIds = new Map<string, string>();
const canOpenAuxTerminal = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);

function terminalTabLabel(tab: TerminalTab) {
  return t(tabLabelKey(tab), { index: tab.index });
}
function createTerminalTab() {
  if (!canOpenAuxTerminal.value || !canAddTerminalTab.value) return;
  if (!createTerminalTabState()) return;
  void nextTick(scheduleFit);
}
function activateTerminalTab(id: string) {
  activateTerminalTabState(id);
  void nextTick(scheduleFit);
}
function closeTerminalTab(id: string) {
  if (!closeTerminalTabState(id)) return;
  auxTerminalSessionIds.delete(id);
  void nextTick(scheduleFit);
}
function onAuxTabOpened(tabId: string, sessionId: string) {
  auxTerminalSessionIds.set(tabId, sessionId);
}
function closeAllAuxTerminalTabs() {
  auxTerminalSessionIds.clear();
  resetTerminalTabs();
  void nextTick(scheduleFit);
}
// 本地/RDP 不支持共享 SSH transport 的额外 PTY：切换前收掉内嵌页签。
watch([localUiMode, isRdpConnection], ([local, rdp]) => {
  if (local || rdp) closeAllAuxTerminalTabs();
});
// 主会话断开/切换本地/RDP 时销毁分屏，避免留下不可见的 PTY。
watch(canSplitTerminal, (allowed) => {
  if (!allowed) closeTerminalSplit();
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
// 终端让宽（纯计算在 lib/paneLayout）：右侧栏（SFTP/指标/容器）按用户拖拽比例。
const terminalBasis = computed(() => ({
  flexBasis: terminalFlexBasis({ sftpOpen: sftpPaneOpen.value, splitRatio: splitRatio.value }),
}));
// 纵向分栏（issue #207）：「始终上下」/ 窄屏断点的两栏高度改由容器上的
// --pane-split 变量消费 splitRatio（拖拽/键盘微调唯一数据源），CSS 不再钉死。
// solo（SFTP 收起）不设变量，回落既有 solo 全屏规则。
const paneSplitStyle = computed(() => paneSplitCssVar({ sftpOpen: sftpPaneOpen.value, splitRatio: splitRatio.value }));
const orderedPaneClass = computed(() => [
  paneOrder.value === "sftp-left" ? "panes panes--reversed" : "panes",
  paneDirectionClass(paneDirection.value),
  sftpPaneOpen.value ? "" : "panes--solo",
].filter(Boolean).join(" "));
// 分栏当前生效轴向：auto 模式下跟随视口断点（与 style.css @media 同值），
// 强制模式恒定。事件时机（拖拽/键盘）现读现用，不做响应式缓存——resize 不会
// 触发重算，但下次事件自然取到新值。
function currentPaneDirection(): "column" | "row" {
  return resolvePaneDirection({ pref: paneDirection.value, viewportWidth: typeof window !== "undefined" ? window.innerWidth : undefined });
}
// 把手提示按轴向：纵向拖的是高度、横向拖的是宽度（与 onDividerKeydown 的
// 方向键映射一致）；跟随 currentPaneDirection 的断点/强制模式。
const dividerResizeHint = computed(() =>
  currentPaneDirection() === "column" ? t("dividerResizeHintHeight") : t("dividerResizeHint"),
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

// —— 自定义 tooltip（对齐 DBX 宿主的气泡提示）——
// 全局接管 title 属性：悬停或键盘聚焦时把值挪到 data-tooltip（抑制原生慢速
// 灰框），350ms 后显示主题化气泡；下方空间不足翻到上方。模板无需改动，所有
// 现有和未来的 title 自动生效。
const tooltip = ref<{ text: string; x: number; y: number; above: boolean; arrowOffset: number } | null>(null);
const tooltipBubble = ref<HTMLElement>();
let tooltipEl: HTMLElement | null = null;
let tooltipTimer = 0;

function hideTooltip() {
  window.clearTimeout(tooltipTimer);
  tooltipTimer = 0;
  tooltip.value = null;
  tooltipEl = null;
}

function scheduleTooltip(target: HTMLElement) {
  // title → data-tooltip：Vue 绑定只在值变化时重写 title，下次触发再挪一次。
  const title = target.getAttribute("title");
  if (title !== null) {
    target.setAttribute("data-tooltip", title);
    target.removeAttribute("title");
    // title 挪走后可访问名称会丢（VoiceOver/NVDA 读不出按钮）：回填 aria-label
    // 兜底；已有 aria-label 的元素以它为准，不覆盖。
    if (!target.hasAttribute("aria-label")) target.setAttribute("aria-label", title);
  }
  const text = target.getAttribute("data-tooltip")?.trim();
  if (!text) return;
  tooltipEl = target;
  tooltipTimer = window.setTimeout(() => {
    tooltipTimer = 0;
    if (!target.isConnected) return;
    const rect = target.getBoundingClientRect();
    const above = rect.bottom + 34 > window.innerHeight && rect.top > 34;
    const center = rect.left + rect.width / 2;
    tooltip.value = { text, x: center, y: above ? rect.top - 6 : rect.bottom + 6, above, arrowOffset: 0 };
    // 量出气泡实际宽度后重新钳位，保证整框（而非仅中心点）落在视口内；
    // 小三角按钳位偏差反向偏移，继续对准触发元素。
    void nextTick(() => {
      const bubble = tooltipBubble.value;
      const current = tooltip.value;
      if (!bubble || !current) return;
      const half = bubble.offsetWidth / 2;
      const clamped = Math.max(half + 8, Math.min(current.x, window.innerWidth - half - 8));
      if (clamped !== current.x) {
        tooltip.value = { ...current, x: clamped, arrowOffset: center - clamped };
      }
    });
  }, 350);
}

function onTooltipOver(event: MouseEvent) {
  const target = (event.target as HTMLElement | null)?.closest?.("[title], [data-tooltip]") as HTMLElement | null;
  if (target === tooltipEl) return;
  hideTooltip();
  if (!target) return;
  scheduleTooltip(target);
}

function onTooltipOut(event: MouseEvent) {
  if (!tooltipEl) return;
  const related = event.relatedTarget as HTMLElement | null;
  if (related && tooltipEl.contains(related)) return;
  hideTooltip();
}

// 键盘可达性：focusin/focusout 走同一套气泡（focus 锚定元素本身，
// 不依赖指针位置），blur 时收起。
function onTooltipFocusIn(event: FocusEvent) {
  const target = (event.target as HTMLElement | null)?.closest?.("[title], [data-tooltip]") as HTMLElement | null;
  if (target === tooltipEl) return;
  hideTooltip();
  if (!target) return;
  scheduleTooltip(target);
}

function onTooltipFocusOut(event: FocusEvent) {
  if (!tooltipEl) return;
  const related = event.relatedTarget as HTMLElement | null;
  if (related && tooltipEl.contains(related)) return;
  hideTooltip();
}

// 通知/错误横幅的动作按钮（下载完成的「打开文件/打开目录」、失败的「重试」）。
interface BannerAction {
  label: string;
  run: () => void;
}
// reka Toast 承载展示与计时（悬停暂停/滑动关闭为内建行为）；通知为单实例替换
// 语义——每次 show 递增 key 重挂 ToastRoot，时长重置（3500ms，带动作放宽到 8000ms）。
function showNotice(message: string, actions: BannerAction[] = []) {
  notice.value = message;
  noticeActions.value = actions;
  noticeKey.value += 1;
  noticeOpen.value = true;
}

function onNoticeOpenChange(open: boolean) {
  if (open) return;
  noticeOpen.value = false;
  notice.value = "";
  noticeActions.value = [];
}

function onSftpErrorOpenChange(open: boolean) {
  if (open) return;
  sftpErrorOpen.value = false;
  sftpError.value = "";
  sftpErrorRetry.value = null;
}

function showError(cause: unknown, target: "terminal" | "sftp" = "sftp", retry?: () => void) {
  const message = cause instanceof Error ? cause.message : String(cause);
  // 常见错误（权限不足/文件不存在）翻成友好文案；其余原样透出。
  const display = friendlySftpError(message, (key) => t(key)) ?? message;
  if (target === "terminal") terminalError.value = display;
  else {
    sftpError.value = display;
    sftpErrorRetry.value = retry ?? null;
    // 错误横幅与通知同款自动消失（保留手动关闭），时限放宽到 8s：
    // 错误信息通常更长，需要读完的时间。
    sftpErrorKey.value += 1;
    sftpErrorOpen.value = true;
  }
}

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
        // 本地终端模式没有远端 SFTP 可跟随（loadDirectory 作用于 SSH 会话）。
        if (followDirectory.value && !isLocalMode.value) void followSftpPath(cwd);
      }
      return true;
    }),
  ];
  terminal.attachCustomKeyEventHandler(handleTerminalKey);
  searchAddon.onDidChangeResults(({ resultCount, resultIndex }) => {
    if (!searchOpen.value) return;
    searchResultCount.value = resultCount;
    searchResultIndex.value = resultCount > 0 && resultIndex >= 0 ? resultIndex + 1 : 0;
    searchMatchState.value = resultCount > 0 ? "match" : "no-match";
  });
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
    // `#` 直连生成等待期：输入冻结（全部吞键防误操作/重复发起），Esc 本地
    // 放弃——令牌过期，迟到的结果不回填（generateText 无取消桥）。
    if (aiSearchPending.value) {
      if (data === "\u001b") {
        aiSearchToken += 1;
        aiSearchPending.value = false;
        aiSearchPendingQuery.value = "";
        aiSearchAnchor.value = null;
        showNotice(t("aiSearch.abandonedNotice"));
      }
      terminalDiag.swallowed += 1;
      return;
    }
    // # AI 命令搜索模式（Warp AI Command Search 同位）：激活后 onData 全部
    // 改道本地状态机，字节不进 PTY（退出无需擦远端）；回车发起宿主 AI 会话。
    if (aiSearchState.value.active) {
      const previous = aiSearchState.value;
      const next = nextAiSearchState(aiSearchState.value, classifyAiSearchInput(data));
      aiSearchState.value = next.state;
      if (next.action === "submit") void submitAiSearch(previous.query);
      else if (next.action === "cancel") closeAiSearch();
      else syncAiSearchAnchor();
      terminalDiag.swallowed += 1;
      return;
    }
    // 激活判定：空行键入 #、能力+开关就绪、所有浮层/执行态门干净——# 字节
    // 不发远端（能力缺失时 # 照常放行，shell 注释语义天然兜底）。
    if (
      data === "#" &&
      pendingTerminalInput === "" &&
      aiBridgeOk() &&
      aiSettingsState.value.search &&
      canActivateAiSearch({
        alternateActive: terminal?.buffer.active.type === "alternate",
        commandRunning: commandRunning.value,
        transferBusy: terminalTransferBusy.value,
        suggestionOpen: suggestionOpen.value,
        completionOpen: completionOpen.value,
        historyPanelOpen: historyPanelOpen.value,
        quickSelectOpen: quickSelectOpen.value,
        searchOpen: searchOpen.value,
      })
    ) {
      aiSearchState.value = { active: true, query: "" };
      dismissPromptHints();
      syncAiSearchAnchor();
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
  terminalHost.value.addEventListener("paste", terminalPasteHandler, true);
  terminalWheelHandler = (event) => handleTerminalWheel(event);
  terminalHost.value.addEventListener("wheel", terminalWheelHandler, { passive: false, capture: true });
  terminalMouseDownHandler = (event) => handleTerminalMouseDown(event);
  terminalHost.value.addEventListener("mousedown", terminalMouseDownHandler);
  terminalMouseUpHandler = (event) => handleTerminalMouseUp(event);
  terminalHost.value.addEventListener("mouseup", terminalMouseUpHandler);
  // 终端宿主尺寸变化（窗口缩放/命令条让位）时：fit 重排行数之外，还要
  // 重读建议浮层锚点——光标行/列的像素位置随行高与滚动变化，浮层开着时
  // 停在旧位置会错位或越过新边界（review 第二批：resize 主动重定位）。
  resizeObserver = new ResizeObserver(() => {
    scheduleFit();
    syncSuggestionAnchorsOnSettle();
    syncGhostAnchorOnSettle();
  });
  resizeObserver.observe(terminalHost.value);
  if (webglEnabled.value && !wallpaperActive.value) {
    webglRenderer.value = attachWebglRenderer(terminal, () => new WebglAddon(), webglRecoveryOptions());
  }
  if (highlightEnabled.value) attachHighlightRender();
  if (actionLinksEnabled.value) attachActionLinks();
  if (isGutterActive()) attachGutterListeners();
  scheduleFit();
}

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

function openTerminalSearch() {
  if (!terminal) return;
  terminalMenuOpen.value = false;
  // iTerm2 风格：打开搜索时用当前选区首行预填查询，并带入持久化的选项开关。
  searchSeedQuery.value = terminalSearchSeedFromSelection(terminal.getSelection() || "");
  searchSeedOptions.value = sanitizeSearchOptions(pluginStore.getItem(TERMINAL_SEARCH_OPTIONS_KEY));
  searchOpen.value = true;
}

function closeTerminalSearch() {
  searchOpen.value = false;
  resetSearchResults();
  searchAddon?.clearDecorations();
  terminal?.focus();
}

function clearTerminalSearch() {
  resetSearchResults();
  searchAddon?.clearDecorations();
}

function resetSearchResults() {
  searchMatchState.value = "idle";
  searchResultIndex.value = 0;
  searchResultCount.value = 0;
}

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

// ---------------------------------------------------------------------------
// AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）：# 命令搜索 / 失败
// 修复 / 唤起助手。三条体验都只「发送快照到宿主 AI 面板」——AI 生成内容一律
// 不回写 PTY、不代执行（执行权在人，红线见方案 §0）；插件零密钥，能力缺失
// 全链路可见降级（aiBridgeOk 各调用点新鲜探测）。
// ---------------------------------------------------------------------------

function syncAiSearchAnchor() {
  aiSearchAnchor.value = readTerminalSuggestionAnchor();
}

// AI 搜索条定位：底边贴光标行顶并留 gap——浮在光标行上方，不遮输入行
// （与 TerminalPromptHints 同一语义；此前 top 贴光标行顶，与光标同行）。
// 包含块高度优先实测 terminal-pane（batch/标记条让位时 host 比 pane 矮，
// 与 history 面板同一坐标系），退化用 suggestionViewport；皆不可测时不给
// 内联样式，走 CSS 兜底位（贴终端底部左缘）。
const aiSearchBarEl = ref<HTMLElement | null>(null);
const aiSearchBarStyle = computed(() => {
  const anchor = aiSearchAnchor.value;
  if (!anchor) return undefined;
  const containerHeight = aiSearchBarEl.value?.parentElement?.clientHeight || suggestionViewport.value.height || 0;
  if (!(containerHeight > 0)) return undefined;
  // 光标在第一行/近顶时上方放不下提示条，翻到光标行下方（与
  // TerminalPromptHints 同一放置语义，choosePromptHintsPlacement 共用）。
  const barHeight = aiSearchBarEl.value?.offsetHeight || PROMPT_HINTS_FALLBACK_BAR_HEIGHT;
  if (choosePromptHintsPlacement(anchor.y, containerHeight, barHeight) === "below") {
    // bottom 显式置 auto：CSS 兜底位有 bottom:12px，内联只给 top 时两者同时
    // 生效会把绝对定位元素拉伸成「通天框」（真机截图回归）。
    return { left: `${anchor.x}px`, top: `${overlayBelowTop(anchor.y, anchor.cellHeight ?? 0)}px`, bottom: "auto" };
  }
  return { left: `${anchor.x}px`, bottom: `${flippedOverlayBottom(anchor.y, containerHeight)}px` };
});

function closeAiSearch() {
  aiSearchState.value = createAiSearchState();
  aiSearchAnchor.value = null;
}

/** `#` 发起：优先直连生成（aiCompletion 能力位，桌面运行时）——列模型 →
 *  挑默认 → generateText（prompt 与面板会话共用 buildAiSearchPrompt）→
 *  **生成即回填光标处（Warp 同款默认插入，2026-10-02 体验反馈：回填确认层
 *  拆除）**——回填不等于执行，回车权仍在用户（红线不变）；Why 行走通知提示。
 *  首行为空/异常/直连能力缺失回退既有面板会话路径。 */
async function submitAiSearch(query: string) {
  const trimmed = query.trim();
  closeAiSearch();
  if (!trimmed) return;
  const api = window.dbxPlugin;
  if (aiCompletionAvailable(api)) {
    // 等待态：提示条切 loading（query 冻结显示），Esc 可放弃；令牌防迟到
    // 结果回填。listAiModels/generateText 两段 await 全程在 pending 内。
    const token = ++aiSearchToken;
    aiSearchPendingQuery.value = trimmed;
    aiSearchPending.value = true;
    // closeAiSearch 会清锚点，等待条渲染依赖它——重读一次（同一 tick，光标
    // 未动，坐标不变）。
    aiSearchAnchor.value = readTerminalSuggestionAnchor();
    try {
      const model = pickDefaultAiModel(await listAiModels(api));
      if (!model) throw new Error("no configured ai model");
      const result = (await generateAiText(api, { configId: model.configId, model: model.model, prompt: buildAiSearchPrompt({ query: trimmed }) })).trim();
      if (token !== aiSearchToken) return; // 用户已放弃：结果丢弃不回填
      const parsed = parseAiResultText(result);
      if (!parsed) throw new Error("empty ai response");
      replaceTerminalLineWith(parsed.command, false);
      terminal?.focus();
      showNotice(parsed.why || t("aiSearch.filledNotice"));
      return;
    } catch {
      if (token !== aiSearchToken) return; // 放弃后失败同样静默（不打扰）
      // 直连失败（无模型/宿主确认取消/供应商错误已泛化）→ 回退面板会话。
    } finally {
      if (token === aiSearchToken) {
        aiSearchPending.value = false;
        aiSearchPendingQuery.value = "";
        aiSearchAnchor.value = null;
      }
    }
  }
  const sent = await openAiConversation(api, buildAiSearchRequest({ query: trimmed, context: aiContextExtras() })).catch(() => false);
  showNotice(t(sent ? "aiSearch.sentNotice" : "aiSearch.unavailable"));
}

// —— 推荐位（P2，host.ai.setRecommendations）：修复条在场时向宿主 AI 面板
// 推一张「修复上次失败命令」卡（≤5 张、随工作台存续）；清空点与新命令开始/
// 条消失/会话复位对齐，best-effort 不阻塞主链路。
let fixRecommendationPushed = false;
function pushFixRecommendation() {
  const bar = aiFixBar.value;
  const api = window.dbxPlugin;
  if (!bar || fixRecommendationPushed || !aiRecommendationsAvailable(api)) return;
  fixRecommendationPushed = true;
  void setAiRecommendations(api, {
    context: { ...aiContextExtras(), command: bar.command, exitCode: bar.exitCode, kind: "ai-fix" },
    items: [{ id: "ssh-ai-fix", label: t("aiFix.recommendation"), prompt: `Fix this failed command: ${bar.command} (exit code ${bar.exitCode})`, order: 0 }],
  }).catch(() => undefined);
}
function clearFixRecommendation() {
  if (!fixRecommendationPushed) return;
  fixRecommendationPushed = false;
  void clearAiRecommendations(window.dbxPlugin).catch(() => undefined);
}

/** 本地白名单分诊（ssh/alert/triage 既有 RPC，best-effort）：category +
 *  白名单建议命令作 prompt 提示位，帮 LLM 收窄误诊面；失败静默跳过。 */
async function aiFixTriageHint(output: string): Promise<string | undefined> {
  if (!output.trim()) return undefined;
  try {
    const view = await window.dbxPlugin.invoke<{ category?: string; suggestions?: Array<{ command?: string }> }>("ssh/alert/triage", { payload: output });
    const commands = (view?.suggestions ?? []).map((item) => item?.command).filter((c): c is string => typeof c === "string" && c.length > 0).slice(0, 3);
    if (!view?.category && !commands.length) return undefined;
    return [`category=${view?.category ?? "unknown"}`, ...(commands.length ? [`whitelisted suggestions: ${commands.join("; ")}`] : [])].join(", ");
  } catch {
    return undefined;
  }
}

/** 修复条发起：优先直连生成（aiCompletion 能力位，桌面运行时）——列模型 →
 *  挑默认 → generateText → 弹生成结果确认 → **用户显式确认后回填输入行**
 *  （不回车不执行，红线不变）；直连不可用/失败/取消回退面板会话（ask/agent
 *  由设置 opt-in）。首次发送过快照预览确认（记「不再询问」）。 */
async function aiFixFromBar() {
  const bar = aiFixBar.value;
  if (!bar) return;
  const api = window.dbxPlugin;
  if (!aiBridgeOk()) {
    showNotice(t("aiFix.unavailable"));
    return;
  }
  if (!aiSettingsState.value.fixConsent) {
    const message = `${t("aiFix.confirmBody", { command: bar.command, code: String(bar.exitCode) })}\n\n${bar.output}`;
    const accepted = await confirmDialog(message);
    if (!accepted) return;
    updateAiSettings({ fixConsent: true });
  }
  if (aiCompletionAvailable(api)) {
    try {
      const model = pickDefaultAiModel(await listAiModels(api));
      if (!model) throw new Error("no configured ai model");
      const triageHint = await aiFixTriageHint(bar.output);
      const prompt = buildAiFixPrompt({ command: bar.command, exitCode: bar.exitCode, output: bar.output, triageHint });
      const result = (await generateAiText(api, { configId: model.configId, model: model.model, prompt })).trim();
      // Warp 同款默认插入（2026-10-02 体验反馈，与 `#` 路径一致）：生成即回填
      // 光标处，不弹回填确认——回填不等于执行，回车权仍在用户；Why 行走通知。
      const parsed = parseAiResultText(result);
      if (!parsed) throw new Error("empty ai response");
      aiFixBar.value = null;
      clearFixRecommendation();
      replaceTerminalLineWith(parsed.command, false);
      terminal?.focus();
      showNotice(parsed.why || t("aiFix.filledNotice"));
      return;
    } catch {
      // 直连失败（无模型/宿主确认取消/供应商错误已泛化）→ 回退面板会话。
    }
  }
  const sent = await openAiConversation(api, buildAiFixRequest({ command: bar.command, exitCode: bar.exitCode, output: bar.output, context: aiContextExtras(), agentMode: aiSettingsState.value.agentMode })).catch(() => false);
  if (sent) {
    aiFixBar.value = null;
    clearFixRecommendation();
  }
  showNotice(t(sent ? "aiFix.sentNotice" : "aiFix.unavailable"));
}

/** 无选区时取 buffer 尾部 n 行（可读文本；发送前统一过脱敏管线）。 */
function readTerminalTailLines(lines: number): string {
  const term = terminal;
  if (!term) return "";
  try {
    const buffer = term.buffer.active;
    const start = Math.max(0, buffer.length - lines);
    const rows: string[] = [];
    for (let y = start; y < buffer.length; y += 1) rows.push(buffer.getLine(y)?.translateToString(true) ?? "");
    return rows.join("\n").replace(/\s+$/, "");
  } catch {
    return "";
  }
}

/** ai-assist 热键：选中文本优先，无选区取屏幕尾部 40 行；快照过脱敏管线后
 *  发起 ask 会话。空上下文也可发（通用协助）。 */
async function aiAssistFromTerminal() {
  if (!aiBridgeOk()) {
    showNotice(t("aiAssist.unavailable"));
    return;
  }
  const selection = terminal?.hasSelection() ? (terminal.getSelection() ?? "") : "";
  const raw = selection || readTerminalTailLines(40);
  const sent = await openAiConversation(window.dbxPlugin, buildAiAssistRequest({ query: "", selection: prepareAiOutputSnapshot(raw), context: aiContextExtras(), agentMode: aiSettingsState.value.agentMode })).catch(() => false);
  showNotice(t(sent ? "aiAssist.sentNotice" : "aiAssist.unavailable"));
}

/** 修复条样式：底边贴触发光标行顶（宿主净高/锚点不可测时回退 40px，clamp ≥2px）。 */
const aiFixBarStyle = computed(() => {
  const bar = aiFixBar.value;
  if (!bar) return undefined;
  const hostHeight = suggestionViewport.value.height || 0;
  const bottom = hostHeight > 0 && bar.anchorY >= 0 ? Math.max(2, hostHeight - bar.anchorY + 2) : 40;
  return { bottom: `${bottom}px` };
});

function runTerminalSearch(query: string, options: { caseSensitive: boolean; regex: boolean; wholeWord: boolean }, direction: "next" | "prev") {
  if (!searchAddon || !query) return;
  const searchOptions: ISearchOptions = {
    caseSensitive: options.caseSensitive,
    regex: options.regex,
    wholeWord: options.wholeWord,
    decorations: {
      matchBackground: "#64748b55",
      matchOverviewRuler: "#64748b",
      activeMatchBackground: "#3b82f655",
      activeMatchColorOverviewRuler: "#3b82f6",
    },
  };
  if (direction === "prev") searchAddon.findPrevious(query, searchOptions);
  else searchAddon.findNext(query, searchOptions);
}

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
  closeAiSearch();
  if (promptHintsQuietTimer) {
    clearTimeout(promptHintsQuietTimer);
    promptHintsQuietTimer = undefined;
  }
  promptHintsOutputQuiet.value = true;
  aiSearchToken += 1;
  aiSearchPending.value = false;
  aiSearchPendingQuery.value = "";
  aiFixBar.value = null;
  clearFixRecommendation();
  aiOutputCapture.reset();
  aiOutputDecoder = new TextDecoder("utf-8", { fatal: false });
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
    aiOutputCapture.reset();
    aiFixBar.value = null;
    clearFixRecommendation();
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
    // OSC 633 Cwd doubles as a directory-follow fallback when the backend could
    // not install OSC 7 tracking but the remote shell integration emits 633 frames.
    if (followDirectory.value && directoryTrackingSupported.value === false && updates.cwd) {
      void followSftpPath(updates.cwd);
    }
  }
}

/** AI 修复条触发（Warp Fix-with-AI 同位，在 writeTerminalOutput 的采集之后
 *  调用——快进快出命令 E/C/D 同 chunk，快照必须含同 chunk 正文）：非零退出 +
 *  门干净 + 非预期非零命令（test/grep 豁免，lib/aiFix）→ 以当前快照出条；
 *  anchorY 取当前光标行顶。 */
function maybeOfferAiFix(updates: Osc633StreamUpdates) {
  const exitCode = updates.lastExitCode;
  const command = commandMarker.command;
  if (typeof exitCode !== "number" || exitCode === 0 || !command) return;
  if (
    !shouldOfferAiFix(
      { bridgeAvailable: aiBridgeOk(), fixEnabled: aiSettingsState.value.fix, alternateActive: terminal?.buffer.active.type === "alternate" },
      exitCode,
      command,
    )
  ) {
    return;
  }
  aiFixBar.value = {
    command,
    exitCode,
    output: prepareAiOutputSnapshot(aiOutputCapture.raw()),
    anchorY: readTerminalSuggestionAnchor()?.y ?? -1,
  };
  pushFixRecommendation();
}

function writeTerminalOutput(data: Uint8Array) {
  const now = Date.now();
  for (const path of directoryParser.push(data)) {
    // WT-2：SetUserVar cwd 优先通道在位且刚更新时，提示符 OSC 7 的旧值不回踩
    // （裁决见 shouldOsc7FollowOverrideUserVarCwd；无该通道时行为与从前一致）。
    if (!shouldOsc7FollowOverrideUserVarCwd(userVarCwdEvent, path, now)) continue;
    terminalCwd.value = path;
    if (followDirectory.value) void followSftpPath(path);
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
  if (aiChunkWasActive || commandMarker.active || aiChunkUpdates.lastExitCode != null) aiOutputCapture.append(stripAnsiEscapes(aiOutputDecoder.decode(data, { stream: true })));
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
  if (serialUpload.value.phase !== "idle") {
    serialUploadAbortRequested = true;
    if (sessionId) void window.dbxPlugin.invoke("serial/upload/cancel", { sessionId }).catch(() => undefined);
    serialUpload.value = initialSerialUploadState();
  }
  if (!sessionId) return;
  await window.dbxPlugin.invoke("serial/close", { sessionId }).catch(() => undefined);
  terminal?.focus();
}

// —— 串口文件上传（NyaTerm 对齐 P0-3）——————————————————————————
// 起点：SerialUploadDialog 选好文件/协议；文件字节经 File API 分块
// （≤64KiB）送入 sidecar，协议时序完全由 sidecar 引擎驱动。
async function startSerialUpload(request: { file: File; protocol: SerialUploadProtocol }) {
  const sessionId = serialSession.value?.sessionId;
  if (!sessionId || serialUploadBusy.value) return;
  serialUploadAbortRequested = false;
  serialUpload.value = {
    phase: "running",
    fileName: request.file.name,
    protocol: request.protocol,
    sent: 0,
    total: request.file.size,
    reason: "",
  };
  try {
    await streamSerialUploadFile(request.file, {
      sessionId,
      protocol: request.protocol,
      fileName: request.file.name,
      bridge: window.dbxPlugin,
      readChunk: async (start, end) => new Uint8Array(await request.file.slice(start, end).arrayBuffer()),
      shouldAbort: () => serialUploadAbortRequested,
    });
  } catch (cause) {
    if ((cause as Error)?.message === "tooLarge") {
      showError(t("serial.upload.tooLarge"), "terminal");
    } else {
      showError(cause, "terminal");
    }
    serialUpload.value = { ...initialSerialUploadState(), phase: "failed", reason: String((cause as Error)?.message ?? cause) };
  }
}

// overlay 上的取消：中断本地送数并让 sidecar 发协议取消序列（X/Y: CAN×8，
// Z: ZDLE×5+BS×5），引擎落 Failed 事件后由 progress 归并到 overlay。
function cancelSerialUpload() {
  if (!serialUploadBusy.value) return;
  const sessionId = serialSession.value?.sessionId;
  serialUploadAbortRequested = true;
  if (sessionId) void window.dbxPlugin.invoke("serial/upload/cancel", { sessionId }).catch(() => undefined);
  serialUpload.value = { ...serialUpload.value, phase: "failed", reason: "cancelled" };
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
      ["ssh/session/close", splitTerminalSessionId.value],
      ...[...auxTerminalSessionIds.values()].map((sessionId) => ["ssh/session/close", sessionId] as const),
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

// 会话菜单「选择 Shell 打开」：一次性 override（dock per-shell 语义），不动
// localShell 默认偏好。已在本地模式（运行中或恢复态）= 换 shell 重启；SSH 在
// 连时走既有确认流（override 经 pendingLocalShellOverride 透传）。
function pickLocalShell(program?: string) {
  sessionMenuOpen.value = false;
  sessionMenuShellPickOpen.value = false;
  if (localShellRestored.value || isLocalMode.value) {
    void restartLocalTerminal(program);
    return;
  }
  requestLocalTerminal(program);
}

// 下拉内切到 shell 选择视图：同弹层视图切换，菜单不关，顺带懒加载 shell 列表。
function openSessionShellPick() {
  sessionMenuShellPickOpen.value = true;
  void openLocalShellPrefs();
}

// —— shell 选择器：多平台 shell 发现 + 偏好（VS Code terminal profiles 简化版）——
// 发现清单两个入口（设置菜单 / dock「+」面板菜单）共用；失败静默降级为仅
// 自动探测入口，loading 守卫防重入。
async function loadLocalShells() {
  if (localShellsLoading.value || localShells.value.length) return;
  localShellsLoading.value = true;
  try {
    const result = await window.dbxPlugin.invoke<{ shells: typeof localShells.value }>("local/shells/list", {}, { timeoutMs: 10_000 });
    localShells.value = result.shells || [];
  } catch {
    // 旧 sidecar 无发现方法：菜单退化为仅注入开关（start 仍走自动探测）。
  } finally {
    localShellsLoading.value = false;
  }
}

async function openLocalShellPrefs() {
  await loadLocalShells();
}

// Dock panel "+": opens another dock entry with the selected shell type via the bridge openWorkbench
// (the host owns the surface: inside a panel webview -> a new dock entry; inside a tab -> a new tab).
const localShellSurfaceOpen = ref(false);
// host.listConnections (PR-A4 generic extension point): a read-only, secret-free list of the plugin's own connections,
// for in-panel connection switching; hosts without it degrade to a hidden connection section.
const dockConnections = ref<Array<{ id: string; name: string; providerId: string; connectionType?: string; readOnly?: boolean }>>([]);
async function openLocalShellSurfaceMenu() {
  localShellSurfaceOpen.value = true;
  await loadLocalShells();
  try {
    const listed = await window.dbxPlugin.request<{ connections?: typeof dockConnections.value }>("host.listConnections");
    dockConnections.value = listed?.connections ?? [];
  } catch {
    // Legacy hosts without the listConnections extension point: the connection section stays hidden.
    dockConnections.value = [];
  }
}
function openConnectionSurface(connection: (typeof dockConnections.value)[number]) {
  localShellSurfaceOpen.value = false;
  void window.dbxPlugin.openWorkbench?.(
    WORKBENCH_ID,
    {
      connectionId: connection.id,
      providerId: connection.providerId,
      connectionType: connection.connectionType,
      connection: { id: connection.id, name: connection.name, readOnly: connection.readOnly === true },
    },
    { forceNew: true },
  );
}
function openLocalShellSurface(program?: string) {
  localShellSurfaceOpen.value = false;
  void window.dbxPlugin.openWorkbench?.(
    WORKBENCH_ID,
    { plugin: { mode: "local-terminal", ...(program ? { shell: program } : {}) } },
    { forceNew: true },
  );
}

async function setLocalShellPref(program: string) {
  localShellPref.value = program;
  try {
    await window.dbxPlugin.invoke("local/preferences/set", { localShell: program });
  } catch {
    // 旧 sidecar：会话内存态兜底。
  }
}

async function setLocalShellIntegrationPref(enabled: boolean) {
  localShellIntegrationPref.value = enabled;
  try {
    await window.dbxPlugin.invoke("local/preferences/set", { localShellIntegration: enabled });
  } catch {
    // 旧 sidecar：会话内存态兜底。
  }
}

// 启动目录：空串 = 恢复默认（家目录）；sidecar 在 start 时校验现存目录。
async function setLocalShellCwd(dir: string) {
  localShellCwdPref.value = dir.trim();
  try {
    await window.dbxPlugin.invoke("local/preferences/set", { localShellCwd: localShellCwdPref.value });
  } catch {
    // 旧 sidecar：会话内存态兜底。
  }
}

// 环境变量：持久化取净化副本（未填名的草稿行不落库，保留在 UI 里继续编辑）；
// 名字重复时后者覆盖前者（与 start 载荷的 map 语义一致）。
async function setLocalShellEnv() {
  try {
    await window.dbxPlugin.invoke("local/preferences/set", { localShellEnv: localShellEnvClean.value });
  } catch {
    // 旧 sidecar：会话内存态兜底。
  }
}

function addLocalShellEnvRow() {
  localShellEnvPref.value = [...localShellEnvPref.value, { name: "", value: "" }];
}

function removeLocalShellEnvRow(index: number) {
  localShellEnvPref.value = localShellEnvPref.value.filter((_, i) => i !== index);
  void setLocalShellEnv();
}

// P0.2 self-open: opens a separate connectionless local-terminal tab through the host openWorkbench bridge.
// A4 target contract (spec §4/§11): the plugin payload lives only in context.plugin and the instance identity
// (workbenchId) is generated by the host — when a legacy host omits it, the workbenchId fallback covers it.
// The menu item is hidden on hosts without this bridge.
const canOpenLocalTab = computed(() => Boolean(window.dbxPlugin?.openWorkbench));

async function openLocalTerminalTab() {
  const api = window.dbxPlugin;
  if (!api.openWorkbench) return;
  await api.openWorkbench(
    WORKBENCH_ID,
    { plugin: { mode: "local-terminal" } },
    { forceNew: true },
  );
}

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

// 新建会话（同连接再开一个 tab）：context 复制当前一份并换发新 workbenchId——
// 后端按 workbenchId 开独立 PTY（#41 会话隔离），多个 tab 互不干扰、boot 恢复
// 各回各的会话。克隆的 workbenchState 摘掉 sessionId/terminalSequence，避免新
// tab 尝试回附旧会话。forceNew 让宿主跳过"同 connectionId 复用已有 tab"的查重
// （桥路径不传 connectionId，按钮开的 tab 会互相查重，导致只能多开一个）；
// 旧宿主忽略第三参，退化为原查重行为。connectionId 显式写入 context：宿主的
// 重推凭据（reinit re-push）与 hostContext 合并都键在 context.connectionId 上，
// 不能依赖宿主已把它合进 context（旧宿主没有那层合并）。
function sessionTabContext(reuseAuthenticatedTransport: boolean, spawnCommand?: string): Record<string, unknown> {
  const context: Record<string, unknown> = {
    ...hostContext.value,
    connectionId: connectionId.value,
    workbenchId: randomUUID(),
    reuseAuthenticatedTransport,
    reuseAuthenticatedSessionId: reuseAuthenticatedTransport ? session.value?.sessionId : undefined,
    // WT-4（WezTerm spawn 对标）：命令会话的一次性打开参数。非宿主保留
    // 字段，随 context 原样透传给新工作台，由 sessionTransportReuse 状态
    // 消费；契约见 docs/architecture/PROTOCOL.zh-CN.md「同 transport 命令会话」。
    spawnCommand: spawnCommand?.trim() || undefined,
  };
  const persisted = context.workbenchState;
  if (persisted && typeof persisted === "object") {
    const { sessionId: _sessionId, terminalSequence: _terminalSequence, ...rest } = persisted as Record<string, unknown>;
    context.workbenchState = rest;
  }
  return context;
}

function openNewSessionTab() {
  const api = window.dbxPlugin;
  if (!api.openWorkbench || !connectionId.value) return;
  // target: "tab"：面板表面（dock iframe）调用时显式指定主工作台 tab 落点；
  // tab 表面调用本就落 tab，旧宿主忽略该选项回落面板条目（可选降级）。
  void api.openWorkbench(WORKBENCH_ID, sessionTabContext(false), { forceNew: true, target: "tab" });
}

// 复制会话：新 tab 仍拥有独立 PTY、回放缓冲和 workbenchId，但后端在当前
// 已认证 SSH transport 上另开 channel，因此堡垒机不会再次发起 MFA。这里不
// 复制或缓存 OTP；若原 transport 已失效，后端会要求走“新建会话”重新连接。
function openCopiedSessionTab() {
  const api = window.dbxPlugin;
  if (!api.openWorkbench || !connectionId.value || !connected.value) return;
  void api.openWorkbench(WORKBENCH_ID, sessionTabContext(true), { forceNew: true, target: "tab" });
}

// 命令会话（WT-4，WezTerm spawn 对标）：复制会话的同族入口，channel 启动
// 动作从 shell 换成执行指定命令（独立 PTY，后端 shell 单引号转义）。弹窗
// 收命令后 forceNew 开新 tab；共享 transport 引用、MaxSessions 失败可见等
// 语义全部复用复制会话链路（PROTOCOL「同 transport 命令会话」节）。
const spawnSessionDialogOpen = ref(false);
const spawnSessionCommand = ref("");

function openCommandSessionTab() {
  if (!window.dbxPlugin?.openWorkbench || !connectionId.value || !connected.value) return;
  spawnSessionCommand.value = "";
  spawnSessionDialogOpen.value = true;
}

function confirmCommandSessionTab() {
  const command = spawnSessionCommand.value.trim();
  const api = window.dbxPlugin;
  if (!api?.openWorkbench || !connectionId.value || !connected.value || !command) return;
  spawnSessionDialogOpen.value = false;
  void api.openWorkbench(WORKBENCH_ID, sessionTabContext(true, command), { forceNew: true, target: "tab" });
}


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
  resumeInput,
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
const dockerPanelOpen = computed(() => sftpPaneOpen.value && rightPanelView.value === "docker");

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


// —— SFTP 单栏文件树：懒加载 / 祖先路径揭示 / 目录跟随 / 显示隐藏切换 ——
// 当前目录数据仍以 entries/currentPath 为准（loadDirectory 是唯一出口），树节点
// 在列表落地后由 watch([currentPath, entries]) 同步；祖先路径用独立 list 调用，
// 不改变当前目录，避免打开深层路径时闪烁。
let sftpTreeSessionId = "";
let sftpTreeInitInFlight = false;
// 树 epoch：重建（显隐切换）与会话重置都会递增；reveal/刷新在每次 await 后
// 校验 epoch，丢弃落在旧根上的迟到结果，避免把子项挂到已被替换的树上。
let sftpTreeRebuildSeq = 0;

async function listSftpDirectory(path: string): Promise<SftpSanitizedEntry[]> {
  const sessionId = session.value?.sessionId;
  if (!sessionId) return [];
  const result = await window.dbxPlugin.invoke<{ entries: unknown }>(sudoMode.value ? "sudo/listDir" : "sftp/list", {
    sessionId,
    path: normalizeRemotePath(path),
    includeOwner: false,
  });
  return sanitizeSftpEntries(result.entries);
}

// 树行详情懒加载：树列表关闭了 includeOwner，悬浮详情图标时补一次 stat；
// 结果按 uri+modifiedAt 缓存（最多 200 条），刷新后条目变化会自然重新取。
const SFTP_DETAIL_CACHE_LIMIT = 200;
const sftpDetailCache = new Map<string, SftpDetailInfo>();
// 换会话/重连后同一路径的 stat 可能来自另一台主机，缓存不能跨会话复用。
watch(() => session.value?.sessionId, () => sftpDetailCache.clear());
async function loadSftpDetails(entry: SftpSanitizedEntry): Promise<SftpDetailInfo | null> {
  const sessionId = session.value?.sessionId;
  if (!sessionId) return null;
  const fallbackPath = pathFromUri(entry.uri) || "/";
  // chmod 通常不改 mtime：权限串也纳入缓存键。
  const cacheKey = `${entry.uri}|${entry.modifiedAt ?? ""}|${entry.permissions ?? ""}`;
  const cached = sftpDetailCache.get(cacheKey);
  if (cached) return cached;
  try {
    const stat = await window.dbxPlugin.invoke<Partial<SftpDetailInfo> & { mode?: string }>(sudoMode.value ? "sudo/stat" : "sftp/stat", {
      sessionId,
      path: fallbackPath,
    });
    const detail = mergeSftpDetail(entry, stat, fallbackPath);
    sftpDetailCache.set(cacheKey, detail);
    if (sftpDetailCache.size > SFTP_DETAIL_CACHE_LIMIT) {
      const oldest = sftpDetailCache.keys().next().value;
      if (oldest) sftpDetailCache.delete(oldest);
    }
    return detail;
  } catch {
    // 权限/旧 sidecar：详情卡回落到 entry 自带字段，不弹全局错误。
    return null;
  }
}

/** 列表是否确实属于 path：路径栏提交会先改 currentPath 再发请求，期间
 *  entries 仍是旧目录，不能把旧子项挂到新节点上。 */
function listingBelongsToPath(listing: readonly { uri: string }[], path: string): boolean {
  if (!listing.length) return true;
  return listing.every((entry) => parentPath(pathFromUri(entry.uri)) === path);
}

/**
 * 展开并揭示绝对路径：根 → 逐段祖先 → 目标。根/祖先未加载时即时列目录；
 * 目标正好是当前目录且列表已落地时直接复用 entries，避免重复请求。
 * tree epoch 在每次 await 后复查（重建/会话重置会换掉根），过期的揭示直接
 * 放弃，不向旧根写子节点。quiet 用于目录跟随等后台路径：失败不弹全局横幅。
 */
async function revealSftpPath(path: string, options: { quiet?: boolean } = {}): Promise<void> {
  if (!session.value) return;
  const epoch = sftpTreeRebuildSeq;
  const target = normalizeRemotePath(path);
  try {
    let root = sftpTreeRoot.value;
    if (!root.loaded) {
      const rootEntries = currentPath.value === "/" && sftpListingLoaded.value && listingBelongsToPath(entries.value, "/")
        ? entries.value
        : await listSftpDirectory("/");
      if (epoch !== sftpTreeRebuildSeq || !session.value) return;
      root = sftpTreeRoot.value;
      applyFileTreeChildren(root, "/", rootEntries, sftpShowHidden.value);
    }
    root = sftpTreeRoot.value;
    root.expanded = true;
    const segments = target === "/" ? [] : target.split("/").filter(Boolean);
    let cursor = "/";
    for (const segment of segments) {
      const nextPath = joinRemote(cursor, segment);
      let node = findFileTreeNode(root, nextPath);
      if (!node) return; // 隐藏项/已被删除：中止在最近的可用祖先。
      if (node.kind !== "directory") return;
      node.expanded = true;
      if (!node.loaded) {
        const listing = currentPath.value === nextPath && sftpListingLoaded.value && listingBelongsToPath(entries.value, nextPath)
          ? entries.value
          : await listSftpDirectory(nextPath);
        if (epoch !== sftpTreeRebuildSeq || !session.value) return;
        root = sftpTreeRoot.value;
        node = findFileTreeNode(root, nextPath);
        if (!node || node.kind !== "directory") return;
        node = applyFileTreeChildren(root, nextPath, listing, sftpShowHidden.value);
        if (!node) return;
        node.expanded = true;
        node.loaded = true;
      }
      cursor = nextPath;
    }
    if (epoch !== sftpTreeRebuildSeq) return;
    const targetNode = findFileTreeNode(sftpTreeRoot.value, target);
    if (targetNode?.kind === "directory") targetNode.expanded = true;
  } catch (cause) {
    if (epoch !== sftpTreeRebuildSeq) return;
    if (!options.quiet) showError(cause);
  }
}

/** 重列所有已加载且展开的树节点（不改变 currentPath/选中）；单个分支失败
 *  保留旧子项，当前目录的错误由 loadDirectory 统一反馈。
 *  已加载但折叠的分支不额外请求（代价高且用户看不到），先标记 loaded=false，
 *  下一次展开由 toggleSftpTreeNode 重新 list——否则在折叠目录里保存/改名后，
 *  重新展开会直接复用旧 children 显示过期内容。 */
async function refreshExpandedSftpTree(): Promise<void> {
  if (!session.value) return;
  const epoch = sftpTreeRebuildSeq;
  markCollapsedTreeNodesStale(sftpTreeRoot.value);
  const paths: string[] = [];
  collectExpandedTreePaths(sftpTreeRoot.value, paths);
  for (const path of paths) {
    if (epoch !== sftpTreeRebuildSeq || !session.value) return;
    const node = findFileTreeNode(sftpTreeRoot.value, path);
    if (!node || node.kind !== "directory") continue;
    node.loading = true;
    try {
      const listing = await listSftpDirectory(path);
      if (epoch !== sftpTreeRebuildSeq) return;
      applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
    } catch {
      // 分支刷新失败保留旧子树，避免一次抖动把已展开目录清空。
    } finally {
      node.loading = false;
    }
  }
}

/** 变更后的统一刷新：当前目录列表（失败逐级回退父目录，覆盖当前目录被删/
 *  祖先被改名的场景）+ 所有已展开分支 + 把当前路径重新揭示。
 *  loadDirectory 会清空选中；这里先快照，刷新完成后只恢复「仍挂在当前树上」
 *  的 URI（编辑器保存保住选中文件，删除/改名的 URI 自然掉落）。会话已切换
 *  时绝不恢复，避免把旧会话选择带到新会话。 */
async function refreshSftpAfterMutation(): Promise<void> {
  if (!session.value) return;
  const sessionId = session.value.sessionId;
  const pathAtStart = currentPath.value;
  const previousSelection = [...selectedUris.value];
  const previousSelectedPath = selectedPath.value;
  const previousLastClicked = lastClickedUri.value;
  let probe = currentPath.value;
  for (;;) {
    const errorKey = sftpErrorKey.value;
    await loadDirectory(probe);
    if (!session.value) return;
    if (sftpErrorKey.value === errorKey) break;
    const parent = parentPath(probe);
    if (parent === probe) break;
    probe = parent;
  }
  await refreshExpandedSftpTree();
  await revealSftpPath(currentPath.value, { quiet: true });
  // 会话切走、或刷新期间用户已导航到别的目录：不把旧目录的选中带回来。
  if (!session.value || session.value.sessionId !== sessionId || currentPath.value !== pathAtStart || !previousSelection.length) return;
  // 等 entries watcher 把新列表挂到树上后再取交集，避免拿到刷新前的节点。
  await nextTick();
  const loadedUris = new Set(loadedTreeEntries.value.map((entry) => entry.uri));
  const kept = previousSelection.filter((uri) => loadedUris.has(uri));
  if (!kept.length) return;
  selectedUris.value = kept;
  if (kept.includes(previousSelectedPath)) selectedPath.value = previousSelectedPath;
  lastClickedUri.value = kept.includes(previousLastClicked) ? previousLastClicked : kept[0];
}

/** 收集当前展开且已加载的目录路径（重建树时保持展开分支）。 */
function collectExpandedTreePaths(node: SftpFileTreeNode, paths: string[]): void {
  if (node.kind !== "directory") return;
  if (node.expanded && node.loaded) paths.push(node.path);
  for (const child of node.children) collectExpandedTreePaths(child, paths);
}

/**
 * 显示/隐藏切换后重建树：旧树标记 stale，按展开路径从新根重放。
 * 重建而不是原地 apply，避免父级重拉时把已加载的深层子树（旧 loaded=false）
 * 误收起；当前目录末端仍用最新 entries，保证隐藏项立即出现/消失。
 */
async function rebuildSftpTree(): Promise<void> {
  if (!session.value) return;
  const seq = ++sftpTreeRebuildSeq;
  const paths: string[] = [];
  collectExpandedTreePaths(sftpTreeRoot.value, paths);
  const expanded = new Set<string>([currentPath.value, ...paths]);
  markFileTreeStale(sftpTreeRoot.value);
  sftpTreeRoot.value = createFileTreeRoot("/");
  const ordered = [...expanded]
    .filter((path) => path !== "/")
    .sort((left, right) => left.split("/").filter(Boolean).length - right.split("/").filter(Boolean).length);
  for (const path of ordered) {
    if (seq !== sftpTreeRebuildSeq) return;
    await revealSftpPath(path);
  }
  if (seq === sftpTreeRebuildSeq) await revealSftpPath(currentPath.value);
}

function resetSftpTree() {
  sftpTreeRebuildSeq += 1;
  sftpTreeRoot.value = createFileTreeRoot("/");
  sftpTreeSessionId = "";
}

/** 初始树目标：跟随开启且有终端 cwd → cwd；否则当前用户 home；home 未知 → "/"。 */
async function resolveSftpTreeTarget(): Promise<string> {
  if (followDirectory.value && terminalCwd.value) return normalizeRemotePath(terminalCwd.value);
  if (!sftpHomePath.value) await refreshSftpHomePath();
  return sftpHomePath.value ? normalizeRemotePath(sftpHomePath.value) : "/";
}

/** 每会话只初始化一次（/ 与当前目录一致时也确保首个列表落地）。 */
async function initializeSftpTree(): Promise<void> {
  const sessionId = session.value?.sessionId;
  if (!sessionId || !sftpPaneOpen.value || isRdpConnection.value || localUiMode.value) return;
  if (sftpTreeInitInFlight) return;
  // 会话 id 变了但 closeSession 未走的恢复路径：旧树整体作废。
  if (sftpTreeSessionId && sftpTreeSessionId !== sessionId) resetSftpTree();
  if (sftpTreeSessionId === sessionId && sftpTreeRoot.value.loaded) {
    // 面板收起期间目录跟随只同步了 currentPath：重新打开时把树揭示到当前目录。
    await revealSftpPath(currentPath.value);
    return;
  }
  sftpTreeInitInFlight = true;
  try {
    const before = currentPath.value;
    const target = await resolveSftpTreeTarget();
    if (!session.value || session.value.sessionId !== sessionId) return;
    if (currentPath.value !== before) {
      // home 探测期间用户已手动导航：以用户选择为准，不用初始目标回拉。
      await revealSftpPath(currentPath.value);
      sftpTreeSessionId = sessionId;
      return;
    }
    if (target !== currentPath.value || !sftpListingLoaded.value) await loadDirectory(target);
    await revealSftpPath(target);
    sftpTreeSessionId = sessionId;
  } finally {
    sftpTreeInitInFlight = false;
  }
}

/** 目录跟随：面板收起时保持旧语义（只同步当前目录数据），面板打开时再揭示树。
 *  在途 guard 合并 OSC 7 / 633 / SetUserVar 同 tick 触发的重复跟进。 */
let followSftpBusy = false;
let pendingFollowPath = "";
async function followSftpPath(path: string): Promise<void> {
  if (!followDirectory.value || !session.value || isLocalMode.value) return;
  const target = normalizeRemotePath(path);
  if (!sftpPaneOpen.value) {
    if (currentPath.value !== target) await loadDirectory(target, true);
    return;
  }
  if (followSftpBusy) {
    if (target !== pendingFollowPath) pendingFollowPath = target;
    return;
  }
  followSftpBusy = true;
  try {
    if (currentPath.value !== target) await loadDirectory(target, true);
    // 跟随后台揭示失败已经有 #209 软提示（loadDirectory 静默档），不再叠全局横幅。
    await revealSftpPath(target, { quiet: true });
  } finally {
    followSftpBusy = false;
    const next = pendingFollowPath;
    pendingFollowPath = "";
    if (next && next !== target && followDirectory.value) void followSftpPath(next);
  }
}

/** 单击树行文件夹只做展开/收起：懒加载子项走 listSftpDirectory +
 *  applyFileTreeChildren，绝不修改 currentPath/entries/选中（进入目录只由
 *  双击/Enter 触发，见 openSftpTreeNode）。失败回退展开态。
 *  与 reveal 共用 epoch/session 守卫：列表返回时若树已重建、会话已切换或
 *  节点已被父级刷新替换，迟到结果直接丢弃，不向新树写旧子项。 */
async function toggleSftpTreeNode(node: SftpFileTreeNode): Promise<void> {
  if (node.kind !== "directory" || node.loading) return;
  if (node.expanded) {
    node.expanded = false;
    return;
  }
  node.expanded = true;
  if (node.loaded) return;
  node.loading = true;
  const epoch = sftpTreeRebuildSeq;
  const sessionId = session.value?.sessionId;
  const requestIsCurrent = () => isSftpTreeRequestCurrent(
    { epoch, sessionId, node },
    { epoch: sftpTreeRebuildSeq, sessionId: session.value?.sessionId, root: sftpTreeRoot.value },
  );
  try {
    const listing = await listSftpDirectory(node.path);
    if (!requestIsCurrent()) {
      node.expanded = false;
      return;
    }
    const applied = applyFileTreeChildren(sftpTreeRoot.value, node.path, listing, sftpShowHidden.value);
    if (!applied) node.expanded = false;
  } catch (cause) {
    node.expanded = false;
    if (requestIsCurrent()) showError(cause);
  } finally {
    node.loading = false;
  }
}

/** 双击/Enter 文件夹：进入该目录（loadDirectory 同时更新 currentPath/entries
 *  并清空选择，子树由 entries watcher 落地）。 */
async function openSftpTreeNode(node: SftpFileTreeNode): Promise<void> {
  if (node.kind === "directory") {
    node.expanded = true;
    await loadDirectory(node.path);
    return;
  }
  // 树内文件打开走终端上方的编辑器页签；右键「预览」仍走 openEntry。
  await openEditorTab(node.entry);
}

function commitTreeRename(node: SftpFileTreeNode, value: string): void {
  renameDraft.value = value;
  void commitRename(node.entry);
}

// loadDirectory 落地后同步对应树节点；applyFileTreeChildren 只写 node，不改
// currentPath/entries，因此不会触发本 watcher 自循环。只监听 entries：路径栏
// 提交会先改 currentPath，若同时监听会在响应回来前拿旧列表挂到新节点。
watch(entries, (listing) => {
  if (!session.value) return;
  const path = currentPath.value;
  if (!listingBelongsToPath(listing, path)) return;
  const node = findFileTreeNode(sftpTreeRoot.value, path);
  if (!node) {
    // 路径栏/书签/历史跳到尚未展开的深层目录：按当前列表补建祖先并揭示。
    // 树尚未初始化时交给 initializeSftpTree，避免首连重复列根目录。
    if (sftpTreeRoot.value.loaded) void revealSftpPath(path);
    return;
  }
  if (node.kind !== "directory") return;
  applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
  node.expanded = true;
  node.loaded = true;
  node.loading = false;
});

// 隐藏项显隐：重建展开分支（本地 rebuilt 后旧 loaded 全部失效，避免假展开）。
watch(sftpShowHidden, () => {
  void rebuildSftpTree();
});

// 目录跟随：终端 cwd 变化或开关打开时揭示对应路径（面板收起由调用点兜底）。
watch([terminalCwd, followDirectory], ([cwd, follow]) => {
  if (!follow || !cwd || !connected.value || !sftpPaneOpen.value) return;
  void followSftpPath(cwd);
});

/** 空白处右键：新建文件夹 / 新建文件 / 上传文件 / 刷新（与行菜单共用同一 ContextMenu 根；
 *  行右键已由 showFileMenu 先行接管，这里按事件目标兜底空白区）。 */
function onFileAreaContextMenu(event: MouseEvent) {
  if ((event.target as HTMLElement).closest(".sftp-tree-row")) return;
  terminalMenuOpen.value = false;
  fileMenu.value = undefined;
  blankMenu.value = true;
}

function blankMenuAction(action: "mkdir" | "newFile" | "upload" | "uploadFolder" | "refresh" | "symlink") {
  const menu = blankMenu.value;
  blankMenu.value = false;
  if (!menu) return;
  if (action === "refresh") {
    void loadDirectory();
    return;
  }
  if (action === "mkdir") {
    operationDraft.value = "";
    operationDialog.value = "mkdir";
  } else if (action === "symlink") {
    beginSymlinkCreate();
  } else if (action === "upload") {
    void chooseUpload();
  } else if (action === "uploadFolder") {
    chooseFolderUpload();
  } else {
    openNewFileDialog();
  }
}

/** 文件夹右键刷新：当前目录走整页 loadDirectory；树内其他目录重列子项并
 *  保持原展开态（折叠目录刷新缓存，展开目录原地更新，不强制展开）。 */
async function refreshSftpFolder(entry: SftpEntry) {
  fileMenu.value = undefined;
  if (!session.value) return;
  const path = pathFromUri(entry.uri);
  if (path === currentPath.value) {
    await loadDirectory();
    return;
  }
  const node = findFileTreeNode(sftpTreeRoot.value, path);
  if (!node || node.kind !== "directory") {
    await revealSftpPath(path, { quiet: true });
    return;
  }
  const wasExpanded = node.expanded;
  node.loading = true;
  try {
    const listing = await listSftpDirectory(path);
    const applied = applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
    if (applied) applied.expanded = wasExpanded;
  } catch (cause) {
    showError(cause);
  } finally {
    node.loading = false;
  }
}

/** 上传完成后的目标目录刷新：当前目录走整页列表，树内目录重列子项并保留
 *  原展开态；随后刷新所有已展开分支与折叠缓存，父目录立即可见新结果。 */
async function refreshSftpAfterUpload(targetDir?: string): Promise<void> {
  if (!session.value) return;
  const target = normalizeRemotePath(targetDir || currentPath.value);
  if (target === currentPath.value) {
    await loadDirectory();
  } else {
    const node = findFileTreeNode(sftpTreeRoot.value, target);
    if (node && node.kind === "directory") {
      const wasExpanded = node.expanded;
      node.loading = true;
      try {
        const listing = await listSftpDirectory(target);
        const applied = applyFileTreeChildren(sftpTreeRoot.value, target, listing, sftpShowHidden.value);
        if (applied) applied.expanded = wasExpanded;
      } catch {
        // 目标目录暂时不可读时保留旧列表，全局错误条已由上传链给出。
      } finally {
        node.loading = false;
      }
    } else {
      await revealSftpPath(target, { quiet: true });
    }
  }
  markCollapsedTreeNodesStale(sftpTreeRoot.value);
  await refreshExpandedSftpTree();
}

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

function togglePaneOrder() {
  paneOrder.value = paneOrder.value === "terminal-left" ? "sftp-left" : "terminal-left";
  persistState();
  void nextTick(scheduleFit);
}

function openRightPanel(view: RightPanelView) {
  // SFTP/指标仅在 SSH 工作台可用；容器面板连本机 daemon 也可用。
  if (view === "sftp" && (localUiMode.value || isRdpConnection.value)) return;
  if (view === "metrics" && (!connected.value || isRdpConnection.value)) return;
  if (view === "docker" && isRdpConnection.value) return;
  if (view !== "metrics" && metricsOpen.value) closeMetrics();
  rightPanelView.value = view;
  sftpPaneOpen.value = true;
  if (view === "metrics") openMetrics();
  persistState();
  void nextTick(scheduleFit);
}

function closeRightPanel() {
  if (metricsOpen.value) closeMetrics();
  sftpPaneOpen.value = false;
  persistState();
  void nextTick(scheduleFit);
}

function toggleSftpPane() {
  if (sftpPaneOpen.value && rightPanelView.value === "sftp") closeRightPanel();
  else openRightPanel("sftp");
}

function toggleMetricsPanel() {
  if (sftpPaneOpen.value && rightPanelView.value === "metrics") closeRightPanel();
  else openRightPanel("metrics");
}

function toggleDockerPanel() {
  if (sftpPaneOpen.value && rightPanelView.value === "docker") closeRightPanel();
  else openRightPanel("docker");
}

// 全局偏好只影响新工作台的初始面板状态；当前工作台不被连带切换。
function toggleSftpPaneDefaultOpen() {
  sftpPaneDefaultOpen.value = !sftpPaneDefaultOpen.value;
  try {
    pluginStore.setItem(SFTP_PANE_OPEN_KEY, sftpPaneDefaultOpen.value ? "true" : "false");
  } catch {
    // localStorage 不可用时偏好仅对当前会话生效。
  }
}

function loadSftpPaneDefaultOpen(): boolean {
  try {
    return sanitizeSftpPaneDefaultOpen(pluginStore.getItem(SFTP_PANE_OPEN_KEY));
  } catch {
    return false;
  }
}

// SFTP 自动连接全局偏好：关闭后连接成功不再自动加载 SFTP 目录（#179）。
function toggleSftpAutoConnect() {
  sftpAutoConnect.value = !sftpAutoConnect.value;
  try {
    pluginStore.setItem(SFTP_AUTO_CONNECT_KEY, sftpAutoConnect.value ? "true" : "false");
  } catch {
    // localStorage 不可用时偏好仅对当前会话生效。
  }
}

function loadSftpAutoConnect(): boolean {
  try {
    return sanitizeSftpAutoConnectPref(pluginStore.getItem(SFTP_AUTO_CONNECT_KEY));
  } catch {
    return true;
  }
}

// 目录跟随全局偏好：开关切换时同步写入（与 per-tab workbenchState 并行，
// 后者为恢复的 tab 提供更精确的状态；这里是新 tab 的缺省来源）。
function loadDirectoryFollowPref(): boolean {
  try {
    return sanitizeDirectoryFollowPref(pluginStore.getItem(FOLLOW_DIRECTORY_KEY));
  } catch {
    return false;
  }
}

function persistDirectoryFollowPref(enabled: boolean) {
  try {
    pluginStore.setItem(FOLLOW_DIRECTORY_KEY, enabled ? "true" : "false");
  } catch {
    // pluginStore 不可用时全局偏好仅对当前会话生效，per-tab 恢复不受影响。
  }
}

// 下载偏好（保存目录 + 每次询问 + 终端字体来源）：权威存储在 sidecar
// preferences.json——工作台 iframe 是 sandbox="allow-scripts"（opaque
// origin），localStorage 直接抛 SecurityError；localStorage 仅作 web
// 浏览器直连场景的同步缓存。
let prefsHydrated: Promise<void> | null = null;

function loadDownloadDir(): string {
  return downloadDirState.value;
}

function persistDownloadDir(value: string) {
  downloadDirState.value = value.trim();
  void syncPrefs();
}

function loadDownloadUseDefaultDir(): boolean {
  return downloadUseDefaultState.value;
}

function persistDownloadUseDefaultDir(value: boolean) {
  downloadUseDefaultState.value = value;
  void syncPrefs();
}

function loadDownloadConflictPolicy(): DownloadConflictPolicy {
  return downloadConflictState.value;
}

function persistDownloadConflictPolicy(value: DownloadConflictPolicy) {
  downloadConflictState.value = sanitizeConflictPolicy(value);
  void syncPrefs();
}

// 上传并发 / 重复目标策略（P1-5）：设置弹窗经适配器读写，权威态在此。

function loadSftpCompatMode(): boolean {
  return sftpCompatModeState.value;
}

function persistSftpCompatMode(value: boolean) {
  sftpCompatModeState.value = value;
  void syncPrefs();
}

function loadSftpNameEncoding(): SftpNameEncoding {
  return sftpNameEncodingState.value;
}

function persistSftpNameEncoding(value: SftpNameEncoding) {
  sftpNameEncodingState.value = sanitizeNameEncoding(value);
  void syncPrefs();
}

// 压缩传输（M33）：设置弹窗经适配器读写，权威态在此。
function loadTransferCompressMode(): SftpCompressMode {
  return transferCompressModeState.value;
}

function persistTransferCompressMode(value: SftpCompressMode) {
  transferCompressModeState.value = sanitizeTransferCompressMode(value);
  void syncPrefs();
}

function loadTransferCompressThreshold(): number {
  return transferCompressThresholdState.value;
}

function persistTransferCompressThreshold(value: number) {
  transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(value);
  void syncPrefs();
}

// 命令输入建议（P1-1）：设置弹窗经适配器读写，权威态在此。
function loadSuggestionsEnabled(): boolean {
  return suggestionsEnabledState.value;
}

function persistSuggestionsEnabled(value: boolean) {
  suggestionsEnabledState.value = value;
  void syncPrefs();
}

function loadSuggestionMinChars(): number {
  return suggestionMinCharsState.value;
}

function persistSuggestionMinChars(value: number) {
  suggestionMinCharsState.value = clampSuggestionMinChars(value);
  void syncPrefs();
}

function loadSuggestionMaxChars(): number {
  return suggestionMaxCharsState.value;
}

function persistSuggestionMaxChars(value: number) {
  suggestionMaxCharsState.value = clampSuggestionMaxChars(value);
  void syncPrefs();
}

function cachePrefs() {
  try {
    if (downloadDirState.value) window.localStorage.setItem(DOWNLOAD_DIR_KEY, downloadDirState.value);
    else window.localStorage.removeItem(DOWNLOAD_DIR_KEY);
    // 默认开：只在关闭时落键（"0"），未来默认策略变化时老用户不被钉死。
    if (!downloadUseDefaultState.value) window.localStorage.setItem(DOWNLOAD_USE_DEFAULT_KEY, "0");
    else window.localStorage.removeItem(DOWNLOAD_USE_DEFAULT_KEY);
    if (downloadConflictState.value !== "rename") window.localStorage.setItem(DOWNLOAD_CONFLICT_KEY, downloadConflictState.value);
    else window.localStorage.removeItem(DOWNLOAD_CONFLICT_KEY);
    if (transferConcurrencyState.value !== 3) window.localStorage.setItem(TRANSFER_CONCURRENCY_KEY, String(transferConcurrencyState.value));
    else window.localStorage.removeItem(TRANSFER_CONCURRENCY_KEY);
    if (transferMaxActiveState.value !== 3) window.localStorage.setItem(TRANSFER_MAX_ACTIVE_KEY, String(transferMaxActiveState.value));
    else window.localStorage.removeItem(TRANSFER_MAX_ACTIVE_KEY);
    if (transferDownloadLimitState.value !== 0) window.localStorage.setItem(TRANSFER_DOWNLOAD_LIMIT_KEY, String(transferDownloadLimitState.value));
    else window.localStorage.removeItem(TRANSFER_DOWNLOAD_LIMIT_KEY);
    if (sftpCompatModeState.value) window.localStorage.setItem(SFTP_COMPAT_MODE_KEY, "1");
    else window.localStorage.removeItem(SFTP_COMPAT_MODE_KEY);
    if (sftpNameEncodingState.value !== "auto") window.localStorage.setItem(SFTP_NAME_ENCODING_KEY, sftpNameEncodingState.value);
    else window.localStorage.removeItem(SFTP_NAME_ENCODING_KEY);
    // 压缩传输（M33）：默认 auto/默认 64 时不落键，老用户不被钉死。
    if (transferCompressModeState.value !== "auto") window.localStorage.setItem(TRANSFER_COMPRESS_MODE_KEY, transferCompressModeState.value);
    else window.localStorage.removeItem(TRANSFER_COMPRESS_MODE_KEY);
    if (transferCompressThresholdState.value !== 64) window.localStorage.setItem(TRANSFER_COMPRESS_THRESHOLD_KEY, String(transferCompressThresholdState.value));
    else window.localStorage.removeItem(TRANSFER_COMPRESS_THRESHOLD_KEY);
    // 默认已改为 ask：显式选择 rename 也必须落键，否则下次启动会被默认值覆盖。
    window.localStorage.setItem(TRANSFER_DUPLICATE_KEY, transferDuplicateState.value);
    if (!suggestionsEnabledState.value) window.localStorage.setItem(SUGGESTIONS_ENABLED_KEY, "0");
    else window.localStorage.removeItem(SUGGESTIONS_ENABLED_KEY);
    if (suggestionMinCharsState.value !== 1) window.localStorage.setItem(SUGGESTIONS_MIN_CHARS_KEY, String(suggestionMinCharsState.value));
    else window.localStorage.removeItem(SUGGESTIONS_MIN_CHARS_KEY);
    if (suggestionMaxCharsState.value !== 64) window.localStorage.setItem(SUGGESTIONS_MAX_CHARS_KEY, String(suggestionMaxCharsState.value));
    else window.localStorage.removeItem(SUGGESTIONS_MAX_CHARS_KEY);
  } catch {
    // opaque origin：缓存跳过，内存态仍支撑本次会话。
  }
}

async function syncPrefs() {
  cachePrefs();
  try {
    await window.dbxPlugin.invoke("local/preferences/set", {
      downloadDir: downloadDirState.value,
      downloadUseDefaultDir: downloadUseDefaultState.value,
      downloadConflictPolicy: downloadConflictState.value,
      transfer_concurrency: transferConcurrencyState.value,
      transfer_duplicate_policy: transferDuplicateState.value,
      transfer_max_active: transferMaxActiveState.value,
      transfer_download_limit_kib: transferDownloadLimitState.value,
      sftp_compat_mode: sftpCompatModeState.value,
      sftp_name_encoding: sftpNameEncodingState.value,
      transfer_compress_mode: transferCompressModeState.value,
      transfer_compress_threshold_mib: transferCompressThresholdState.value,
      history_suggestions_enabled: suggestionsEnabledState.value,
      history_suggestion_min_chars: suggestionMinCharsState.value,
      history_suggestion_max_chars: suggestionMaxCharsState.value,
    });
    // 存储迁移批 3：RPC 成功即镜像同一份 14 键子集（云同步载荷；本键无设备
    // 本地字段，localShell 等不在内）。
    persistPreferencesMirror({
      downloadDir: downloadDirState.value,
      downloadUseDefaultDir: downloadUseDefaultState.value,
      downloadConflictPolicy: downloadConflictState.value,
      transfer_concurrency: transferConcurrencyState.value,
      transfer_duplicate_policy: transferDuplicateState.value,
      transfer_max_active: transferMaxActiveState.value,
      transfer_download_limit_kib: transferDownloadLimitState.value,
      sftp_compat_mode: sftpCompatModeState.value,
      sftp_name_encoding: sftpNameEncodingState.value,
      transfer_compress_mode: transferCompressModeState.value,
      transfer_compress_threshold_mib: transferCompressThresholdState.value,
      history_suggestions_enabled: suggestionsEnabledState.value,
      history_suggestion_min_chars: suggestionMinCharsState.value,
      history_suggestion_max_chars: suggestionMaxCharsState.value,
    });
  } catch {
    // 旧 sidecar 无此方法：本次会话内存态兜底（镜像不写，等下次成功）。
  }
}

function hydratePrefs(): Promise<void> {
  // Promise 记忆而非布尔：并发调用（onMounted 与本地终端直通分支）共享同一
  // 次加载，直通分支 await 它时偏好保证已就绪。
  prefsHydrated ??= hydratePrefsOnce();
  return prefsHydrated;
}

async function hydratePrefsOnce() {
  try {
    downloadDirState.value = window.localStorage.getItem(DOWNLOAD_DIR_KEY)?.trim() || "";
    downloadUseDefaultState.value = window.localStorage.getItem(DOWNLOAD_USE_DEFAULT_KEY) !== "0";
    downloadConflictState.value = sanitizeConflictPolicy(window.localStorage.getItem(DOWNLOAD_CONFLICT_KEY));
    transferConcurrencyState.value = clampTransferConcurrency(window.localStorage.getItem(TRANSFER_CONCURRENCY_KEY) ?? undefined);
    transferDuplicateState.value = sanitizeTransferDuplicatePolicy(window.localStorage.getItem(TRANSFER_DUPLICATE_KEY));
    transferMaxActiveState.value = clampTransferMaxActive(window.localStorage.getItem(TRANSFER_MAX_ACTIVE_KEY) ?? undefined);
    transferDownloadLimitState.value = clampTransferDownloadLimit(window.localStorage.getItem(TRANSFER_DOWNLOAD_LIMIT_KEY) ?? undefined);
    sftpCompatModeState.value = window.localStorage.getItem(SFTP_COMPAT_MODE_KEY) === "1";
    sftpNameEncodingState.value = sanitizeNameEncoding(window.localStorage.getItem(SFTP_NAME_ENCODING_KEY));
    transferCompressModeState.value = sanitizeTransferCompressMode(window.localStorage.getItem(TRANSFER_COMPRESS_MODE_KEY));
    transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(window.localStorage.getItem(TRANSFER_COMPRESS_THRESHOLD_KEY) ?? undefined);
    suggestionsEnabledState.value = window.localStorage.getItem(SUGGESTIONS_ENABLED_KEY) !== "0";
    suggestionMinCharsState.value = clampSuggestionMinChars(window.localStorage.getItem(SUGGESTIONS_MIN_CHARS_KEY));
    suggestionMaxCharsState.value = clampSuggestionMaxChars(window.localStorage.getItem(SUGGESTIONS_MAX_CHARS_KEY));
  } catch {
    // 同上：等待 sidecar 权威值。
  }
  try {
    const prefs = await window.dbxPlugin.invoke<{
      downloadDir?: unknown;
      downloadUseDefaultDir?: unknown;
      downloadConflictPolicy?: unknown;
      localShell?: unknown;
      localShellIntegration?: unknown;
      action_links_enabled?: unknown;
      action_links_matchers?: unknown;
      terminal_show_line_numbers?: unknown;
      terminal_show_timestamps?: unknown;
      terminal_timestamp_format?: unknown;
      transfer_concurrency?: unknown;
      transfer_duplicate_policy?: unknown;
      transfer_max_active?: unknown;
      transfer_download_limit_kib?: unknown;
      sftp_compat_mode?: unknown;
      sftp_name_encoding?: unknown;
      transfer_compress_mode?: unknown;
      transfer_compress_threshold_mib?: unknown;
      history_suggestions_enabled?: unknown;
      history_suggestion_min_chars?: unknown;
      history_suggestion_max_chars?: unknown;
      ctx_search_engines?: unknown;
      wallpaper_enabled?: unknown;
      wallpaper_opacity?: unknown;
      rdp_experimental_enabled?: unknown;
      localShellCwd?: unknown;
      localShellEnv?: unknown;
    }>("local/preferences/get", {});
    // 背景图本体与偏好同拉（旧 sidecar 无 wallpaper/* 时静默缺席）。
    void loadWallpaperImage();
    if (typeof prefs.downloadDir === "string") downloadDirState.value = prefs.downloadDir.trim();
    if (typeof prefs.downloadUseDefaultDir === "boolean") downloadUseDefaultState.value = prefs.downloadUseDefaultDir;
    if (prefs.downloadConflictPolicy !== undefined) downloadConflictState.value = sanitizeConflictPolicy(prefs.downloadConflictPolicy);
    if (typeof prefs.localShell === "string") localShellPref.value = prefs.localShell;
    if (typeof prefs.localShellIntegration === "boolean") localShellIntegrationPref.value = prefs.localShellIntegration;
    // 启动目录/环境变量：结构不合法静默回落默认（旧 sidecar 无键 = undefined）。
    if (typeof prefs.localShellCwd === "string") localShellCwdPref.value = prefs.localShellCwd;
    if (Array.isArray(prefs.localShellEnv)) {
      localShellEnvPref.value = prefs.localShellEnv
        .filter((entry): entry is { name: unknown; value: unknown } => typeof entry === "object" && entry !== null && "name" in entry)
        .map((entry) => ({ name: typeof entry.name === "string" ? entry.name : "", value: typeof entry.value === "string" ? entry.value : "" }));
    }
    // 动作链接 / gutter：键位缺省时保持内存默认（功能关闭），不无谓覆写。
    if (prefs.action_links_enabled !== undefined || prefs.action_links_matchers !== undefined) {
      actionLinksSettings.value = sanitizeActionLinksSettings({ enabled: prefs.action_links_enabled, matchers: prefs.action_links_matchers });
    }
    if (prefs.terminal_show_line_numbers !== undefined || prefs.terminal_show_timestamps !== undefined || prefs.terminal_timestamp_format !== undefined) {
      gutterSettings.value = sanitizeGutterSettings({
        showLineNumbers: prefs.terminal_show_line_numbers,
        showTimestamps: prefs.terminal_show_timestamps,
        timestampFormat: prefs.terminal_timestamp_format,
      });
    }
    if (prefs.transfer_concurrency !== undefined) transferConcurrencyState.value = clampTransferConcurrency(prefs.transfer_concurrency);
    if (prefs.transfer_duplicate_policy !== undefined) transferDuplicateState.value = sanitizeTransferDuplicatePolicy(prefs.transfer_duplicate_policy);
    if (prefs.transfer_max_active !== undefined) transferMaxActiveState.value = clampTransferMaxActive(prefs.transfer_max_active);
    if (prefs.transfer_download_limit_kib !== undefined) transferDownloadLimitState.value = clampTransferDownloadLimit(prefs.transfer_download_limit_kib);
    if (prefs.sftp_compat_mode !== undefined) sftpCompatModeState.value = prefs.sftp_compat_mode === true;
    if (prefs.transfer_compress_mode !== undefined) transferCompressModeState.value = sanitizeTransferCompressMode(prefs.transfer_compress_mode);
    if (prefs.transfer_compress_threshold_mib !== undefined) transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(prefs.transfer_compress_threshold_mib);
    if (prefs.sftp_name_encoding !== undefined) sftpNameEncodingState.value = sanitizeNameEncoding(prefs.sftp_name_encoding);
    if (prefs.history_suggestions_enabled !== undefined) suggestionsEnabledState.value = prefs.history_suggestions_enabled === true;
    if (prefs.history_suggestion_min_chars !== undefined) suggestionMinCharsState.value = clampSuggestionMinChars(prefs.history_suggestion_min_chars);
    if (prefs.history_suggestion_max_chars !== undefined) suggestionMaxCharsState.value = clampSuggestionMaxChars(prefs.history_suggestion_max_chars);
    // 在线搜索引擎表：键缺省保持默认 Google（ctxSearchEngines 解析对空/非法行鲁棒）。
    if (typeof prefs.ctx_search_engines === "string") ctxSearchEnginesText.value = prefs.ctx_search_engines;
    // 背景图偏好：键缺省保持内存默认（关 / 45%）。
    if (typeof prefs.wallpaper_enabled === "boolean") wallpaperEnabled.value = prefs.wallpaper_enabled;
    if (prefs.wallpaper_opacity !== undefined) wallpaperOpacity.value = Math.min(90, Math.max(10, Math.round(Number(prefs.wallpaper_opacity) || 45)));
    // 存储迁移批 3（IMPL_PLAN_STORAGE_SYNC）：14 个传输/下载偏好键的镜像——
    // store 缺失从 sidecar 播种；漂移（云同步恢复后）则 store 赢：采纳镜像到
    // 状态并推回 sidecar（syncPrefs）。
    const mirrored = loadPreferencesMirror();
    const sidecarView = extractPreferencesMirror(prefs);
    if (mirrored === null) {
      persistPreferencesMirror(sidecarView);
    } else if (!preferencesMirrorEquals(mirrored, sidecarView)) {
      downloadDirState.value = mirrored.downloadDir;
      downloadUseDefaultState.value = mirrored.downloadUseDefaultDir;
      downloadConflictState.value = mirrored.downloadConflictPolicy;
      transferConcurrencyState.value = mirrored.transfer_concurrency;
      transferDuplicateState.value = mirrored.transfer_duplicate_policy;
      transferMaxActiveState.value = mirrored.transfer_max_active;
      transferDownloadLimitState.value = mirrored.transfer_download_limit_kib;
      sftpCompatModeState.value = mirrored.sftp_compat_mode;
      sftpNameEncodingState.value = mirrored.sftp_name_encoding;
      transferCompressModeState.value = mirrored.transfer_compress_mode;
      transferCompressThresholdState.value = mirrored.transfer_compress_threshold_mib;
      suggestionsEnabledState.value = mirrored.history_suggestions_enabled;
      suggestionMinCharsState.value = mirrored.history_suggestion_min_chars;
      suggestionMaxCharsState.value = mirrored.history_suggestion_max_chars;
      void syncPrefs();
    }
    cachePrefs();
  } catch {
    // 旧 sidecar：保留 localStorage 种子或默认。
  }
}

// 建议长度上下限钳制已下沉 lib/preferencesMirror（存储迁移批 3）。
// 「使用默认地址」关闭时，下载/导出前弹出目录选择小窗。resolve 语义：
// { dir, setDefault } = 用户确认（dir 空串 = 默认下载目录；setDefault =
// 勾选了「将此次目录设为默认地址」）；undefined = 取消本次下载。
interface DownloadPrompt {
  fileName: string;
  dir: string;
  setDefault: boolean;
  resolve: (result: { dir: string; setDefault: boolean } | undefined) => void;
}
const downloadPrompt = ref<DownloadPrompt | null>(null);

// 单槽弹窗互斥（与 useUploadChain 的重复询问同款）：并发任务同时触发询问时，
// 后问覆盖槽位会让先问的 promise 永久挂起（该下载/上传静默卡死）。串成链，
// 后到者的询问等先到者结算完成后再占槽；弹窗 UI 保持一次只问一个。
let downloadPromptAskChain: Promise<unknown> = Promise.resolve();

function askDownloadTarget(fileName: string): Promise<{ dir: string; setDefault: boolean } | undefined> {
  const asked = downloadPromptAskChain.then(
    () =>
      new Promise<{ dir: string; setDefault: boolean } | undefined>((resolve) => {
        downloadPrompt.value = {
          fileName,
          dir: loadDownloadDir() || localDownloadDir.value,
          setDefault: false,
          resolve,
        };
      }),
  );
  downloadPromptAskChain = asked.catch(() => undefined);
  return asked;
}

function resolveDownloadPrompt(result?: { dir: string; setDefault: boolean }) {
  downloadPrompt.value?.resolve(result);
  downloadPrompt.value = null;
}

// 勾选「设为默认地址」后的闭环回写：目录成为新默认 + 自动打开
// 「使用默认地址」开关（设置页草稿同步，弹窗开着也能立即看到）。
function applyChosenDirAsDefault(dir: string) {
  const normalized = dir.trim();
  if (normalized) {
    persistDownloadDir(normalized);
    settingsDialog.value?.setDownloadDirDraft(normalized);
  }
  persistDownloadUseDefaultDir(true);
  settingsDialog.value?.setDownloadUseDefaultDraft(true);
}

// 「浏览」按钮打开应用内目录选择器（FolderPickerDialog，sidecar 列本机
// 目录）：沙箱 iframe 没有目录选择 API，原生系统对话框有窗口层级问题。
// target 记录选中值回填到哪——下载询问弹窗还是设置页草稿。
const folderPickerTarget = ref<"prompt" | "settings" | null>(null);

function onFolderPicked(path: string) {
  if (folderPickerTarget.value === "prompt" && downloadPrompt.value) {
    downloadPrompt.value.dir = path;
  } else if (folderPickerTarget.value === "settings") {
    settingsDialog.value?.setDownloadDirDraft(path);
  }
  folderPickerTarget.value = null;
}

// 「询问我」冲突策略：目标目录已有同名文件时弹确认（自动重命名/覆盖/取消）。
interface DownloadConflictPrompt {
  fileName: string;
  path: string;
  resolve: (choice: "rename" | "overwrite" | undefined) => void;
}
const downloadConflictPrompt = ref<DownloadConflictPrompt | null>(null);

let downloadConflictAskChain: Promise<unknown> = Promise.resolve();

function askDownloadConflict(fileName: string, path: string) {
  const asked = downloadConflictAskChain.then(
    () =>
      new Promise<"rename" | "overwrite" | undefined>((resolve) => {
        downloadConflictPrompt.value = { fileName, path, resolve };
      }),
  );
  downloadConflictAskChain = asked.catch(() => undefined);
  return asked;
}

function resolveDownloadConflict(choice: "rename" | "overwrite" | undefined) {
  downloadConflictPrompt.value?.resolve(choice);
  downloadConflictPrompt.value = null;
}

// 落盘前的冲突解析：返回传给后端的冲突模式，undefined = 用户取消。
// rename/overwrite 策略直接放行；ask 仅在确实撞名时打断，预检失败不阻断。
async function resolveDownloadConflictFor(dir: string, fileName: string): Promise<"rename" | "overwrite" | undefined> {
  const policy = loadDownloadConflictPolicy();
  if (policy !== "ask") return policy;
  const targetDir = dir || loadDownloadDir() || localDownloadDir.value;
  if (!targetDir) return "rename";
  try {
    const probe = await window.dbxPlugin.invoke<{ exists: boolean; path: string }>("local/fs/exists", { dir: targetDir, name: fileName });
    if (!probe.exists) return "rename";
    return await askDownloadConflict(fileName, probe.path);
  } catch {
    return "rename";
  }
}


/**
 * 终端行为落地：把行为偏好写进 xterm 选项。字体/行高/字间距等外观项不在此处
 * （见 applyTerminalAppearance），这里只管行为类选项。
 */
function applyTerminalBehavior() {
  if (!terminal) return;
  const patch = terminalBehaviorOptionPatch(terminalBehavior.value);
  terminal.options.scrollback = patch.scrollback;
  terminal.options.scrollOnUserInput = patch.scrollOnUserInput;
  terminal.options.wordSeparator = patch.wordSeparator;
  terminal.options.ignoreBracketedPasteMode = patch.ignoreBracketedPasteMode;
  terminal.options.macOptionIsMeta = patch.macOptionIsMeta;
}

/** 行为设置局部更新（设置页控件）：归一化 → 持久化 → 即时生效。 */
// 布局方向偏好改动（issue #187）：轴向切换后终端/SFTP 尺寸变化，重排 xterm。
// 持久化由 WORKBENCH_LAYOUT watch 统一写穿。
function setPaneDirection(value: PaneDirectionPref) {
  if (paneDirection.value === value) return;
  paneDirection.value = value;
  scheduleFit();
}

function updateTerminalBehavior(patch: Partial<TerminalBehaviorSettings>) {
  const next = sanitizeTerminalBehavior({ ...terminalBehavior.value, ...patch });
  const copyChanged = next.copyOnSelect !== terminalBehavior.value.copyOnSelect;
  terminalBehavior.value = next;
  persistTerminalBehavior(next);
  applyTerminalBehavior();
  // 选中复制沿用既有即时反馈文案。
  if (copyChanged) {
    showNotice(t(next.copyOnSelect ? "terminalSelectCopy.enabledNotice" : "terminalSelectCopy.disabledNotice"));
  }
}

/** 快捷键绑定更新：归一化 → 持久化。派发每次按键实时读表，无需重挂钩子。 */
function updateTerminalHotkeys(bindings: TerminalHotkeyBindings) {
  const next = sanitizeTerminalHotkeys(bindings, applePlatform);
  terminalHotkeys.value = next;
  persistTerminalHotkeys(next);
}

function startDividerDrag(event: PointerEvent) {
  const container = paneContainer.value;
  if (!container) return;
  const pointerId = event.pointerId;
  // 纵向布局（强制上下或窄屏断点）把手沿 Y 轴拖拽：按下时定轴，拖动中不变。
  const vertical = currentPaneDirection() === "column";
  const move = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    const bounds = container.getBoundingClientRect();
    if (vertical) {
      // splitRatio 是终端占比；纵向终端恒在上，把手下移 = 终端变高。
      const fromTop = ((next.clientY - bounds.top) / bounds.height) * 100;
      splitRatio.value = Math.max(35, Math.min(80, fromTop));
    } else {
      const fromLeft = ((next.clientX - bounds.left) / bounds.width) * 100;
      const terminalPercent = paneOrder.value === "terminal-left" ? fromLeft : 100 - fromLeft;
      splitRatio.value = Math.max(35, Math.min(80, terminalPercent));
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
  const vertical = currentPaneDirection() === "column";
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
  // splitRatio 是终端占比：横向 terminal-left 时把手右移 = 终端变宽（sftp-left
  // 反向）；纵向终端恒在上，把手下移 = 终端变高。
  if (currentPaneDirection() === "column") {
    if (!bounds.height) return;
    splitRatio.value = Math.max(35, Math.min(80, splitRatio.value + (deltaPx / bounds.height) * 100));
  } else {
    if (!bounds.width) return;
    const sign = paneOrder.value === "terminal-left" ? 1 : -1;
    splitRatio.value = Math.max(35, Math.min(80, splitRatio.value + (deltaPx / bounds.width) * 100 * sign));
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
const editorPaneRef = ref<InstanceType<typeof EditorPane> | null>(null);
const terminalStackRef = ref<HTMLElement | null>(null);
// 编辑器占终端区高度的比例（%）：25-75 夹取，拖拽/方向键即时改，xterm 重排。
const EDITOR_SPLIT_MIN = 25;
const EDITOR_SPLIT_MAX = 75;
const editorSplitRatio = ref(45);
watch(hasEditorTabs, () => {
  // 页签出现/清空会改变终端可用高度：等 DOM 落地后再 fit。
  void nextTick(scheduleFit);
});
watch(editorSplitRatio, () => {
  scheduleFit();
});

function startEditorDividerDrag(event: PointerEvent) {
  const divider = event.currentTarget as HTMLElement | null;
  const stack = terminalStackRef.value;
  if (!divider || !stack) return;
  event.preventDefault();
  const pointerId = event.pointerId;
  const move = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    const bounds = stack.getBoundingClientRect();
    if (!bounds.height) return;
    editorSplitRatio.value = Math.max(
      EDITOR_SPLIT_MIN,
      Math.min(EDITOR_SPLIT_MAX, ((next.clientY - bounds.top) / bounds.height) * 100),
    );
  };
  // pointercancel 时指针已被移出 active 集合，释放前必须 hasPointerCapture 守卫。
  const release = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    divider.removeEventListener("pointermove", move);
    divider.removeEventListener("pointerup", release);
    divider.removeEventListener("pointercancel", release);
    if (divider.hasPointerCapture(pointerId)) {
      try { divider.releasePointerCapture(pointerId); } catch { /* 已被隐式释放 */ }
    }
  };
  divider.setPointerCapture(pointerId);
  divider.addEventListener("pointermove", move);
  divider.addEventListener("pointerup", release);
  divider.addEventListener("pointercancel", release);
}

function onEditorDividerKeydown(event: KeyboardEvent) {
  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
  event.preventDefault();
  const delta = event.key === "ArrowDown" ? 2 : -2;
  editorSplitRatio.value = Math.max(EDITOR_SPLIT_MIN, Math.min(EDITOR_SPLIT_MAX, editorSplitRatio.value + delta));
}

/** Cmd/Ctrl+S 只接管编辑器内的焦点：终端（xterm）内保持既有输入语义。 */
function editorPaneContainsFocus() {
  const root = editorPaneRef.value?.$el;
  return root instanceof HTMLElement && root.contains(document.activeElement);
}

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

// —— 打开方式子菜单与自定义编辑器命令（编辑器映射增强）——
// 子菜单 = 系统默认 + sidecar 探测到的可用编辑器 + 用户自定义命令，选中即开
// （一次性，不写关联）；持久化关联走设置弹窗或自定义命令弹窗的「记住为默认」。
const availableExternalEditors = computed(() => externalEditors.value.filter((editor) => editor.available));
/** 当前文件按关联/默认解析到的编辑器 id（子菜单打勾标识；system 返回空串）。 */
function mappedEditorIdFor(fileName: string): string {
  const resolved = resolveEditorForFile(fileName, editorConfig.value, externalEditors.value);
  return resolved.kind === "system" ? "" : resolved.editor.id;
}
function openWithSystemEntry(entry: SftpEntry) {
  void openInExternalEditor(entry, { kind: "system" });
}
function openWithKnownEntry(entry: SftpEntry, editor: KnownEditor) {
  void openInExternalEditor(entry, { kind: "known", editor });
}
function openWithCustomEntry(entry: SftpEntry, editor: CustomEditor) {
  void openInExternalEditor(entry, { kind: "custom", editor });
}
// 自定义编辑器命令弹窗：命令串由 sidecar argv 化执行（不过 shell），
// 「记住为默认」写入 {pattern → customId} 关联（pattern 取文件扩展名掩码）。
const customEditorOpen = ref(false);
const customEditorEntry = ref<SftpEntry>();
const customEditorName = ref("");
const customEditorCommand = ref("");
const customEditorRemember = ref(true);
const customEditorPattern = computed(() =>
  customEditorEntry.value ? defaultPatternFor(customEditorEntry.value.name) : "*",
);
function beginCustomEditorCommand(entry: SftpEntry) {
  customEditorEntry.value = entry;
  customEditorName.value = "";
  customEditorCommand.value = "";
  customEditorRemember.value = true;
  customEditorOpen.value = true;
}
async function commitCustomEditor() {
  const entry = customEditorEntry.value;
  const command = customEditorCommand.value.trim();
  if (!entry || !command) return;
  const id = newCustomEditorId();
  const name = customEditorName.value.trim() || command.split(/\s+/)[0] || id;
  const next = sanitizeEditorConfig(editorConfig.value);
  next.customEditors = [...next.customEditors, { id, name, command }];
  if (customEditorRemember.value) {
    const pattern = defaultPatternFor(entry.name);
    next.associations = [
      ...next.associations.filter((assoc) => assoc.pattern.toLowerCase() !== pattern.toLowerCase()),
      { pattern, customId: id },
    ];
  }
  updateEditorConfig(next);
  customEditorOpen.value = false;
  await openInExternalEditor(entry, { kind: "custom", editor: { id, name, command } });
}

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
} = useSftpCreateDirectory({ showError, session, sudoMode, currentPath, joinRemote, loadDirectory });


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
// SFTP 路径书签（存储迁移批 1）：权威在 pluginStore（宿主 ui-storage.json，随
// DBX secrets 同步加密上云）；sidecar sftp/bookmarks/list 仅作首次拉取的一次性
// 搬迁种子。星标收藏 + 路径弹层跳转/删除。
// ---------------------------------------------------------------------------

async function refreshBookmarks() {
  const stored = loadSftpBookmarksFromStore();
  if (stored !== null) {
    sftpBookmarks.value = stored;
    return;
  }
  try {
    // 键缺失 = 未迁移：sidecar 清单（含空表）一次性搬入并落键。
    sftpBookmarks.value = persistSftpBookmarks(await listBookmarks());
  } catch {
    // 后端未升级/读取失败时保留既有列表（optional 特性静默降级，不阻塞路径栏）。
  }
}

function toggleBookmarkSave() {
  if (!connected.value) return;
  if (bookmarkSaveOpen.value) {
    bookmarkSaveOpen.value = false;
    return;
  }
  closeToolbarPopovers();
  bookmarkLabelDraft.value = defaultBookmarkLabel(currentPath.value);
  bookmarkSaveOpen.value = true;
}

async function confirmBookmarkSave() {
  if (!connected.value || bookmarkSaving.value) return;
  const input = { label: bookmarkLabelDraft.value, path: currentPath.value };
  // 前端先行校验（与后端同规则）：label 空/超长/重复、path 空/超长、超上限。
  const localError = validateBookmarkInput(input, sftpBookmarks.value);
  if (localError) {
    showNotice(t(`sftpBookmark.error.${localError}`, localError === "limitReached" ? { limit: SFTP_BOOKMARKS_LIMIT } : {}));
    return;
  }
  bookmarkSaving.value = true;
  try {
    const { bookmarks, bookmark } = upsertBookmark(sftpBookmarks.value, input);
    sftpBookmarks.value = persistSftpBookmarks(bookmarks);
    bookmarkSaveOpen.value = false;
    showNotice(t("sftpBookmark.saved", { label: bookmark.label }));
  } catch (cause) {
    showError(cause);
  } finally {
    bookmarkSaving.value = false;
  }
}

async function removeBookmark(bookmark: SftpBookmark) {
  sftpBookmarks.value = persistSftpBookmarks(removeBookmarkFromList(sftpBookmarks.value, bookmark.id));
  showNotice(t("sftpBookmark.deleted"));
}

// 连接建立后拉取书签（全局共享，不随会话清空）；打开路径弹层时刷新兜底。
watch(connected, (value) => {
  if (value) void refreshBookmarks();
});
watch(pathHistoryOpen, (open) => {
  if (open) void refreshBookmarks();
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

function openNewFileDialog() {
  if (!connected.value || !canWrite.value) return;
  newFileDraft.value = "";
  newFileDialog.value = true;
}

async function createNewFile() {
  const sessionId = session.value?.sessionId;
  const name = newFileDraft.value.trim();
  if (!sessionId || !name || newFileSubmitting.value) return;
  newFileSubmitting.value = true;
  try {
    await window.dbxPlugin.invoke(sudoMode.value ? "sudo/touch" : "sftp/touch", {
      sessionId,
      path: joinRemote(currentPath.value, name),
    });
    newFileDialog.value = false;
    showNotice(t("sftpNewFile.done", { name }));
    await loadDirectory();
  } catch (cause) {
    showError(cause);
  } finally {
    newFileSubmitting.value = false;
  }
}

async function openAttributes(entry: SftpEntry) {
  const sessionId = session.value?.sessionId;
  if (!sessionId) return;
  fileMenu.value = undefined;
  attrsTarget.value = entry;
  attrsInfo.value = undefined;
  attrsMode.value = entry.permissions || "";
  attrsLoading.value = true;
  try {
    attrsInfo.value = await window.dbxPlugin.invoke<SftpStatInfo>(sudoMode.value ? "sudo/stat" : "sftp/stat", {
      sessionId,
      path: pathFromUri(entry.uri),
    });
    if (attrsInfo.value?.mode) attrsMode.value = attrsInfo.value.mode;
  } catch (cause) {
    showError(cause);
  } finally {
    attrsLoading.value = false;
  }
}

function closeAttributes() {
  attrsTarget.value = undefined;
  attrsInfo.value = undefined;
}

async function saveAttributesPermissions() {
  const sessionId = session.value?.sessionId;
  const entry = attrsTarget.value;
  const mode = attrsMode.value.trim();
  if (!sessionId || !entry || !mode || attrsSubmitting.value) return;
  attrsSubmitting.value = true;
  try {
    await window.dbxPlugin.invoke(sudoMode.value ? "sudo/chmod" : "sftp/chmod", {
      sessionId,
      path: pathFromUri(entry.uri),
      mode,
    });
    showNotice(t("permissionsUpdated"));
    if (attrsInfo.value) attrsInfo.value = { ...attrsInfo.value, mode };
    await loadDirectory();
  } catch (cause) {
    showError(cause);
  } finally {
    attrsSubmitting.value = false;
  }
}

function remoteBasename(path: string) {
  const index = path.lastIndexOf("/");
  return index < 0 ? path : path.slice(index + 1);
}

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

/** 根目录、当前目录及其任一祖先目录禁止进入多选：它们属于批量删除/归档/
 *  下载的高危目标，且把当前目录或它的祖先移走会让整个面板状态失效。
 *  复制/预览/下载等只读动作不受此限制（菜单按 fileMenuProtected 逐项禁用）。 */
function isProtectedDirectory(entry: { uri: string; kind: string }): boolean {
  return !isSelectableSftpEntry(entry, currentPath.value, pathFromUri);
}

// 已加载树里的目录 URI 集合：URI 选区过滤器据此区分目录与文件；树尚未
// 加载到的 URI 按非目录保留，避免误删文件选择。
const loadedDirectoryUris = computed(() => {
  const uris = new Set<string>();
  for (const entry of loadedTreeEntries.value) {
    if (entry.kind === "directory") uris.add(entry.uri);
  }
  return uris;
});

function isLoadedDirectoryUri(uri: string): boolean {
  return loadedDirectoryUris.value.has(uri);
}

/** 树行选中入口（文件/文件夹统一）：文件夹仅在修饰键点击时到这里，先拦截
 *  根/当前目录，再交给多选逻辑；Ctrl 追加与 Shift 范围结果统一过过滤器，
 *  受保护目录不会进入 selectedUris。 */
function selectTreeEntry(entry: SftpEntry, event: MouseEvent | KeyboardEvent) {
  if (isProtectedDirectory(entry)) return;
  selectFile(entry, event as MouseEvent);
  const selectable = filterSelectableSftpUris(
    selectedUris.value,
    currentPath.value,
    pathFromUri,
    isLoadedDirectoryUri,
  );
  if (selectable.length === selectedUris.value.length) return;
  selectedUris.value = selectable;
  // Shift 范围可能把 selectedPath（点击入口）之外的受保护目录带入，但点击
  // 入口本身已被拦；仍按兜底处理：锚点被剔除时清空高亮，避免选中样式残留。
  if (selectedPath.value && !selectable.includes(selectedPath.value)) selectedPath.value = "";
}

// R3-P2-5：文件行键盘语义——Enter 打开（目录进入/文件预览）、F2 重命名、
// Delete 删除，对齐主流文件管理器；动作决策走 fileRowKeydown 纯模块（有
// 单测），重命名输入框内的按键已自带 .stop。
function onFileRowKeydown(event: KeyboardEvent, entry: SftpEntry) {
  const action = decideFileRowAction(event.key, canWrite.value);
  if (!action) return;
  event.preventDefault();
  event.stopPropagation();
  // 根/当前目录：键盘改名/删除与右键菜单一样禁用（范围选择可包含文件夹，
  // 但不能把正在浏览的目录移走/删掉）。
  if (isProtectedDirectory(entry) && action !== "open") return;
  if (action === "open") void openEditorTab(entry);
  else if (action === "rename") beginRename(entry);
  else deleteTarget.value = entry;
}



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
} = useSymlink({ t, showError, session, currentPath, pathFromUri, joinRemote, loadDirectory });

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

// —— 右键菜单「在线搜索」（IMPL_PLAN Task P2-8）——
// 引擎表以原始文本持久化（sidecar preferences，allowlist 键 ctx_search_engines，
// 每行 name|url 模板）；解析收口在 TerminalContextMenu.vue 的纯函数，非法行静默
// 丢弃、解析永不失败（空表只是隐藏 Search online 项）。
const ctxSearchEnginesText = ref(DEFAULT_CTX_SEARCH_ENGINES_TEXT);
const ctxSearchEngines = computed(() => parseCtxSearchEnginesText(ctxSearchEnginesText.value));
function updateCtxSearchEngines(text: string) {
  ctxSearchEnginesText.value = text;
  void persistTerminalFeaturePrefs({ ctx_search_engines: text });
}
function searchSelectionOnline(engine: CtxSearchEngine) {
  terminalMenuOpen.value = false;
  const query = terminal?.getSelection() || "";
  if (!query) return;
  const url = buildSearchUrl(engine, query);
  if (!url) return;
  // TODO(host): 宿主尚未提供 openExternal；待宿主开放后改为直接唤起系统浏览器。
  // 当前兜底：把搜索链接复制进剪贴板并提示（失败走 sftp 错误条）。
  copyTextToClipboard(url, "ctxSearch.linkCopied");
  terminal?.focus();
}

// —— 背景图（IMPL_PLAN Task P2-9，对标 NyaTerm；MVP 简化）——
// 图源权威态在 sidecar（local/wallpaper/get|set|clear，桌面端落盘
// <plugin_data_dir>/wallpaper，≤8MiB png/jpeg/webp）；web/docker 形态 set 失败
// （sidecar 存储不在本机）或旧 sidecar 无此方法时降级为仅本次会话内存态，
// UI 有说明且不持久化。开关/透明度经 preferences allowlist 键持久化。
const wallpaperEnabled = ref(false);
// 百分比 10..=90（sidecar 侧钳制同口径），渲染时 /100。
const wallpaperOpacity = ref(45);
const wallpaperDataUrl = ref("");
const wallpaperSessionOnly = ref(false);
const wallpaperActive = computed(() => wallpaperEnabled.value && !!wallpaperDataUrl.value);
async function loadWallpaperImage() {
  try {
    const result = await window.dbxPlugin.invoke<{ dataUrl?: string }>("local/wallpaper/get", {});
    if (typeof result.dataUrl === "string") wallpaperDataUrl.value = result.dataUrl;
  } catch {
    // 旧 sidecar：背景图缺席，保持内存态。
  }
}
async function setWallpaperImage(image: { base64: string; mime: string }) {
  try {
    const result = await window.dbxPlugin.invoke<{ dataUrl?: string }>("local/wallpaper/set", { imageBase64: image.base64 });
    if (typeof result.dataUrl === "string") wallpaperDataUrl.value = result.dataUrl;
    wallpaperSessionOnly.value = false;
  } catch {
    // web/docker 形态或旧 sidecar：仅本次会话内存态（mime 来自上传文件读取）。
    wallpaperDataUrl.value = `data:${image.mime || "image/png"};base64,${image.base64}`;
    wallpaperSessionOnly.value = true;
  }
}
async function clearWallpaperImage() {
  wallpaperSessionOnly.value = false;
  wallpaperDataUrl.value = "";
  try {
    await window.dbxPlugin.invoke("local/wallpaper/clear", {});
  } catch {
    // 同 set：会话内存态已清，落盘态留待桌面形态下次清除。
  }
}
function updateWallpaperEnabled(enabled: boolean) {
  wallpaperEnabled.value = enabled;
  void persistTerminalFeaturePrefs({ wallpaper_enabled: enabled });
}
function updateWallpaperOpacity(percent: number) {
  wallpaperOpacity.value = Math.min(90, Math.max(10, Math.round(percent)));
  void persistTerminalFeaturePrefs({ wallpaper_opacity: wallpaperOpacity.value });
}
// 背景图生效期强制 DOM 渲染器（P2-9）：WebGL 画布不透明，盖死背景层；关闭
// 背景图后按用户 WebGL 开关恢复。复用现有 syncWebglRenderer 切换点。
// （注册点必须在 wallpaperActive 定义之后：watch 首次求值会沿依赖链触达它。）
watch(rendererWebglEffective, (next) => {
  if (!terminal) return;
  webglRenderer.value = syncWebglRenderer(terminal, next, webglRenderer.value, () => new WebglAddon(), webglRecoveryOptions());
});

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

function toggleQuickMenu() {
  const next = !quickMenuOpen.value;
  closeToolbarPopovers();
  quickMenuOpen.value = next;
  if (next) {
    // 每次打开回到列表态：清空搜索/展开（M32-A3 后弹层只剩列表执行）。
    quickSearch.value = "";
    quickExpandedId.value = null;
  }
}

// 会话下拉：与其它 popover 同款互斥收口；打开时回到动作列表视图。
function toggleSessionMenu() {
  const next = !sessionMenuOpen.value;
  closeToolbarPopovers();
  sessionMenuOpen.value = next;
  if (next) {
    sessionMenuShellOpen.value = false;
    sessionMenuShellPickOpen.value = false;
  }
}

// 下拉内切到 shell 选项视图：同弹层视图切换，菜单不关，顺带懒加载 shell 列表。
function openSessionShellPrefs() {
  sessionMenuShellOpen.value = true;
  void openLocalShellPrefs();
}

function toggleConnectionInfo() {
  const next = !connectionInfoOpen.value;
  closeToolbarPopovers();
  connectionInfoOpen.value = next;
  if (next) {
    void measureLatency();
    void refreshConnectionAuthMethod();
    // 发行版徽标数据源是 metrics 快照：未拉过时补拉一次（一次 exec，约 0.4s），
    // 否则从未开过指标浮层的会话在连接信息里永远看不到徽标。
    if (!metrics.value) void refreshMetrics();
  }
}


// ---- 模板内联互斥清单收敛为具名 toggle（round2），与五个函数 toggle 同族 ----

function toggleColumnsMenu() {
  const next = !columnsOpen.value;
  closeToolbarPopovers();
  columnsOpen.value = next;
}

function toggleTransferPanel() {
  const next = !transferPanelOpen.value;
  closeToolbarPopovers();
  if (next) closeMetrics();
  transferPanelOpen.value = next;
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

// —— dock 面板左上角系统信息（CPU/内存/网速）：复用 ssh/metrics 既有链路 ——

// 本地终端（运行中或恢复的 shell）没有 SSH 会话，面板带改采本机样本：
// local/metrics 复用同一采集脚本走本机 shell（macOS/Linux；Windows 无 sh
// 时结构化报错、整条隐藏）。telnet/serial/vnc/rdp 的用户语义在远端设备，
// 本机指标会造成误导——不适用（connected 分支天然排除它们）。
const localTerminalActive = computed(() => isLocalMode.value || localShellRestored.value);

async function refreshPanelMetrics() {
  if (localTerminalActive.value) {
    const sample = await window.dbxPlugin
      .invoke<ServerMetrics>("local/metrics", {}, { timeoutMs: 30_000 })
      .catch(() => undefined);
    if (sample) {
      metrics.value = sample;
      metricsError.value = "";
    } else {
      metricsError.value = t("metricsLocalUnavailable");
    }
    return;
  }
  void refreshMetrics();
}

// 连接建立（或本地终端激活）后 5s 环境轮询（与指标卡同一采样环；指标卡打开
// 时让位其自带轮询，避免双发）。隐藏的 webview 不打扰；断开/失败整体隐藏。
let panelMetricsTimer = 0;
watch([connected, panelSurface, localTerminalActive], ([isConnected, onPanel, isLocal]) => {
  window.clearInterval(panelMetricsTimer);
  if (!onPanel || (!isConnected && !isLocal)) return;
  void refreshPanelMetrics();
  panelMetricsTimer = window.setInterval(() => {
    if (!metricsOpen.value && !metricsLoading.value && document.visibilityState === "visible") void refreshPanelMetrics();
  }, 5000);
}, { immediate: true });
onBeforeUnmount(() => window.clearInterval(panelMetricsTimer));

// 三项就绪才显示（首拍未回/旧 sidecar 缺 network 字段/请求失败都不渲染占位）。
const panelMetricsVisible = computed(() =>
  panelSurface.value && (connected.value || localTerminalActive.value) && metrics.value != null && !metricsError.value,
);
// SSH 面板指标失败不再静默整条隐藏：降级显示占位带，tooltip 携带原因——
// 「左上角没有 metrics」从无声降级变成可观察状态（exec 通道故障/超时/旧
// sidecar，hover 即见），下一拍成功自愈回正常数值。本地终端的平台性缺失
// （Windows 无 sh）仍按既有约定整条隐藏，不制造噪音。
const panelMetricsFailed = computed(() =>
  panelSurface.value && connected.value && !localTerminalActive.value && !!metricsError.value,
);
const panelMetricsErrorTitle = computed(() => `${t("metrics")}: ${metricsError.value}`);
const panelCpuPercent = computed<number | null>(() => {
  const value = metrics.value?.cpu?.percent;
  return value == null ? null : Math.round(value);
});
const panelMemPercent = computed<number | null>(() => {
  const memory = metrics.value?.memory;
  if (!memory?.totalBytes) return null;
  return Math.round(((memory.usedBytes ?? 0) / memory.totalBytes) * 100);
});
const panelNetRates = computed<{ rx: number; tx: number } | null>(() => {
  let rx = 0;
  let tx = 0;
  let seen = false;
  for (const net of metrics.value?.network ?? []) {
    rx += Math.max(0, net.rxRate || 0);
    tx += Math.max(0, net.txRate || 0);
    seen = true;
  }
  return seen ? { rx, tx } : null;
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

// 权限矩阵（所有者/属组/其他人 × 读/写/执行）与八进制草稿双向换算：
// 草稿非法或为空时以 0 为基准，setuid/setgid/sticky 高位原样保留。
const PERM_ROLES = [
  { who: 6, key: "permOwner" },
  { who: 3, key: "permGroup" },
  { who: 0, key: "permOthers" },
] as const;
const PERM_COLUMNS = [
  { bit: 4, key: "permRead" },
  { bit: 2, key: "permWrite" },
  { bit: 1, key: "permExec" },
] as const;

function draftModeBits(raw: string): number {
  const text = raw.trim();
  return /^[0-7]{3,4}$/.test(text) ? parseInt(text, 8) : 0;
}

function permBit(raw: string, who: number, bit: number): boolean {
  return Boolean(draftModeBits(raw) & (bit << who));
}

function applyPermBit(raw: string, who: number, bit: number, on: boolean): string {
  const bits = draftModeBits(raw);
  const mode = on ? bits | (bit << who) : bits & ~(bit << who);
  return `0${mode.toString(8)}`;
}

function toggleChmodPerm(who: number, bit: number, event: Event) {
  const input = event.target;
  if (input instanceof HTMLInputElement) chmodDraft.value = applyPermBit(chmodDraft.value, who, bit, input.checked);
}

function toggleAttrsPerm(who: number, bit: number, event: Event) {
  const input = event.target;
  if (input instanceof HTMLInputElement) attrsMode.value = applyPermBit(attrsMode.value, who, bit, input.checked);
}

function beginChmod(entry: SftpEntry) {
  if (!canWrite.value) return;
  chmodTarget.value = entry;
  chmodDraft.value = entry.permissions || "";
  fileMenu.value = undefined;
}

function openSettings() {
  settingsOpen.value = true;
  void probeLocalCapabilities();
}

async function confirmChmod() {
  const entry = chmodTarget.value;
  const mode = chmodDraft.value.trim();
  if (!session.value || !entry || !mode) return;
  chmodSubmitting.value = true;
  try {
    if (sudoMode.value) {
      await window.dbxPlugin.invoke("sudo/chmod", {
        sessionId: session.value.sessionId,
        path: pathFromUri(entry.uri),
        mode,
      });
    } else {
      await window.dbxPlugin.invoke("sftp/chmod", {
        sessionId: session.value.sessionId,
        path: pathFromUri(entry.uri),
        mode,
      });
    }
    chmodTarget.value = undefined;
    sftpDetailCache.clear();
    await loadDirectory();
    showNotice(t("permissionsUpdated"));
  } catch (cause) {
    showError(cause);
  } finally {
    chmodSubmitting.value = false;
  }
}


// 右键菜单打开瞬间的选区快照（复制用它，而不是点击时刻的实时选区）。
const menuSelectionSnapshot = ref("");

function showTerminalMenu(event: MouseEvent) {
  // RDP 画布占住终端视图时，右键属于远程桌面（画布 mousedown 已把右键发往
  // rdp/input）；本地/SSH 终端菜单整类不适用，压住菜单与系统菜单即返回。
  if (isRdpMode.value || isRdpConnection.value) {
    event.preventDefault();
    return;
  }
  // 右键四档（对标 Tabby「Mouse → Right click」）：off / menu / paste / clipboard。
  // clipboard 档按有无选区决定复制还是粘贴；Shift+右键恒出菜单，是 off 与 paste
  // 档下唯一回到菜单的逃生口（与既有行为一致）。
  // preventDefault 只给非菜单分支：reka 触发器据此跳过开菜单（同时也压住系统菜单）；
  // 菜单分支一旦 preventDefault，ContextMenuTrigger 自己就打不开了。
  const target = resolveRightClickBehavior(terminalBehavior.value, {
    hasSelection: terminal?.hasSelection() ?? false,
    shiftKey: event.shiftKey,
  });
  if (target === "menu") {
    // 打开瞬间快照选区文本：菜单接管焦点后 xterm 可能清空选区，复制时再
    // getSelection() 就会拿到空串（右键复制无效的静默失败路径）。
    menuSelectionSnapshot.value = terminal?.hasSelection() ? (terminal.getSelection() ?? "") : "";
    terminalMenuOpen.value = true;
    fileMenu.value = undefined;
    return;
  }
  event.preventDefault();
  terminalMenuOpen.value = false;
  fileMenu.value = undefined;
  if (target === "paste") void pasteTerminal();
  else if (target === "copy") void copyTerminalSelection();
}

function showFileMenu(event: MouseEvent, entry: SftpEntry) {
  // 不再 preventDefault/stopPropagation：事件要冒泡到包裹文件树的
  // ContextMenuTrigger 完成定位与打开；空白区与行共用同一菜单根，由容器处理器
  // onFileAreaContextMenu 按事件目标区分（行内目标直接返回）。
  // 右键未选中的行（或当前无选区）时先把选区收敛到该行：否则批量动作会继续
  // 作用于右键前的旧选区，而视觉上被点中的行看起来是活动项。命中行已在
  // 选区内（含多选）时保留原选区。收敛结果统一过滤受保护目录：右键受保护
  // 目录只更新 selectedPath 作为视觉锚点，绝不进入批量选区。
  const seed = resolveSftpContextMenuSelection(entry.uri, selectedUris.value);
  const selectable = filterSelectableSftpUris(
    seed ?? selectedUris.value,
    currentPath.value,
    pathFromUri,
    isLoadedDirectoryUri,
  );
  if (seed) {
    selectedUris.value = selectable;
    if (selectable.length) lastClickedUri.value = entry.uri;
  } else if (selectable.length !== selectedUris.value.length) {
    // 旧状态里残留的受保护目录同样在右键时剔除。
    selectedUris.value = selectable;
  }
  selectedPath.value = entry.uri;
  fileMenuProtected.value = isProtectedDirectory(entry);
  fileMenu.value = {
    entry,
    // 与批量操作/计数同源：只快照仍挂在已加载树上的选中项。
    selection: selectedEntries.value.map((item) => item.uri),
  };
  terminalMenuOpen.value = false;
  blankMenu.value = false;
  transferHistoryMenu.value = undefined;
}

// 右键菜单关闭（reka 外点/Esc/动作选中都会把 fileMenu 置空）：受保护标记
// 跟随菜单生命周期，下一次普通行右键重新计算。
watch(fileMenu, (menu) => {
  if (!menu) fileMenuProtected.value = false;
});

function showTransferHistoryMenu(event: MouseEvent, entry: TransferHistoryEntry) {
  // 浏览器下载、上传及旧记录都可能没有可验证的本机路径：拦截系统菜单与
  // reka 触发器（preventDefault 后 reka 跳过打开），但不展示无效操作。
  if (!entry.localPath) {
    event.preventDefault();
    transferHistoryMenu.value = undefined;
    return;
  }
  transferHistoryMenu.value = { taskId: entry.taskId };
  terminalMenuOpen.value = false;
  fileMenu.value = undefined;
  blankMenu.value = false;
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
}

function closeMenus() {
  terminalMenuOpen.value = false;
  fileMenu.value = undefined;
  blankMenu.value = false;
  transferHistoryMenu.value = undefined;
  closeToolbarPopovers();
}

/** document click 收口：reka 菜单/弹层内容 portal 到 body，内部点击会冒泡到
 *  document——旧实现面板上有 @click.stop，内部点击从不触发这里的清扫，保持语义一致。
 *  dialog-content 同样排除：模态弹窗（如 Docker 面板的日志抽屉/kill 确认）
 *  portal 在工具条 popover 之外，点弹窗内部不该把背后的工具条弹层连带收掉。 */
function onDocumentClickCloseMenus(event: MouseEvent) {
  if ((event.target as HTMLElement | null)?.closest?.('[data-slot="context-menu-content"], [data-slot="popover-content"], [data-slot="dialog-content"]')) return;
  closeMenus();
}

/** 多选批量：复制所选路径（换行拼接写入剪贴板）。 */
function copySelectedPaths() {
  const menu = fileMenu.value;
  fileMenu.value = undefined;
  if (!menu) return;
  const uris = menu.selection.length ? menu.selection : [menu.entry.uri];
  copyTextToClipboard(uris.map((uri) => pathFromUri(uri)).join("\n"), "sftpCopy.copiedPaths", { count: uris.length });
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

function pathFromUri(uri: string) {
  return uri.replace(/^sftp:/, "") || "/";
}

function normalizeRemotePath(path: string) {
  let value = path.trim() || "/";
  try { value = decodeURIComponent(value); } catch {}
  if (!value.startsWith("/")) value = `/${value}`;
  value = value.replace(/\/{2,}/g, "/");
  return value === "/" ? value : value.replace(/\/+$/, "");
}

function joinRemote(parent: string, name: string) {
  return `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/${name.replace(/^\/+/, "")}`;
}

function parentPath(path: string) {
  const normalized = path.replace(/\/+$/, "");
  const index = normalized.lastIndexOf("/");
  return index <= 0 ? "/" : normalized.slice(0, index);
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
  if (shouldLazyLoadSftpListing({ paneOpen: open, sessionLive: !!session.value, listingLoaded: sftpListingLoaded.value }) || !sftpTreeRoot.value.loaded) {
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
  window.clearTimeout(promptHintsQuietTimer);
  stopCommandMarkerTick();
  stopAgentPromptTimer();
  resolvePasteConfirm(false);
  resolvePendingConfirmDialog(false);
  if (terminalHost.value) {
    if (terminalPasteHandler) terminalHost.value.removeEventListener("paste", terminalPasteHandler, true);
    if (terminalWheelHandler) terminalHost.value.removeEventListener("wheel", terminalWheelHandler, true);
    if (terminalMouseDownHandler) terminalHost.value.removeEventListener("mousedown", terminalMouseDownHandler);
    if (terminalMouseUpHandler) terminalHost.value.removeEventListener("mouseup", terminalMouseUpHandler);
  }
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

// 空提示符快捷键引导条（TerminalPromptHints）：引导 Warp 对齐批次的键位
// （↑ 历史 / Ctrl+R 搜索 / Ctrl+Space 补全 / → 接受行内建议）。开关与一次性
// 消散旗标存 pluginStore；显隐门在 lib/terminalPromptHints.ts（纯函数），
// 行空判定经 trackPendingInput / setPendingTerminalInput 两个汇点镜像；
// 用户实际用过任一被引导功能（开面板/手动补全/接受 ghost）即永久消散。
// 声明位置收在 setup 尾部：watch 立即求值 computed（→ terminalTransferBusy
// → zmodemBusy），依赖链上有后段才声明的 const，放前面必踩 TDZ。
const promptHintsEnabled = ref(loadPromptHintsEnabled());
const promptHintsDismissed = ref(loadPromptHintsDismissed());
const promptHintsLineEmpty = ref(true);
const promptHintsAnchor = ref<SuggestionAnchor | null>(null);
// 输出活跃抑制（真机反馈：登录 banner 刷屏期引导条闪现遮挡输出）：写输出即
// 置 quiet=false，600ms 静默后放行（防抖单 timer；会话复位归位）。
const promptHintsOutputQuiet = ref(true);
let promptHintsQuietTimer: ReturnType<typeof setTimeout> | undefined;
function markTerminalOutputActive() {
  promptHintsOutputQuiet.value = false;
  if (promptHintsQuietTimer) clearTimeout(promptHintsQuietTimer);
  promptHintsQuietTimer = setTimeout(() => {
    promptHintsQuietTimer = undefined;
    promptHintsOutputQuiet.value = true;
  }, 600);
}
// 密码提示启发采样：光标行与上一行文本命中 password/passphrase 提示 → 抑制。
function passwordPromptOnScreen(): boolean {
  try {
    const buffer = terminal?.buffer.active;
    if (!buffer || buffer.type !== "normal") return false;
    const row = buffer.baseY + Math.min(buffer.cursorY, buffer.length - 1);
    for (let y = row; y >= Math.max(0, row - 1); y -= 1) {
      if (isPasswordPromptLine(buffer.getLine(y)?.translateToString(true) ?? "")) return true;
    }
  } catch {
    return false;
  }
  return false;
}
// 终端交互提示采样（issue #150 及其反馈扩展）：光标行邻域命中询问输入/
// 选择提示（动态码、密码、选择菜单、y/n 确认）→「交互待答态」。纯判定在
// lib/interactivePromptGuard，这里只做 xterm buffer 读取。
function interactivePromptOnScreen(): boolean {
  try {
    const buffer = terminal?.buffer.active;
    if (!buffer || buffer.type !== "normal") return false;
    const row = buffer.baseY + Math.min(buffer.cursorY, buffer.length - 1);
    return interactivePromptNearCursor(row, (line) => buffer.getLine(line)?.translateToString(true) ?? "");
  } catch {
    return false;
  }
}
/** 「终端交互提示待答」门：交互提示行在场（远端询问输入/要求选择）或连接
 *  挑战弹窗未决（host-key/连接挑战 workbench 回退）任一成立。惰性求值：仅
 *  键入/按键期采样，无 setup 顺序约束。 */
function interactivePromptGatePending(): boolean {
  return isInteractivePromptPending({
    promptLineOnScreen: interactivePromptOnScreen(),
    challengeDialogPending: !!hostKeyPrompt.value,
  });
}
// 引导条键位标签跟随实时绑定（不写死键名）：历史搜索取 command-history 的
// 首个 Ctrl 系绑定（默认表里即 Ctrl+R，⌘⇧H 在前但 mac 上 ⌘ 系不是终端键位
// 心智），补全取 completions 首个绑定（默认 Ctrl+/）。解绑则隐藏对应项。
function promptHintsKeyLabel(actionId: TerminalHotkeyActionId, preferCtrl: boolean): string {
  const combos = terminalHotkeys.value[actionId] ?? [];
  if (!combos.length) return "";
  const pick = preferCtrl ? (combos.find((combo) => combo.startsWith("Ctrl+")) ?? combos[0]) : combos[0];
  return formatHotkeyDisplay(pick, applePlatform);
}
const promptHintsHistoryKey = computed(() => promptHintsKeyLabel("command-history", true));
const promptHintsCompletionsKey = computed(() => promptHintsKeyLabel("completions", false));
const promptHintsVisible = computed(() =>
  shouldShowPromptHints({
    enabled: promptHintsEnabled.value,
    dismissed: promptHintsDismissed.value,
    sessionActive: isLocalMode.value || !!session.value?.sessionId,
    lineEmpty: promptHintsLineEmpty.value,
    alternateActive: terminal?.buffer.active.type === "alternate",
    commandRunning: commandRunning.value,
    transferBusy: terminalTransferBusy.value,
    overlayOpen: suggestionOpen.value || completionOpen.value || historyPanelOpen.value || quickSelectOpen.value || searchOpen.value,
    aiSearchActive: aiSearchState.value.active,
    outputQuiet: promptHintsOutputQuiet.value,
    passwordPromptOnScreen: passwordPromptOnScreen(),
  }),
);
watch(promptHintsVisible, (visible) => {
  if (visible) promptHintsAnchor.value = readTerminalSuggestionAnchor();
});
function setPromptHintsEnabled(value: boolean) {
  promptHintsEnabled.value = value;
  savePromptHintsEnabled(value);
  // 重开开关即清除消散旗标——用户明确要再看引导。
  if (value && promptHintsDismissed.value) {
    promptHintsDismissed.value = false;
    savePromptHintsDismissed(false);
  }
}
/** 消散（✗ 或用过任一功能）：置位 + 持久化，本安装不再弹。 */
function dismissPromptHints() {
  if (promptHintsDismissed.value) return;
  promptHintsDismissed.value = true;
  savePromptHintsDismissed(true);
}

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
        <Popover :open="connectionInfoOpen" @update:open="(open) => { if (!open) connectionInfoOpen = false; }">
          <PopoverAnchor as-child>
            <button type="button" class="icon-button icon-neutral" :title="t('connectionInfo')" :aria-label="t('connectionInfo')" :aria-expanded="connectionInfoOpen" @click.stop="toggleConnectionInfo"><Info /></button>
          </PopoverAnchor>
          <PopoverContent class="popover connection-info-popover" align="start" :side-offset="5">
          <h3>{{ t("connectionInfo") }}</h3>
          <dl class="connection-info-grid">
            <dt>{{ t("connectionInfoHost") }}</dt><dd class="mono"><span v-if="metricsDistroBadge" class="distro-badge" :style="{ backgroundColor: metricsDistroBadge.color }" :title="metricsDistroBadge.name">{{ metricsDistroBadge.label }}</span> {{ connection.host || connection.name || "–" }}</dd>
            <dt>{{ t("connectionInfoPort") }}</dt><dd class="mono">{{ connection.port || 22 }}</dd>
            <dt>{{ t("connectionInfoUser") }}</dt><dd class="mono">{{ connection.username || "–" }}</dd>
            <dt>{{ t("connectionInfoAuth") }}</dt><dd>{{ connectionAuthMethodLabel }}</dd>
            <template v-if="connection.readOnly || connectionReadOnly"><dt>{{ t("readOnly") }}</dt><dd>{{ t("yes") }}</dd></template>
            <dt>{{ t("connectionInfoLatency") }}</dt>
            <dd>
              <span class="mono">{{ connectionLatencyBusy ? t("connectionInfoMeasuring") : formatLatency(connectionLatency) }}</span>
              <span v-if="connectionLatencyFailed && !connectionLatencyBusy" class="task-error">{{ t("connectionInfoFailed") }}</span>
              <button class="link-button" :disabled="connectionLatencyBusy || !connected" @click="measureLatency">{{ t("connectionInfoMeasure") }}</button>
            </dd>
          </dl>
          </PopoverContent>
        </Popover>
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
                <button class="icon-button icon-cyan" :class="{ 'is-active': rdpResolutionOpen }" :title="t('rdp.resolution.title')" :aria-label="t('rdp.resolution.title')" :aria-expanded="rdpResolutionOpen" @click.stop="rdpResolutionOpen = !rdpResolutionOpen"><Proportions /></button>
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
                <button class="icon-button icon-cyan" :class="{ 'is-active': rdpPasteOpen }" :title="t('rdp.paste.title')" :aria-label="t('rdp.paste.title')" :aria-expanded="rdpPasteOpen" @click.stop="rdpPasteOpen = !rdpPasteOpen"><ClipboardPaste /></button>
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
              <button class="icon-button icon-emerald" :class="{ 'is-active': sessionMenuOpen }" :title="t('sessionMenu.title')" :aria-label="t('sessionMenu.title')" :aria-expanded="sessionMenuOpen" @click.stop="toggleSessionMenu"><ChevronDown /></button>
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
                        @click="openLocalShellSurfaceMenu"
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
        <label v-if="!localUiMode" class="follow-directory-control" :title="t('followTerminal')">
          <Switch size="sm" :model-value="followDirectory" :disabled="!connected || panelSurface" @update:model-value="setDirectoryTracking" />
          <span>{{ t("followTerminal") }}</span>
        </label>
        <span class="toolbar-separator" aria-hidden="true" />
        <button v-if="!localUiMode" class="icon-button icon-neutral" :title="t('commandTitle')" :aria-label="t('commandTitle')" :disabled="!connected" @click="openCommandDialog"><SquareTerminal /></button>
        <button v-if="!localUiMode && !panelSurface" class="icon-button icon-neutral" :class="{ 'is-active': batchBarOpen }" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" :aria-pressed="batchBarOpen" :disabled="!connected" @click="toggleBatchBar"><ListChecks /></button>
        <div>
          <Popover :open="quickMenuOpen" @update:open="(open) => { if (!open) quickMenuOpen = false; }">
            <PopoverAnchor as-child>
              <button class="icon-button icon-amber" :title="t('quickCommands')" :aria-label="t('quickCommands')" :disabled="!connected" @click.stop="toggleQuickMenu"><Zap /></button>
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
              <button class="icon-button" :class="agentMode === 'off' ? 'icon-neutral' : 'icon-emerald is-active'" :title="t('agentTerminalQuickHint')" :aria-label="t('agentTerminalQuickHint')" :aria-pressed="agentMode !== 'off'" :disabled="!connected" @click.stop="toggleAgentModeMenu"><Bot /></button>
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
              <button class="icon-button icon-violet" :title="t('sftpPane.preferences')" :aria-label="t('sftpPane.preferences')" :disabled="panelSurface" @click.stop="toggleColumnsMenu"><SlidersHorizontal /></button>
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
          <button class="icon-button icon-blue" :title="t('transfers')" :aria-label="t('transfers')" :disabled="panelSurface" @click.stop="toggleTransferPanel"><ArrowUpDown /><span v-if="activeTransfers" class="activity-dot" /></button>

        </div>
        </template>
      </div>
      <!-- dock 面板左组：系统信息带（CPU/内存/网速，5s 轮询），flex:1 把
           右侧按钮组推到右缘；数据未就绪/不支持时整条隐藏不占位。 -->
      <div v-if="panelSurface" class="panel-toolbar-left">
        <div v-if="panelMetricsVisible" class="panel-metrics" :title="t('metrics')">
          <span>CPU <b>{{ panelCpuPercent ?? "–" }}%</b></span>
          <span>MEM <b>{{ panelMemPercent ?? "–" }}%</b></span>
          <span v-if="panelNetRates">↓ <b>{{ formatRate(panelNetRates.rx) }}</b> ↑ <b>{{ formatRate(panelNetRates.tx) }}</b></span>
        </div>
        <div v-else-if="panelMetricsFailed" class="panel-metrics panel-metrics-degraded" :title="panelMetricsErrorTitle">
          <span>CPU <b>–%</b></span>
          <span>MEM <b>–%</b></span>
        </div>
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
        <button v-if="!isRdpConnection" class="icon-button compact" :title="t('transfers')" :aria-label="t('transfers')" @click.stop="toggleTransferPanel"><ArrowUpDown /><span v-if="activeTransfers" class="activity-dot" /></button>
        <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
        <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
        <Popover v-if="!isRdpMode" :open="panelPaletteOpen" @update:open="(open) => { if (!open) panelPaletteOpen = false; }">
          <PopoverAnchor as-child>
            <button class="icon-button compact icon-violet" :class="{ 'is-active': panelPaletteOpen }" :title="t('terminalAppearance.schemeSection')" :aria-label="t('terminalAppearance.schemeSection')" :aria-expanded="panelPaletteOpen" @click.stop="panelPaletteOpen = !panelPaletteOpen"><Palette /></button>
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
      <Popover :open="transferPanelOpen" @update:open="(open) => { if (!open && !transferHistoryMenu) transferPanelOpen = false; }">
        <PopoverAnchor as-child>
          <span class="transfer-popover-anchor" aria-hidden="true" />
        </PopoverAnchor>
        <PopoverContent class="popover transfer-popover" align="end" :side-offset="5">
            <h3>{{ t("transfers") }}</h3>
            <div v-if="!transferList.length && !folderUploadProgress" class="empty compact">{{ t("noTransfers") }}</div>
            <!-- 文件夹批量上传（issue #78）：聚合进度卡——目录 X/Y · 文件 N/M · 字节。
                 逐文件的任务卡仍会正常出现在下方（uploadSource 注册 transferTasks）。 -->
            <article v-if="folderUploadProgress" class="transfer-card">
              <div class="transfer-title"><FolderUp /><span>{{ t("folderUpload.title") }}</span><strong>{{ folderUploadPercent(folderUploadProgress) }}%</strong></div>
              <progress :value="folderUploadPercent(folderUploadProgress)" max="100" />
              <div class="transfer-meta"><span>{{ t("folderUpload.progress", { dirs: folderUploadProgress.directoriesDone, dirsTotal: folderUploadProgress.directoriesTotal, files: folderUploadProgress.filesDone, filesTotal: folderUploadProgress.filesTotal }) }}</span><span>{{ formatBytes(folderUploadProgress.bytesDone) }} / {{ formatBytes(folderUploadProgress.totalBytes) }}</span></div>
              <p v-if="folderUploadProgress.currentFile" class="transfer-path mono" :title="folderUploadProgress.currentFile">{{ folderUploadProgress.currentFile }}</p>
            </article>
            <!-- 普通多文件上传批次聚合（issue #165）：文件 N/M + 总字节；批次
                 卡整批常驻，填补逐文件卡交接间隙的"传完就没"空窗；批次编排
                 结束后由 lib/fileBatch 延迟收尾清卡（不清会在 30s 终态清收后
                 回跳 0/N·0%）。失败后缀只在有失败时出现（失败计入不吞总数）。 -->
            <article v-if="fileBatchProgress" class="transfer-card">
              <div class="transfer-title"><FileUp /><span>{{ t("uploadBatch.title") }}</span><strong>{{ batchUploadPercent(fileBatchProgress) }}%</strong></div>
              <progress :value="batchUploadPercent(fileBatchProgress)" max="100" />
              <div class="transfer-meta"><span>{{ t("uploadBatch.progress", { files: fileBatchProgress.filesDone, filesTotal: fileBatchProgress.filesTotal }) }}<template v-if="fileBatchProgress.filesFailed"> · {{ t("uploadBatch.failed", { count: fileBatchProgress.filesFailed }) }}</template></span><span>{{ formatBytes(fileBatchProgress.bytesDone) }} / {{ formatBytes(fileBatchProgress.totalBytes) }}</span></div>
              <p v-if="fileBatchProgress.currentFile" class="transfer-path mono" :title="fileBatchProgress.currentFile">{{ fileBatchProgress.currentFile }}</p>
            </article>
            <article v-for="task in transferList" :key="task.taskId" class="transfer-card">
              <div class="transfer-title"><FileUp v-if="task.direction === 'upload'" /><Download v-else /><span>{{ task.fileName || task.taskId }}</span><span v-if="task.compression === 'gzip'" class="transfer-badge" :title="t('transferCompress.badgeHint')">{{ t("transferCompress.badge") }}</span><strong v-if="task.phase !== 'staging'">{{ transferPercent(task) }}%</strong></div>
              <progress :value="transferBarValue(task)" max="100" />
              <div class="transfer-meta"><span>{{ t(`transferStatus.${task.status}`) }}</span><span :title="transferBytesTitle(task)">{{ formatBytes(transferShownBytes(task)) }} / {{ formatBytes(task.size) }}</span><span v-if="transferSpeeds[task.taskId]">{{ formatBytes(transferSpeeds[task.taskId]) }}/s</span></div>
              <!-- 目录下载：在传文件相对路径，让长传输有可感知的推进。 -->
              <p v-if="task.currentFile" class="transfer-path mono" :title="task.currentFile">{{ task.currentFile }}</p>
              <p v-if="task.localPath" class="transfer-path mono" :title="task.localPath">{{ task.localPath }}</p>
              <!-- 目录下载部分失败的汇总由完成 toast 承担：failedCount 与终态
                   同拍赋值，终态卡随即转入历史区，活跃卡上的失败行永远渲染
                   不到（UI 回归确认），故不再放置死分支。 -->
              <div v-if="transferPausable(task.status) || task.status === 'queued' || task.status === 'running' || task.localPath" class="transfer-actions">
                <button v-if="transferPausable(task.status)" class="icon-button" :title="t(pausedTaskIds.has(task.taskId) ? 'transferResume' : 'transferPause')" :aria-label="t(pausedTaskIds.has(task.taskId) ? 'transferResume' : 'transferPause')" @click="toggleTransferPause(task)"><Play v-if="pausedTaskIds.has(task.taskId)" /><Pause v-else /></button>
                <button v-if="task.status === 'queued' || task.status === 'running'" class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" @click="cancelTransfer(task)"><X /></button>
                <button v-if="task.localPath" class="icon-button" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click="revealTransferTarget(task.localPath)"><FolderOpen /></button>
                <button v-if="task.localPath" class="icon-button" :title="t('openDownloadedFile')" :aria-label="t('openDownloadedFile')" @click="openTransferTarget(task.localPath)"><FileText /></button>
              </div>
              <p v-if="task.error" class="task-error">{{ task.error }}</p>
            </article>
            <!-- 保留上传断点续传入口；文件选择器本身始终隐藏，仅由 Resume 按钮唤起。 -->
            <template v-if="resumableTasks.length">
              <h3 class="transfer-history-title">{{ t("resumableTitle") }}</h3>
              <article v-for="task in resumableTasks" :key="task.taskId" class="transfer-card">
                <div class="transfer-title"><FileUp /><span :title="task.remotePath">{{ task.fileName }}</span></div>
                <div class="transfer-meta"><span>{{ formatBytes(task.resumableBytes) }} / {{ formatBytes(task.size) }}</span></div>
                <div class="transfer-actions"><button class="icon-button" :title="t('resumableResume')" :aria-label="t('resumableResume')" @click="beginResumeUpload(task)"><Play /></button></div>
              </article>
            </template>
            <input ref="resumeInput" type="file" class="hidden" @change="onResumeFilePicked" />
            <!-- 传输历史与活跃任务并列展示：历史是落盘快照，不应被当前传输状态遮住。 -->
            <div class="transfer-history-head">
              <h3 class="transfer-history-title">{{ t("transfersHistory.title") }}</h3>
              <span class="transfer-history-actions">
                <button type="button" class="icon-button" :title="t('refresh')" :aria-label="t('refresh')" :disabled="transferHistoryLoading || resumableLoading" @click.stop="refreshTransferPanel"><RefreshCw :class="{ spinning: transferHistoryLoading || resumableLoading }" /></button>
                <button type="button" class="icon-button" :title="t('transfersHistory.clear')" :aria-label="t('transfersHistory.clear')" :disabled="!transferHistory.length" @click.stop="transferHistoryClearOpen = true"><Trash2 /></button>
              </span>
            </div>
            <div v-if="transferHistoryFailed" class="empty compact">
              <span>{{ t("transfersHistory.loadFailed") }}</span>
              <button type="button" class="link-button" @click.stop="refreshTransferPanel">{{ t("refresh") }}</button>
            </div>
            <div v-else-if="transferHistoryLoading && !transferHistory.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
            <div v-else-if="!transferHistory.length" class="empty compact">{{ t("transfersHistory.empty") }}</div>
            <ContextMenu v-for="entry in transferHistory" :key="entry.taskId" :open="transferHistoryMenu?.taskId === entry.taskId" @update:open="(open) => { if (!open && transferHistoryMenu?.taskId === entry.taskId) transferHistoryMenu = undefined; }">
              <ContextMenuTrigger as-child>
                <article class="transfer-card transfer-history-card" @contextmenu="showTransferHistoryMenu($event, entry)">
                  <div class="transfer-title"><FileUp v-if="entry.direction === 'upload'" /><Download v-else /><span :title="entry.fileName">{{ entry.fileName || entry.taskId }}</span></div>
                  <div class="transfer-meta"><span>{{ t(`transferStatus.${entry.status}`) }}</span><span>{{ formatBytes(entry.size) }}</span></div>
                  <p v-if="entry.localPath" class="transfer-path mono" :title="entry.localPath">{{ entry.localPath }}</p>
                  <p v-if="entry.error" class="task-error">{{ entry.error }}</p>
                </article>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem @select="entry.localPath && revealTransferTarget(entry.localPath)"><FolderOpen />{{ t("revealInFolder") }}</ContextMenuItem>
                <ContextMenuItem @select="entry.localPath && openTransferTarget(entry.localPath)"><FileText />{{ t("openDownloadedFile") }}</ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
            </PopoverContent>
      </Popover>
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
      <!-- 工作台内终端页签：主 PTY + 额外 PTY 都在当前连接会话 tab 内。 -->
      <div v-if="!localUiMode && !isRdpConnection" class="terminal-tabbar" role="tablist" :aria-label="t('terminalTab.tabs')">
        <div v-for="tab in terminalTabs" :key="tab.id" class="terminal-tab" :class="{ 'is-active': tab.id === activeTerminalTabId }">
          <button
            type="button"
            class="terminal-tab-main"
            role="tab"
            :aria-selected="tab.id === activeTerminalTabId"
            @click="activateTerminalTab(tab.id)"
          >
            <TerminalIcon aria-hidden="true" />
            <span>{{ terminalTabLabel(tab) }}</span>
          </button>
          <button
            v-if="tab.kind === 'aux'"
            type="button"
            class="terminal-tab-close"
            :title="t('terminalTab.close')"
            :aria-label="t('terminalTab.close')"
            @click.stop="closeTerminalTab(tab.id)"
          ><X aria-hidden="true" /></button>
        </div>
        <button
          type="button"
          class="terminal-tab-add"
          :title="canAddTerminalTab ? t('terminalTab.new') : t('terminalTab.limit', { count: terminalTabs.length })"
          :aria-label="t('terminalTab.new')"
          :disabled="!canOpenAuxTerminal || !canAddTerminalTab"
          @click="createTerminalTab"
        ><Plus aria-hidden="true" /></button>
      </div>
      <div
        v-show="activeTerminalTabId === 'main'"
        class="terminal-workspace"
        @dragenter.prevent="onTerminalDragEnter"
        @dragover.prevent
        @dragleave="onTerminalDragLeave"
        @drop.prevent="onTerminalDrop($event)"
      >
      <div
        class="terminal-split-area"
        :class="{
          'is-split': terminalSplitOrientation,
          'split-row': terminalSplitOrientation === 'row',
          'split-column': terminalSplitOrientation === 'column',
        }"
      >
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
          <ContextMenuItem v-if="terminalSplitOrientation" @select="closeTerminalSplit"><X />{{ t("terminalSplit.close") }}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseZmodem"><FileUp />{{ t("zmodemUpload") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseTrzszUpload"><FileUp />{{ t("trzszUpload") }}</ContextMenuItem>
        </TerminalContextMenu>
      </ContextMenu>
      <SplitTerminalPane
        v-if="terminalSplitOrientation"
        :connection-id="connectionId || ''"
        :workbench-id="workbenchId"
        :main-session-id="session?.sessionId || ''"
        :theme="splitTerminalTheme"
        :font-family="splitTerminalFont.fontFamily"
        :font-size="splitTerminalFont.fontSize"
        :t="t"
        :locale="locale"
        :active="activeTerminalTabId === 'main'"
        :can-create-tab="canAddTerminalTab && canOpenAuxTerminal"
        :can-split="canSplitTerminal"
        close-kind="split"
        @opened="onSplitSessionOpened"
        @closed="closeTerminalSplit"
        @new-tab="createTerminalTab"
        @split="openTerminalSplit"
      />
      </div>
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
        <section v-if="connected && batchBarOpen && !isRdpConnection" class="batch-bar" @contextmenu.stop @mousedown.stop @click.stop>
          <div>
            <!-- 底部条上的 popover 向上展开（reka side="top"，旧绝对定位会被裁切）。 -->
            <Popover :open="batchTargetsOpen" @update:open="(open) => { if (!open) batchTargetsOpen = false; }">
              <PopoverAnchor as-child>
                <button class="batch-bar-targets" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" @click.stop="toggleBatchTargetsPopover"><ListChecks /><span>{{ t("batchSendTargets", { count: batchSelected.length, total: batchTargets.length }) }}</span></button>
              </PopoverAnchor>
              <PopoverContent class="popover batch-targets-popover" side="top" align="start" :side-offset="6">
              <p class="muted batch-send-hint">{{ t("batchSendHint") }}</p>
              <div class="command-history-header">
                <span>{{ t("batchSendTargets", { count: batchSelected.length, total: batchTargets.length }) }}</span>
                <span class="batch-target-actions">
                  <button class="link-button" @click="pickBatchTargets('connected')">{{ t("batchSendConnected") }}</button>
                  <button class="link-button" @click="pickBatchTargets('all')">{{ t("batchSendAll") }}</button>
                  <button class="link-button" :disabled="batchLoading" @click="refreshBatchTargets">{{ t("refresh") }}</button>
                </span>
              </div>
              <div v-if="batchLoading && !batchTargets.length" class="empty compact"><Loader2 class="spinning" /><span>{{ t("batchSendLoading") }}</span></div>
              <div v-else-if="!batchTargets.length" class="empty compact">{{ t("batchSendNoSessions") }}</div>
              <div v-else class="batch-target-list">
                <label v-for="target in batchTargets" :key="target.sessionId" class="batch-target-row" :class="{ offline: target.connected === false }">
                  <input type="checkbox" :checked="batchSelected.includes(target.sessionId)" @change="toggleBatchTargetId(target.sessionId)" />
                  <span class="mono">{{ batchTargetLabel(target) }}</span>
                  <span v-if="target.sessionId === session?.sessionId" class="batch-badge">{{ t("batchSendCurrent") }}</span>
                  <span v-if="target.connected === false" class="batch-badge batch-badge-warn">{{ t("batchSendDisconnected") }}</span>
                  <span v-if="target.readOnly" class="read-only-badge">{{ t("readOnly") }}</span>
                </label>
              </div>
              </PopoverContent>
            </Popover>
          </div>
          <!-- 快速命令弹层：与右上角面板同形态（搜索 + 可折叠分组 + 命令卡片，
               复用面板样式类与折叠态），点击卡片即回填批量输入框（发送仍由
               回车/发送按钮触发）。无真实分组时保持平铺，避免单条未分组的噪声组头。 -->
          <Popover v-if="!batchSaveMode && quickCommands.length" :open="batchQuickOpen" @update:open="(open) => { if (!open) batchQuickOpen = false; }">
            <PopoverAnchor as-child>
              <button class="batch-bar-quick batch-bar-targets" :title="t('batchSendQuickPick')" :aria-label="t('batchSendQuickPick')" @click.stop="batchQuickOpen = !batchQuickOpen">
                <Braces />
                <span>{{ batchQuickPickName || t("batchSendQuickPick") }}</span>
              </button>
            </PopoverAnchor>
            <PopoverContent class="popover batch-quick-popover" side="top" align="start" :side-offset="6">
              <div class="quick-search">
                <Search />
                <input v-model="batchQuickSearch" :placeholder="t('quickCommandsSearch')" spellcheck="false" />
              </div>
              <div v-if="!filteredBatchQuickCommands.length" class="empty compact">{{ t("quickCommandsNoMatch") }}</div>
              <template v-else-if="quickHasRealGroups">
                <section v-for="group in groupedBatchQuickCommands" :key="group.id || '__ungrouped__'" class="quick-group">
                  <button class="quick-group-head" :aria-expanded="!quickCollapsedGroups.includes(group.id)" @click="toggleQuickGroupCollapse(group.id)">
                    <ChevronDown v-if="!quickCollapsedGroups.includes(group.id)" class="quick-group-chevron" />
                    <ChevronRight v-else class="quick-group-chevron" />
                    <span class="quick-group-name">{{ quickGroupName(group) }}</span>
                    <span class="quick-group-count">{{ group.commands.length }}</span>
                  </button>
                  <template v-if="!quickCollapsedGroups.includes(group.id)">
                    <button v-for="item in group.commands" :key="item.id" class="quick-command-row quick-card" :title="item.command" :aria-label="item.command" @click="pickBatchQuick(item)">
                      <Braces class="quick-card-icon" />
                      <span class="quick-card-text">
                        <strong>{{ item.name }}</strong>
                        <span class="mono">{{ item.command }}</span>
                      </span>
                    </button>
                  </template>
                </section>
              </template>
              <button v-for="item in filteredBatchQuickCommands" v-else :key="item.id" class="quick-command-row quick-card" :title="item.command" :aria-label="item.command" @click="pickBatchQuick(item)">
                <Braces class="quick-card-icon" />
                <span class="quick-card-text">
                  <strong>{{ item.name }}</strong>
                  <span class="mono">{{ item.command }}</span>
                </span>
              </button>
              <footer class="quick-command-footer quick-command-footer--readonly">
                <span class="quick-command-limit">{{ quickCommandGroupSummary }}</span>
              </footer>
            </PopoverContent>
          </Popover>
          <input
            v-if="batchSaveMode"
            v-model="batchSaveName"
            class="batch-bar-input"
            :placeholder="t('quickCommandsName')"
            :maxlength="60"
            autofocus
            :disabled="batchSaving"
            @keydown.enter="confirmBatchBarSave"
          />
          <input
            v-else
            v-model="batchDraft"
            class="batch-bar-input mono"
            spellcheck="false"
            :placeholder="t('batchSendPlaceholder')"
            :disabled="batchSending"
            @input="broadcastBatchBarState()"
            @keydown.up.prevent="browseBatchHistoryUp"
            @keydown.down.prevent="browseBatchHistoryDown"
            @keydown.enter="sendBatchCommand"
          />
          <template v-if="batchSaveMode">
            <button class="icon-button icon-emerald" :title="t('save')" :aria-label="t('save')" :disabled="batchSaving" @click="confirmBatchBarSave"><Save /></button>
            <button class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" :disabled="batchSaving" @click="cancelBatchBarSave"><X /></button>
          </template>
          <button
            v-else
            class="icon-button"
            :title="quickCommands.length >= QUICK_COMMANDS_LIMIT ? t('quickCommandsLimit', { count: quickCommands.length, limit: QUICK_COMMANDS_LIMIT }) : t('batchBarSave')" :aria-label="quickCommands.length >= QUICK_COMMANDS_LIMIT ? t('quickCommandsLimit', { count: quickCommands.length, limit: QUICK_COMMANDS_LIMIT }) : t('batchBarSave')"
            :disabled="!batchDraft.trim() || quickCommands.length >= QUICK_COMMANDS_LIMIT"
            @click="openBatchBarSave"
          ><Save /></button>
          <button class="icon-button icon-emerald batch-bar-send" :title="t('batchSendSend')" :aria-label="t('batchSendSend')" :disabled="batchSending || !batchDraft.trim() || !batchSelected.length" @click="sendBatchCommand">
            <Loader2 v-if="batchSending" class="spinning" />
            <Send v-else />
          </button>
        </section>
      </div>
      <!-- 额外终端页签：每个页签常驻自己的 PTY，切换只切显示，不重建会话。 -->
      <div
        v-for="tab in auxTerminalTabs"
        v-show="activeTerminalTabId === tab.id"
        :key="tab.id"
        class="terminal-tab-surface terminal-tab-aux-surface"
      >
        <SplitTerminalPane
          :connection-id="connectionId || ''"
          :workbench-id="workbenchId"
          :main-session-id="session?.sessionId || ''"
          :theme="splitTerminalTheme"
          :font-family="splitTerminalFont.fontFamily"
          :font-size="splitTerminalFont.fontSize"
          :t="t"
          :locale="locale"
          :show-header="false"
          :active="activeTerminalTabId === tab.id"
          :can-create-tab="canAddTerminalTab && canOpenAuxTerminal"
          close-kind="tab"
          @opened="(sessionId) => onAuxTabOpened(tab.id, sessionId)"
          @closed="closeTerminalTab(tab.id)"
          @new-tab="createTerminalTab"
        />
      </div>
      </section>

      <div
        v-if="sftpPaneOpen && !isRdpConnection && (rightPanelView === 'docker' || !localUiMode)"
        class="divider"
        role="separator"
        :aria-orientation="currentPaneDirection() === 'column' ? 'horizontal' : 'vertical'"
        :aria-valuenow="Math.round(splitRatio)"
        aria-valuemin="35"
        aria-valuemax="80"
        :title="dividerResizeHint"
        tabindex="0"
        @pointerdown="startDividerDrag"
        @keydown="onDividerKeydown($event, nudgeSplitDivider)"
      />

      <section v-if="sftpPaneOpen && !isRdpConnection && (rightPanelView === 'docker' || !localUiMode)" ref="sftpPane" class="sftp-pane right-panel" tabindex="-1" :class="{ 'drag-active': dragActive && sftpViewActive }" @pointerdown="focusSftpPaneOnPointerDown" @paste.capture="sftpViewActive && onSftpClipboardPaste($event)" @dragenter.prevent="sftpViewActive && onSftpDragEnter($event)" @dragover.prevent @dragleave="sftpViewActive && onSftpDragLeave($event)" @drop.prevent="sftpViewActive && onDrop($event)">
        <template v-if="rightPanelView === 'sftp'">
        <div class="path-toolbar">
          <button class="icon-button" :title="t('parentFolder')" :aria-label="t('parentFolder')" :disabled="currentPath === '/'" @click="goParent"><ArrowUp /></button>
          <button class="icon-button icon-amber" :title="t('home')" :aria-label="t('home')" :disabled="!connected" @click="loadHome"><Home /></button>
          <button class="icon-button icon-cyan" :title="t('refresh')" :aria-label="t('refresh')" :disabled="!connected || loadingFiles" @click="loadDirectory()"><RefreshCw :class="{ spinning: loadingFiles }" /></button>
          <!-- #54 路径栏双态：非编辑态把当前路径渲染成可点击分段（末段为当前位置），
               点击分段直接回跳、点击分段外区域进入编辑；编辑态是原先的完整输入框，
               提交后回到分段展示。分段切分走 remotePathInput（有单测）。 -->
          <div class="path-bar">
            <input
              v-show="pathBarEditing"
              ref="pathBarInputEl"
              v-model="currentPath"
              spellcheck="false"
              @keydown.enter="submitPathInput"
              @keydown.esc="cancelPathBarEdit"
              @blur="pathBarEditing = false"
            />
            <nav v-show="!pathBarEditing" class="path-crumbs" tabindex="0" @click="beginPathBarEdit" @keydown.enter.self.prevent="beginPathBarEdit">
              <template v-for="(crumb, index) in pathCrumbs" :key="crumb.path">
                <!-- 根段本身显示为 "/"，其后不再渲染分隔符，避免「// home」双斜杠 -->
                <span v-if="index && pathCrumbs[index - 1].name !== '/'" class="path-crumb-sep" aria-hidden="true">/</span>
                <button v-if="index < pathCrumbs.length - 1" class="path-crumb mono" :title="crumb.path" @click.stop="goToPath(crumb.path)">{{ crumb.name }}</button>
                <span v-else class="path-crumb current mono" :title="crumb.path" aria-current="location">{{ crumb.name }}</span>
              </template>
            </nav>
          </div>
          <div>
            <Popover :open="bookmarkSaveOpen" @update:open="(open) => { if (!open) bookmarkSaveOpen = false; }">
              <PopoverAnchor as-child>
                <button class="icon-button icon-amber" :title="t('sftpBookmark.add')" :aria-label="t('sftpBookmark.add')" :disabled="!connected" @click.stop="toggleBookmarkSave"><Star /></button>
              </PopoverAnchor>
              <!-- 星标收藏弹层：label 默认取路径末段，可编辑后保存（前端先行校验 + 后端错误回显） -->
              <PopoverContent class="popover bookmark-save-popover" align="end" :side-offset="5">
              <strong class="path-history-title">{{ t("sftpBookmark.add") }}</strong>
              <span class="bookmark-save-path mono" :title="currentPath">{{ currentPath }}</span>
              <input v-model="bookmarkLabelDraft" class="bookmark-label-input mono" :maxlength="SFTP_BOOKMARK_LABEL_MAX_LENGTH" spellcheck="false" :placeholder="t('sftpBookmark.namePlaceholder')" :disabled="bookmarkSaving" autofocus @keydown.enter="confirmBookmarkSave" />
              <div class="bookmark-save-actions">
                <button class="icon-button icon-emerald" :title="t('save')" :aria-label="t('save')" :disabled="bookmarkSaving" @click="confirmBookmarkSave"><Save /></button>
                <button class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" :disabled="bookmarkSaving" @click="bookmarkSaveOpen = false"><X /></button>
              </div>
              </PopoverContent>
            </Popover>
          </div>
          <div>
            <Popover :open="pathHistoryOpen" @update:open="(open) => { if (!open) pathHistoryOpen = false; }">
              <PopoverAnchor as-child>
                <button class="icon-button" :title="t('sftpPathHistory.title')" :aria-label="t('sftpPathHistory.title')" :disabled="!connected" @click.stop="togglePathHistoryMenu"><History /></button>
              </PopoverAnchor>
              <PopoverContent class="popover path-history-popover" align="end" :side-offset="5">
              <strong class="path-history-title">{{ t("sftpPathHistory.title") }}</strong>
              <button v-for="item in currentPathHistory" :key="item" class="path-item mono" :title="item" :aria-label="item" @click="goToPath(item)">{{ item }}</button>
              <div v-if="!currentPathHistory.length" class="empty compact">{{ t("sftpPathHistory.empty") }}</div>
              <!-- 书签区：点击跳转，行尾悬浮删除；全局清单（跨连接共享） -->
              <strong class="path-history-title">{{ t("sftpBookmark.title") }}</strong>
              <template v-if="sftpBookmarks.length">
                <div v-for="bookmark in sftpBookmarks" :key="bookmark.id" class="bookmark-row">
                  <button class="path-item mono" :title="`${bookmark.label} · ${bookmark.path}`" :aria-label="`${bookmark.label} · ${bookmark.path}`" @click="goToPath(bookmark.path)">{{ bookmark.label }}</button>
                  <button class="bookmark-delete" :title="t('delete')" :aria-label="t('delete')" @click.stop="removeBookmark(bookmark)"><Trash2 /></button>
                </div>
              </template>
              <div v-else class="empty compact">{{ t("sftpBookmark.empty") }}</div>
              <strong class="path-history-title">{{ t("sftpQuickPath.title") }}</strong>
              <button v-for="item in SFTP_QUICK_PATHS" :key="item" class="path-item mono" :title="item" :aria-label="item" @click="goToPath(item)">{{ item }}</button>
              </PopoverContent>
            </Popover>
          </div>
          <button class="icon-button" :title="t('sftpPaste.action')" :aria-label="t('sftpPaste.action')" :disabled="!connected || !canWrite || !sftpClipboard || pasteBusy" @click="pasteClipboard"><ClipboardPaste /></button>
          <button class="icon-button icon-teal" :title="`${t('upload')} · Ctrl/Cmd+V`" :aria-label="`${t('upload')} · Ctrl/Cmd+V`" :disabled="!connected || !canWrite" @click.stop="chooseUpload"><FileUp /></button>
          <button v-if="folderUploadSupported" class="icon-button icon-teal" :title="t('folderUpload.title')" :aria-label="t('folderUpload.action')" :disabled="!connected || !canWrite" @click.stop="chooseFolderUpload"><FolderUp /></button>
          <button class="icon-button icon-amber" :title="t('newFolder')" :aria-label="t('newFolder')" :disabled="!connected || !canWrite" @click="operationDraft = ''; operationDialog = 'mkdir'"><FolderPlus /></button>
          <button class="icon-button icon-amber" :title="t('sftpNewFile.action')" :aria-label="t('sftpNewFile.action')" :disabled="!connected || !canWrite" @click="openNewFileDialog"><FilePlus /></button>
          <button class="icon-button icon-violet" :title="t('otpPanel.title')" :aria-label="t('otpPanel.title')" @click="otpPanelOpen = true"><KeyRound /></button>
          <button class="icon-button icon-blue" :title="t('importWizard.title')" :aria-label="t('importWizard.title')" @click="importPanelOpen = true"><FileInput /></button>
          <label class="follow-directory-control sudo-label" :title="!canWrite ? t('readOnly') : t('sudo.modeHint')">
            <Switch size="sm" :model-value="sudoMode" :disabled="!connected || !canWrite" @update:model-value="toggleSudoMode" />
            <span>{{ t("sudo.mode") }}</span>
          </label>
        </div>
        <!-- SFTP 面板主体：单栏文件树（文件夹 + 文件同列），无侧栏。 -->
        <div class="sftp-body">
          <div class="sftp-main">
          <div class="sftp-filter-bar">
            <label class="sftp-search-input">
              <Search />
              <input v-model="sftpSearchDraft" type="search" :placeholder="t('sftpSearch.placeholder')" spellcheck="false" />
              <button v-if="sftpSearchDraft" class="sftp-search-clear" :title="t('cancel')" :aria-label="t('cancel')" @click.prevent="clearSftpSearch"><X /></button>
            </label>
            <Select :model-value="sftpTypeFilter" @update:model-value="(v) => (sftpTypeFilter = v as SftpTypeFilter)">
              <SelectTrigger size="xs" class="sftp-type-filter" :title="t('sftpFilter.all')">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{{ t("sftpFilter.all") }}</SelectItem>
                <SelectItem value="directory">{{ t("sftpFilter.folders") }}</SelectItem>
                <SelectItem value="file">{{ t("sftpFilter.files") }}</SelectItem>
              </SelectContent>
            </Select>
            <button
              class="icon-button sftp-hidden-toggle"
              :class="{ 'is-active': sftpShowHidden }"
              :title="sftpShowHidden ? t('sftpFilter.hideHidden') : t('sftpFilter.showHidden')" :aria-label="sftpShowHidden ? t('sftpFilter.hideHidden') : t('sftpFilter.showHidden')"
              :aria-pressed="sftpShowHidden"
              @click="toggleSftpShowHidden"
            >
              <Eye v-if="sftpShowHidden" />
              <EyeOff v-else />
            </button>
          </div>
          <div v-if="selectedEntries.length > 1" class="sftp-batch-bar">
            <span>{{ t("sftpBatch.selected", { count: selectedEntries.length }) }}</span>
            <template v-if="batchProgress">
              <progress class="batch-progress-bar" :value="batchProgressPercent(batchProgress)" max="100" />
              <span class="batch-progress mono">{{ t("sftpBatch.progress", { done: batchProgress.done, total: batchProgress.total }) }}</span>
            </template>
            <button :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @click="batchArchive"><Archive />{{ t("sftpBatch.archive") }}</button>
            <button class="danger" :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @click="batchDeleteOpen = true"><Trash2 />{{ t("sftpBatch.delete") }}</button>
            <button @click="clearRowSelection"><X />{{ t("sftpBatch.clear") }}</button>
          </div>
          <!-- #209 目录跟随/连接后自动首拉失败的安静态：不弹全局错误横幅，
               在面板内给一条低调的软提示（原始错误进悬停 tooltip）+ 重试按钮。
               重试按手动导航语义执行（再失败才落全局横幅），成功或任一后续
               手动导航成功即清除；断开会话时一并清除。 -->
          <div v-if="sftpSoftError" class="sftp-soft-error" role="status" :title="sftpSoftError.detail">
            <TriangleAlert class="sftp-soft-error-icon" />
            <span class="sftp-soft-error-text">{{ sftpSoftError.message }}</span>
            <button class="sftp-soft-error-action" @click="sftpSoftError.action.run()">{{ sftpSoftError.action.label }}</button>
          </div>
          <!-- 行右键（文件操作）与空白处右键（新建/刷新）共用同一 ContextMenu 根：
               行事件由 SftpTreePane 上抛 showFileMenu 设置负载，空白区由
               onFileAreaContextMenu 兜底；定位/碰撞/Esc/外点关闭均由 reka 承担。
               树行 contextmenu 不 preventDefault，好让事件继续到本触发器。 -->
          <ContextMenu :open="!!(fileMenu || blankMenu)" @update:open="(open) => { if (!open) { fileMenu = undefined; blankMenu = false; } }">
            <ContextMenuTrigger class="contents">
              <SftpTreePane
                :rows="sftpTreeRows"
                :selected-uris="selectedUriSet"
                :selected-path="selectedPath"
                :current-path="currentPath"
                :renaming-path="renamingPath"
                :rename-draft="renameDraft"
                :rename-submitting="renameSubmitting"
                :t="t"
                :locale="locale"
                :load-details="loadSftpDetails"
                :external-drag-active="dragActive"
                :empty-text="sftpFiltersActive ? t('sftpSearch.noMatch') : ''"
                @contextmenu="onFileAreaContextMenu"
                @toggle="toggleSftpTreeNode"
                @select="selectTreeEntry"
                @open="openSftpTreeNode"
                @drop-on="onSftpTreeDrop"
                @drop-target-change="sftpDragDropTarget = $event"
                @context="(node, event) => showFileMenu(event, node.entry)"
                @rename-commit="commitTreeRename"
                @rename-cancel="renamingPath = ''"
                @row-keydown="(node, event) => onFileRowKeydown(event, node.entry)"
              />
            </ContextMenuTrigger>
            <ContextMenuContent>
                <!-- 多选感知：右键时已多选（selection > 1）→ 菜单整体切换为批量区，单项动作隐藏 -->
                <template v-if="fileMenu && !fileMenuProtected && fileMenu.selection.length > 1">
                  <ContextMenuItem @select="batchDownload"><Download />{{ t("download") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @select="batchArchive()"><Archive />{{ t("sftpBatch.archive") }}</ContextMenuItem>
                  <ContextMenuItem variant="destructive" :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @select="batchDeleteOpen = true"><Trash2 />{{ t("sftpBatch.delete") }}</ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem @select="copySelectedPaths"><Copy />{{ t("sftpCopy.copySelected") }}</ContextMenuItem>
                </template>
                <template v-else-if="fileMenu">
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'directory' || fileMenu.entry.kind === 'file'" @select="openEntry(fileMenu.entry)"><Folder v-if="fileMenu.entry.kind === 'directory'" /><FileText v-else />{{ fileMenu.entry.kind === "directory" ? t("openFolder") : t("preview") }}</ContextMenuItem>
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'file' || fileMenu.entry.kind === 'directory'" @select="downloadEntry(fileMenu.entry, sudoMode && canWrite && fileMenu.entry.kind === 'file')"><Download />{{ t("download") }}</ContextMenuItem>
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'directory'" :disabled="!connected || loadingFiles" @select="refreshSftpFolder(fileMenu.entry)"><RefreshCw />{{ t("refresh") }}</ContextMenuItem>
                  <!-- 以 root 下载（M14-C DownloadSudo）：root 大文件二进制下载，读只读门禁
                       与 sudo 族一致（sidecar 对只读连接 fail closed，这里 canWrite 同步禁用）。
                       sudo 模式下默认「下载」已自动换 sudo 车道（UploadSudo/DownloadSudo
                       同一判定），该项隐藏避免重复入口。 -->
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'file' && !sudoMode" :disabled="!canWrite" @select="downloadEntry(fileMenu.entry, true)"><Download />{{ t("sudoDownload.action") }}</ContextMenuItem>
                  <!-- 外部编辑器打开方式（编辑器映射增强）：唯一入口——独立
                       「在外部编辑器中打开」已并入子菜单（首项系统默认即原
                       行为），避免双入口语义冲突。web/docker 的 sidecar 不在
                       本机，监听与回传都不可用，localCanSave 未探测到前保持
                       禁用；选中即开（一次性，不写关联），关联管理在设置弹窗
                       「外部编辑器」区。勾选标识当前映射解析命中，右对齐。 -->
                  <ContextMenuSub v-if="fileMenu.entry.kind === 'file'" @update:open="(open: boolean) => { if (open) void ensureExternalEditors(); }">
                    <ContextMenuSubTrigger :disabled="!canWrite || !localCanSave" class="gap-2"><ExternalLink />{{ t("sftpEdit.openWith") }}<ChevronRight class="ml-auto opacity-60" /></ContextMenuSubTrigger>
                    <ContextMenuSubContent class="min-w-52">
                      <ContextMenuItem :disabled="!canWrite || !localCanSave" @select="openWithSystemEntry(fileMenu.entry)">{{ t("sftpEdit.systemDefault") }}<Check v-if="!mappedEditorIdFor(fileMenu.entry.name)" class="ml-auto" /></ContextMenuItem>
                      <template v-if="availableExternalEditors.length">
                        <ContextMenuSeparator />
                        <ContextMenuItem v-for="editor in availableExternalEditors" :key="`known-${editor.id}`" :disabled="!canWrite || !localCanSave" @select="openWithKnownEntry(fileMenu.entry, editor)">{{ editor.name }}<Check v-if="mappedEditorIdFor(fileMenu.entry.name) === editor.id" class="ml-auto" /></ContextMenuItem>
                      </template>
                      <template v-if="editorConfig.customEditors.length">
                        <ContextMenuSeparator />
                        <ContextMenuItem v-for="editor in editorConfig.customEditors" :key="`custom-${editor.id}`" :disabled="!canWrite || !localCanSave" @select="openWithCustomEntry(fileMenu.entry, editor)">{{ editor.name }}<Check v-if="mappedEditorIdFor(fileMenu.entry.name) === editor.id" class="ml-auto" /></ContextMenuItem>
                      </template>
                      <ContextMenuSeparator />
                      <ContextMenuItem :disabled="!canWrite || !localCanSave" @select="beginCustomEditorCommand(fileMenu.entry)"><Pencil />{{ t("sftpEdit.customCommand") }}</ContextMenuItem>
                    </ContextMenuSubContent>
                  </ContextMenuSub>
                  <!-- 符号链接改指向（P2-6）：读取现有 target 预填后 update。 -->
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'symlink'" :disabled="!canWrite" @select="beginSymlinkEdit(fileMenu.entry)"><Link2 />{{ t("symlink.editAction") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="beginRename(fileMenu.entry)"><Pencil />{{ t("rename") }}</ContextMenuItem>
                  <ContextMenuItem @select="copySelectedEntries('copy')"><Copy />{{ t("sftpCopy.copy") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="copySelectedEntries('cut')"><Scissors />{{ t("sftpCopy.cut") }}</ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem @select="copyTextToClipboard(pathFromUri(fileMenu.entry.uri), 'sftpCopy.copiedPath')"><Copy />{{ t("sftpCopy.copyPath") }}</ContextMenuItem>
                  <ContextMenuItem @select="copyTextToClipboard(fileMenu.entry.name, 'sftpCopy.copiedName')"><FileText />{{ t("sftpCopy.copyName") }}</ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="beginChmod(fileMenu.entry)"><Lock />{{ t("permissionsEdit") }}</ContextMenuItem>
                  <ContextMenuItem @select="openAttributes(fileMenu.entry)"><Info />{{ t("sftpAttrs.action") }}</ContextMenuItem>
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'directory' || (fileMenu.entry.kind === 'file' && !isArchiveName(fileMenu.entry.name))" :disabled="!canWrite || archiveBusy || fileMenuProtected" @select="archiveEntry(fileMenu.entry)"><Archive />{{ t("archive.action") }}</ContextMenuItem>
                  <ContextMenuItem v-if="fileMenu.entry.kind === 'file' && isArchiveName(fileMenu.entry.name)" :disabled="!canWrite || archiveBusy" @select="extractEntry(fileMenu.entry)"><PackageOpen />{{ t("extract.action") }}</ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem variant="destructive" :disabled="!canWrite || fileMenuProtected" @select="deleteTarget = fileMenu.entry"><Trash2 />{{ t("delete") }}</ContextMenuItem>
                </template>
                <!-- 文件列表空白处右键：新建文件夹 / 新建文件 / 上传文件 / 刷新 -->
                <template v-else>
                  <ContextMenuItem :disabled="!canWrite" @select="blankMenuAction('mkdir')"><FolderPlus />{{ t("newFolder") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!canWrite" @select="blankMenuAction('newFile')"><FilePlus />{{ t("sftpNewFile.action") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!canWrite" @select="blankMenuAction('symlink')"><Link2 />{{ t("symlink.createAction") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!connected || !canWrite" @select="blankMenuAction('upload')"><FileUp />{{ t("upload") }}</ContextMenuItem>
                  <ContextMenuItem v-if="folderUploadSupported" :disabled="!connected || !canWrite" @select="blankMenuAction('uploadFolder')"><FolderUp />{{ t("folderUpload.action") }}</ContextMenuItem>
                  <ContextMenuItem :disabled="!connected || loadingFiles" @select="blankMenuAction('refresh')"><RefreshCw />{{ t("refresh") }}</ContextMenuItem>
                </template>
              </ContextMenuContent>
            </ContextMenu>
            <footer class="file-footer"><span>{{ sftpFiltersActive ? t("sftpSearch.footerMatch", { matched: sftpTreeEntries.length, total: sftpTreeTotalRows.length }) : t("items", { count: loadedTreeEntries.length }) }}</span><span v-if="diskUsage" :title="`${diskUsage.filesystem} → ${diskUsage.mount}`">{{ formatBytes(diskUsage.availableBytes) }} {{ t("diskFreeOf", { total: formatBytes(diskUsage.totalBytes) }) }}</span><span>{{ currentPath }}</span></footer>
          </div>
        </div>
        <div v-if="dragActive" class="drop-overlay">
          <FileUp />
          <strong>{{ t("upload") }}<template v-if="sftpDragDropTarget"> · {{ sftpDragDropTarget }}</template></strong>
        </div>
        </template>
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

    <Dialog :open="operationDialog === 'mkdir'" @update:open="(open) => { if (!open) operationDialog = null; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("newFolder") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="operationDialog = null"><X /></button></header>
        <input v-model="operationDraft" autofocus @keydown.enter="createDirectory" />
        <footer><button @click="operationDialog = null">{{ t("cancel") }}</button><button class="primary-button" :disabled="!operationDraft.trim()" @click="createDirectory">{{ t("confirm") }}</button></footer>      </DialogContent>
    </Dialog>

    <!-- 符号链接新建/改指向（P2-6）：target 允许相对路径（symlink 语义），
         编辑模式预填当前指向；改指向后端 readlink 比对做 no-op 兜底。 -->
    <Dialog :open="symlinkDialog !== null" @update:open="(open) => { if (!open) symlinkDialog = null; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="symlinkDialog">
        <header><DialogTitle>{{ symlinkDialog.mode === "edit" ? t("symlink.editTitle", { name: symlinkDialog.name }) : t("symlink.createTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="symlinkDialog = null"><X /></button></header>
        <p v-if="symlinkDialog.mode === 'edit'" class="muted mono">{{ symlinkDialog.linkPath }}</p>
        <input v-if="symlinkDialog.mode === 'create'" v-model="symlinkDraft" autofocus :placeholder="t('symlink.namePlaceholder')" @keydown.enter="commitSymlink" />
        <input v-if="symlinkDialog.mode === 'create'" v-model="symlinkTargetDraft" class="mono" spellcheck="false" :placeholder="t('symlink.targetPlaceholder')" @keydown.enter="commitSymlink" />
        <input v-else v-model="symlinkDraft" class="mono" spellcheck="false" autofocus :placeholder="t('symlink.targetPlaceholder')" @keydown.enter="commitSymlink" />
        <p class="muted">{{ t("symlink.hint") }}</p>
        <footer><button @click="symlinkDialog = null">{{ t("cancel") }}</button><button class="primary-button" :disabled="(symlinkDialog.mode === 'create' ? !symlinkDraft.trim() || !symlinkTargetDraft.trim() : !symlinkDraft.trim()) || symlinkSubmitting" @click="commitSymlink"><Loader2 v-if="symlinkSubmitting" class="spinning" />{{ t("confirm") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 外部编辑器保存回传确认（P2-5；M15 起逐文件排队）：队列头逐个弹，
    决议（上传一次 / 总是上传（记住 watchId）/ 取消）才出队，多文件互不顶替。 -->
    <Dialog :open="watchModifiedQueue.length > 0" @update:open="(open) => { if (!open) dismissWatchModified(); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpEdit.modifiedTitle") }}</DialogTitle><Loader2 v-if="externalEditBusy" class="spinning" /><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="dismissWatchModified"><X /></button></header>
        <p class="sftp-dialog-hint">{{ t("sftpEdit.modifiedMessage", { name: watchModifiedPrompt?.name || "" }) }}</p>
        <footer>
          <button @click="dismissWatchModified">{{ t("cancel") }}</button>
          <button @click="uploadWatchedFileAlways">{{ t("sftpEdit.alwaysUpload") }}</button>
          <button class="primary-button" @click="uploadWatchedFileOnce">{{ t("sftpEdit.uploadOnce") }}</button>
        </footer>
      </DialogContent>
    </Dialog>

    <!-- 自定义编辑器命令（编辑器映射增强）：name/command 记入 pluginStore
         单键配置，「记住为默认」写 {pattern → customId} 关联；确定即用该命令
         打开本次文件。命令串由 sidecar 分词成 argv 直接 spawn（不过 shell）。 -->
    <Dialog :open="customEditorOpen" @update:open="(open) => { if (!open) customEditorOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpEdit.customCommandTitle") }}</DialogTitle><button :title="t('close')" class="icon-button" @click="customEditorOpen = false"><X /></button></header>
        <input v-model="customEditorName" :placeholder="t('sftpEdit.customEditorNamePlaceholder')" />
        <input v-model="customEditorCommand" class="mono" spellcheck="false" autofocus :placeholder="t('sftpEdit.customEditorCommandPlaceholder')" />
        <p class="muted">{{ t("sftpEdit.customEditorCommandHint") }}</p>
        <label class="settings-switch-row">
          <Switch v-model="customEditorRemember" size="sm" />
          <span>{{ t("sftpEdit.rememberAssociation", { pattern: customEditorPattern }) }}</span>
        </label>
        <footer><button @click="customEditorOpen = false">{{ t("cancel") }}</button><button class="primary-button" :disabled="!customEditorCommand.trim()" @click="commitCustomEditor">{{ t("sftpEdit.customCommandRun") }}</button></footer>
      </DialogContent>
    </Dialog>

    <Dialog :open="commandOpen" @update:open="(open) => { if (!open) commandOpen = false; }">
      <DialogContent class="modal command-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("commandTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="commandOpen = false"><X /></button></header>
        <textarea
          v-model="commandDraft"
          class="mono"
          rows="5"
          spellcheck="false"
          autofocus
          :placeholder="t('commandPlaceholder')"
          :disabled="commandRunning"
          @keydown="handleCommandInputKeydown"
        />
        <div v-if="commandHistory.length" class="command-history">
          <div class="command-history-header">
            <span>{{ t("commandHistoryTitle") }}</span>
            <button class="link-button" @click="clearCommandHistory">{{ t("commandHistoryClear") }}</button>
          </div>
          <div class="command-history-list">
            <button
              v-for="item in commandHistory"
              :key="item"
              class="command-history-item mono"
              :title="t('commandHistoryResend')" :aria-label="t('commandHistoryResend')"
              @click="rerunHistoryCommand(item)"
            >{{ item }}</button>
          </div>
        </div>
        <label class="quick-sudo-control" :title="t('quickSudoHint')">
          <Switch v-model="commandUseSudo" size="sm" :disabled="commandRunning" />
          <span>{{ t("quickSudo") }}</span>
        </label>
        <div v-if="commandRunning" class="command-output"><Loader2 class="spinning" /><span>{{ t("commandRunning") }}</span></div>
        <pre v-else-if="commandResult" class="command-output mono">{{ commandOutputText || t("commandNoOutput") }}<span class="command-exit">exit {{ commandResult.exitCode }}</span></pre>
        <p v-if="commandError" class="task-error">{{ commandError }}</p>
        <footer>
          <button @click="commandOpen = false">{{ t("close") }}</button>
          <button v-if="commandRunning" @click="cancelCommand"><X />{{ t("commandCancel") }}</button>
          <button class="primary-button" :disabled="!commandDraft.trim() || commandRunning" @click="runCommand">
            <Loader2 v-if="commandRunning" class="spinning" />
            <SquareTerminal v-else />
            {{ t("commandRun") }}
          </button>
        </footer>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!chmodTarget" @update:open="(open) => { if (!open) chmodTarget = undefined; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="chmodTarget">
        <header><DialogTitle>{{ t("permissionsEdit") }} · {{ chmodTarget.name }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="chmodTarget = undefined"><X /></button></header>
        <div class="perm-matrix" role="group" :aria-label="t('permissionsEdit')">
          <span></span>
          <span v-for="column in PERM_COLUMNS" :key="column.bit" class="perm-matrix-head">{{ t(column.key) }}</span>
          <template v-for="role in PERM_ROLES" :key="role.who">
            <span class="perm-matrix-role">{{ t(role.key) }}</span>
            <label v-for="column in PERM_COLUMNS" :key="column.bit" class="perm-matrix-cell">
              <input type="checkbox" :checked="permBit(chmodDraft, role.who, column.bit)" @change="toggleChmodPerm(role.who, column.bit, $event)" />
            </label>
          </template>
        </div>
        <input v-model="chmodDraft" class="mono" spellcheck="false" :placeholder="t('permissionsPlaceholder')" @keydown.enter="confirmChmod" />
        <p class="muted">{{ t("permissionsHint") }}</p>
        <footer><button @click="chmodTarget = undefined">{{ t("cancel") }}</button><button class="primary-button" :disabled="!chmodDraft.trim() || chmodSubmitting" @click="confirmChmod">{{ t("confirm") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!deleteTarget" @update:open="(open) => { if (!open) deleteTarget = undefined; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="deleteTarget">
        <header><DialogTitle>{{ t("deleteTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="deleteTarget = undefined"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ deleteTarget.name }}</strong><p class="muted">{{ t("deleteMessage") }}</p></div></div>
        <footer><button @click="deleteTarget = undefined">{{ t("cancel") }}</button><button class="danger-button" :disabled="deleteSubmitting" @click="confirmDelete"><Trash2 />{{ t("delete") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <Dialog :open="batchDeleteOpen" @update:open="(open) => { if (!open) batchDeleteOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpBatch.deleteTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="batchDeleteOpen = false"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("sftpBatch.selected", { count: selectedEntries.length }) }}</strong><p class="muted">{{ t("sftpBatch.deleteMessage") }}</p></div></div>
        <div v-if="batchProgress" class="batch-progress-row"><progress class="batch-progress-bar" :value="batchProgressPercent(batchProgress)" max="100" /><span class="batch-progress mono">{{ t("sftpBatch.progress", { done: batchProgress.done, total: batchProgress.total }) }}</span></div>
        <footer><button @click="batchDeleteOpen = false" :disabled="batchDeleteSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="batchDeleteSubmitting" @click="confirmBatchDelete"><Loader2 v-if="batchDeleteSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
      </DialogContent>
    </Dialog>

    <!-- 录制删除确认：应用内弹窗替代 window.confirm（宿主沙箱 iframe 无 allow-modals，confirm 恒 false） -->
    <!-- 下载/导出前的保存目录选择（设置里开启「每次询问」时出现） -->
    <Dialog :open="!!downloadPrompt" @update:open="(open) => { if (!open) resolveDownloadPrompt(undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="downloadPrompt">
        <header><DialogTitle>{{ t("downloadSettings.askTitle") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="resolveDownloadPrompt(undefined)"><X /></button></header>
        <p class="muted mono">{{ downloadPrompt.fileName }}</p>
        <label class="settings-field">
          <span>{{ t("downloadSettings.directory") }}</span>
          <span class="settings-dir-row">
            <input v-model="downloadPrompt.dir" class="mono" spellcheck="false" autofocus @keydown.enter="resolveDownloadPrompt({ dir: downloadPrompt.dir, setDefault: downloadPrompt.setDefault })" />
            <button v-if="localCanSave" type="button" class="browse-button" :title="t('downloadSettings.browse')" :aria-label="t('downloadSettings.browse')" @click="folderPickerTarget = 'prompt'"><FolderOpen /></button>
          </span>
        </label>
        <label class="settings-field settings-switch-row">
          <input v-model="downloadPrompt.setDefault" type="checkbox" />
          <span>{{ t("downloadSettings.setDefaultThisTime") }}</span>
        </label>
        <p class="muted settings-note">{{ t("downloadSettings.askHint") }}</p>
        <footer>
          <button @click="resolveDownloadPrompt(undefined)">{{ t("cancel") }}</button>
          <button class="primary-button" @click="resolveDownloadPrompt({ dir: downloadPrompt.dir, setDefault: downloadPrompt.setDefault })">{{ t("save") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 「询问我」冲突策略：目标目录已有同名文件时的选择 -->
    <Dialog :open="!!downloadConflictPrompt" @update:open="(open) => { if (!open) resolveDownloadConflict(undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="downloadConflictPrompt">
        <header><DialogTitle>{{ t("downloadConflict.title") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="resolveDownloadConflict(undefined)"><X /></button></header>
        <p>{{ t("downloadConflict.message", { name: downloadConflictPrompt.fileName }) }}</p>
        <p class="muted mono">{{ downloadConflictPrompt.path }}</p>
        <footer>
          <button @click="resolveDownloadConflict(undefined)">{{ t("cancel") }}</button>
          <button class="danger-button" @click="resolveDownloadConflict('overwrite')">{{ t("downloadSettings.conflict.overwrite") }}</button>
          <button class="primary-button" @click="resolveDownloadConflict('rename')">{{ t("downloadSettings.conflict.rename") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 命令会话（WT-4，WezTerm spawn 对标）：输入命令后在已认证 transport 的新 channel 上执行 -->
    <Dialog :open="spawnSessionDialogOpen" @update:open="(open) => { if (!open) spawnSessionDialogOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("spawnSessionDialog.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="spawnSessionDialogOpen = false"><X /></button></header>
        <p class="muted">{{ t("spawnSessionDialog.hint") }}</p>
        <input
          v-model="spawnSessionCommand"
          autofocus
          spellcheck="false"
          autocomplete="off"
          :placeholder="t('spawnSessionDialog.placeholder')"
          @keydown.enter="confirmCommandSessionTab"
        />
        <footer>
          <button @click="spawnSessionDialogOpen = false">{{ t("cancel") }}</button>
          <button class="primary-button" :disabled="!spawnSessionCommand.trim()" @click="confirmCommandSessionTab">{{ t("confirm") }}</button>
        </footer>
      </DialogContent>
    </Dialog>

    <!-- 上传重复目标「询问我」（P1-5）：重命名 / 覆盖 / 取消，支持应用到本批次 -->
    <Dialog :open="!!uploadDuplicatePrompt" @update:open="(open) => { if (!open) resolveUploadDuplicate(undefined); }">      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="uploadDuplicatePrompt">
        <header><DialogTitle>{{ t("transferCfg.duplicateTitle") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="resolveUploadDuplicate(undefined)"><X /></button></header>
        <p>{{ t("transferCfg.duplicateMessage", { name: uploadDuplicatePrompt.fileName }) }}</p>
        <p class="muted mono">{{ uploadDuplicatePrompt.path }}</p>
        <label class="settings-field settings-switch-row">
          <input v-model="uploadDuplicateApplyAll" type="checkbox" />
          <span>{{ t("transferCfg.applyAll") }}</span>
        </label>
        <footer>
          <button @click="resolveUploadDuplicate(undefined)">{{ t("cancel") }}</button>
          <button class="danger-button" @click="resolveUploadDuplicate('overwrite')">{{ t("transferCfg.policy.overwrite") }}</button>
          <button class="primary-button" @click="resolveUploadDuplicate('rename')">{{ t("transferCfg.policy.rename") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

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

    <Dialog :open="newFileDialog" @update:open="(open) => { if (!open) newFileDialog = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpNewFile.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="newFileDialog = false"><X /></button></header>
        <input v-model="newFileDraft" autofocus spellcheck="false" :placeholder="t('sftpNewFile.placeholder')" @keydown.enter="createNewFile" />
        <footer><button @click="newFileDialog = false">{{ t("cancel") }}</button><button class="primary-button" :disabled="!newFileDraft.trim() || newFileSubmitting" @click="createNewFile"><Loader2 v-if="newFileSubmitting" class="spinning" />{{ t("confirm") }}</button></footer>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!attrsTarget" @update:open="(open) => { if (!open) closeAttributes(); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="attrsTarget">
        <header><DialogTitle>{{ t("sftpAttrs.title") }} · {{ attrsTarget.name }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="closeAttributes"><X /></button></header>
        <div v-if="attrsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <template v-else-if="attrsInfo">
          <dl class="attrs-grid">
            <dt>{{ t("sftpAttrs.path") }}</dt><dd class="mono">{{ attrsInfo.path }}</dd>
            <dt>{{ t("sftpAttrs.type") }}</dt><dd>{{ t(`sftpAttrs.kind.${attrsInfo.kind}`) }}</dd>
            <dt>{{ t("size") }}</dt><dd class="numeric">{{ attrsInfo.kind === "directory" ? "–" : formatBytes(attrsInfo.size || 0) }}</dd>
            <dt>{{ t("sftpAttrs.permissions") }}</dt><dd class="mono">{{ attrsInfo.mode || "–" }}</dd>
            <dt>{{ t("sftpAttrs.owner") }}</dt><dd>{{ attrsOwnerDisplay }}</dd>
            <dt>{{ t("sftpAttrs.modified") }}</dt><dd>{{ formatModified(attrsInfo.modifiedAt) || "–" }}</dd>
          </dl>
          <div class="perm-matrix" role="group" :aria-label="t('sftpAttrs.permissions')">
            <span></span>
            <span v-for="column in PERM_COLUMNS" :key="column.bit" class="perm-matrix-head">{{ t(column.key) }}</span>
            <template v-for="role in PERM_ROLES" :key="role.who">
              <span class="perm-matrix-role">{{ t(role.key) }}</span>
              <label v-for="column in PERM_COLUMNS" :key="column.bit" class="perm-matrix-cell">
                <input type="checkbox" :disabled="!canWrite" :checked="permBit(attrsMode, role.who, column.bit)" @change="toggleAttrsPerm(role.who, column.bit, $event)" />
              </label>
            </template>
          </div>
          <label class="attrs-permissions-edit">
            <span>{{ t("sftpAttrs.permissions") }}</span>
            <input v-model="attrsMode" class="mono" spellcheck="false" :placeholder="t('permissionsPlaceholder')" :disabled="!canWrite" @keydown.enter="saveAttributesPermissions" />
          </label>
          <p class="muted">{{ t("permissionsHint") }}</p>
        </template>
        <footer>
          <button @click="closeAttributes">{{ t("close") }}</button>
          <button class="primary-button" :disabled="!canWrite || !attrsMode.trim() || attrsSubmitting" @click="saveAttributesPermissions"><Loader2 v-if="attrsSubmitting" class="spinning" />{{ t("sftpAttrs.save") }}</button>
        </footer>
        </template>
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
    <!-- 安全弹窗：不允许 Esc / 点击遮罩关闭，必须显式信任或拒绝（不在 Esc 链中） -->
    <Dialog :open="!!hostKeyPrompt">
      <DialogContent class="modal host-key-modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="hostKeyPrompt">
        <header><DialogTitle>{{ t("hostKeyDialog.title") }}</DialogTitle></header>
        <p>{{ t("hostKeyDialog.desc") }}</p>
        <dl><dt>{{ t("hostKeyDialog.server") }}</dt><dd>{{ hostKeyPrompt.host }}:{{ hostKeyPrompt.port }}</dd><dt>{{ t("hostKeyDialog.keyType") }}</dt><dd>{{ hostKeyPrompt.keyType }}</dd><dt>{{ t("hostKeyDialog.fingerprint") }}</dt><dd class="fingerprint">{{ hostKeyPrompt.fingerprint }}</dd></dl>
        <label class="remember"><input v-model="rememberHostKey" type="checkbox" /> {{ t("hostKeyDialog.remember") }}</label>
        <footer><button @click="resolveHostKey(false)">{{ t("hostKeyDialog.reject") }}</button><button class="primary-button" @click="resolveHostKey(true)">{{ t("hostKeyDialog.trust") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- RDP 证书确认（rdp-certificate challenge）：安全弹窗，不允许 Esc / 点击
         遮罩关闭；SHA256 指纹 + knownHostStatus 徽标 + 120s 倒计时 + remember，
         超时 fail-closed（应答与 sidecar 超时同样按拒绝处理）。 -->
    <Dialog :open="!!rdpCertPrompt">
      <DialogContent class="modal host-key-modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="rdpCertPrompt">
        <header><DialogTitle>{{ t("rdp.cert.title") }}</DialogTitle></header>
        <p>{{ t("rdp.cert.desc") }}</p>
        <dl>
          <dt>{{ t("rdp.cert.server") }}</dt><dd>{{ rdpCertPrompt.host }}:{{ rdpCertPrompt.port }}</dd>
          <dt>{{ t("rdp.cert.fingerprint") }}</dt><dd class="fingerprint">{{ rdpCertPrompt.fingerprint }}</dd>
        </dl>
        <p class="muted rdp-cert-status">{{ t(`rdp.cert.status.${rdpCertStatusKey(rdpCertPrompt.knownHostStatus)}`) }}</p>
        <label class="remember"><input v-model="rdpCertRemember" type="checkbox" /> {{ t("rdp.cert.remember") }}</label>
        <p class="muted rdp-cert-expires">{{ t("rdp.cert.expires", { seconds: rdpCertRemaining }) }}</p>
        <footer><button @click="resolveRdpCertificate(false)">{{ t("rdp.cert.reject") }}</button><button class="primary-button" @click="resolveRdpCertificate(true)">{{ t("rdp.cert.accept") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 安全弹窗：不允许 Esc / 点击遮罩关闭，必须显式批准或拒绝（不在 Esc 链中） -->
    <Dialog :open="!!agentPromptHead">
      <DialogContent class="modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="agentPromptHead">
        <header><DialogTitle>{{ agentPromptHead.source === "mcp" ? t("agentPrompt.mcpSource", { tool: agentPromptHead.tool }) : t("agentPromptTitle") }}</DialogTitle></header>
        <div class="agent-prompt-meta">
          <span>{{ t("agentPromptSource") }} <code class="mono">{{ agentPromptHead.tool }}</code></span>
          <span class="agent-risk-badge" :class="agentPromptHead.risk === 'elevated' ? 'elevated' : 'low'">{{ agentPromptHead.risk === "elevated" ? t("agentPromptRiskElevated") : t("agentPromptRiskLow") }}</span>
        </div>
        <label class="agent-prompt-command">
          <span>{{ t("agentPromptCommandLabel") }}</span>
          <textarea v-model="agentPromptCommand" class="mono" rows="3" spellcheck="false" />
        </label>
        <!-- 记住不限风险档：strict 模式下低危命令同样每次弹审、同样需要免审
             记忆（IMPL_PLAN 预期 strict/auto 下 approve+remember 二次零弹窗）；
             破坏性命令由后端 D2 兜底忽略 remember。 -->
        <label class="agent-prompt-remember">
          <input v-model="agentPromptRemember" type="checkbox" />
          <span>{{ t("approval.remember") }}</span>
        </label>
        <p class="muted agent-prompt-countdown">{{ t("agentPromptTimeoutHint", { seconds: Math.ceil(agentPromptRemaining) }) }}</p>
        <footer><button @click="resolveAgentPrompt('deny')">{{ t("agentPromptDeny") }}</button><button class="primary-button" @click="resolveAgentPrompt('approve')">{{ t("agentPromptApprove") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 告警排查：异构告警 → 结构化 + 分类 + 只读诊断命令清单 -->
    <Dialog :open="alertTriageOpen" @update:open="(open) => { if (!open) alertTriageOpen = false; }">
      <DialogContent class="modal alert-triage-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("alertTriage.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="alertTriageOpen = false"><X /></button>
        </header>
        <p class="muted alert-triage-hint">{{ t("alertTriage.hint") }}</p>
        <textarea v-model="alertTriagePayload" class="mono alert-triage-payload" rows="6" :placeholder="t('alertTriage.placeholder')" :disabled="alertTriageBusy" spellcheck="false" autofocus />
        <p v-if="alertTriageError" class="task-error" role="alert">{{ alertTriageError }}</p>
        <footer class="alert-triage-actions">
          <button class="primary-button" :disabled="alertTriageBusy || !sanitizeTriagePayload(alertTriagePayload)" @click="runAlertTriage">{{ t("alertTriage.analyze") }}</button>
        </footer>
        <div v-if="alertTriageBusy" class="empty compact" role="status"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="!alertTriageResult && !alertTriageError" class="empty compact">{{ t("alertTriage.emptyResult") }}</div>
        <div v-else-if="alertTriageResult" class="alert-triage-result">
          <div class="alert-triage-summary">
            <span class="alert-severity-badge" :class="severityClass(alertTriageResult.normalized.severity)">{{ t(`alertTriage.severity.${severityClass(alertTriageResult.normalized.severity)}`) }}</span>
            <span class="alert-category">{{ t(`alertTriage.category.${alertTriageResult.category}`) }}</span>
            <strong v-if="alertTriageResult.normalized.title" class="alert-title">{{ alertTriageResult.normalized.title }}</strong>
          </div>
          <p v-if="alertTriageResult.normalized.message" class="muted alert-message mono">{{ alertTriageResult.normalized.message }}</p>
          <p v-if="!alertTriageResult.suggestions.length" class="muted">{{ t("alertTriage.emptyResult") }}</p>
          <ul v-else class="alert-suggestion-list">
            <li v-for="suggestion in alertTriageResult.suggestions" :key="suggestion.command" class="alert-suggestion-row">
              <code class="mono alert-suggestion-command">{{ suggestion.command }}</code>
              <span class="alert-purpose muted">{{ purposeKeyLabel(suggestion.purposeKey, t) }}</span>
              <button :disabled="!session" :title="!session ? t('alertTriage.noSession') : ''" :aria-label="!session ? t('alertTriage.noSession') : ''" @click="sendSuggestionToTerminal(suggestion.command)">{{ t("alertTriage.sendToTerminal") }}</button>
            </li>
          </ul>
          <footer v-if="alertTriageResult.suggestions.length" class="alert-triage-actions">
            <button @click="copySuggestions">{{ t("alertTriage.copyAll") }}</button>
          </footer>
        </div>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!pasteConfirm" @update:open="(open) => { if (!open) resolvePasteConfirm(false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="pasteConfirm">
        <header>
          <DialogTitle>{{ pasteConfirm.danger ? t("terminalDanger.title") : t("terminalPasteConfirm.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="resolvePasteConfirm(false)"><X /></button>
        </header>
        <div v-if="pasteConfirm.danger" class="destructive-copy warning">
          <span class="destructive-icon"><TriangleAlert /></span>
          <div>
            <strong>{{ t("terminalDanger.detected") }}</strong>
            <p class="task-error mono">{{ pasteConfirm.hits.map((hit) => hit.label).join(" · ") }}</p>
            <p class="muted">{{ t("terminalDanger.desc") }}</p>
          </div>
        </div>
        <p v-else class="muted">{{ t("terminalPasteConfirm.desc") }}</p>
        <p class="muted">{{ t("terminalPasteConfirm.summary", { lines: pasteConfirm.lines, chars: pasteConfirm.chars }) }}<template v-if="pasteConfirm.lines > 1"> · {{ t("terminalPasteConfirm.multiline") }}</template></p>
        <pre class="command-output mono">{{ pasteConfirm.preview }}</pre>
        <footer>
          <button @click="resolvePasteConfirm(false)">{{ t("cancel") }}</button>
          <button :class="pasteConfirm.danger ? 'danger-button' : 'primary-button'" @click="resolvePasteConfirm(true)">{{ t("terminalPasteConfirm.confirm") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- SSH-H1：通用应用内确认弹窗（宿主沙箱 iframe 无 allow-modals，window.confirm
         恒 false）——预览关闭/大文件预览/重命名覆盖/粘贴覆盖/网页大文件下载/进程强杀
         此前在真机上全部静默失效，现统一走本弹窗（Promise 原语同 pasteConfirm）。 -->
    <Dialog :open="!!pendingConfirmDialog" @update:open="(open) => { if (!open) resolvePendingConfirmDialog(false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="pendingConfirmDialog">
        <header>
          <DialogTitle>{{ t("confirm") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="resolvePendingConfirmDialog(false)"><X /></button>
        </header>
        <p class="muted confirm-message">{{ pendingConfirmDialog.message }}</p>
        <footer>
          <button @click="resolvePendingConfirmDialog(false)">{{ t("cancel") }}</button>
          <button :class="pendingConfirmDialog.danger ? 'danger-button' : 'primary-button'" @click="resolvePendingConfirmDialog(true)">{{ t("confirm") }}</button>
        </footer>
        </template>
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

    <!-- 本地终端确认：SSH 会话仍连着时先关闭再进入本地模式 -->
    <Dialog :open="localOpenConfirmOpen" @update:open="(open) => { if (!open) localOpenConfirmOpen = false; }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("localTerminal.openConfirmTitle") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="localOpenConfirmOpen = false"><X /></button>
        </header>
        <p class="muted">{{ t("localTerminal.openConfirm") }}</p>
        <footer>
          <button @click="localOpenConfirmOpen = false">{{ t("cancel") }}</button>
          <button class="primary-button" @click="confirmLocalTerminal">{{ t("localTerminal.open") }}</button>
        </footer>
      </DialogContent>
    </Dialog>

    <input ref="uploadInput" class="hidden" type="file" multiple @change="onUploadInput" />
    <!-- webkitdirectory 会自动携带多选语义；目录选择以相对路径（webkitRelativePath）回传。 -->
    <input v-if="folderUploadSupported" ref="folderUploadInput" class="hidden" type="file" multiple webkitdirectory @change="onFolderUploadInput" />
    <input ref="zmodemInput" class="hidden" type="file" multiple @change="onZmodemInput" />
    <input ref="trzszInput" class="hidden" type="file" multiple @change="onTrzszPickInput" @cancel="onTrzszPickCancel" />
  </main>
</template>

<style scoped src="./styles/app.css"></style>

