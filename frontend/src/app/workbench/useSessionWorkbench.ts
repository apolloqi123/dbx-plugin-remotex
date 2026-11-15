import { pick } from "./lib/pick";
import type { WorkbenchApi } from "./useWorkbench";

/** Bindings owned by the sessions domain, in declaration order. */
const SESSIONS_KEYS = [
  "activeExternalWatches", "addLocalShellEnvRow", "agentMode", "agentModeBusy", "agentModeHint",
  "agentModeOpen", "agentPromptCommand", "agentPromptExpired", "agentPromptHead", "agentPromptQueue",
  "agentPromptRemaining", "agentPromptRemember", "agentRunning",
  "applyRdpCustomResolution", "beginRecordCountdown",
  "canOpenLocalTab", "canOpenShellTab", "canSplitTerminal", "canWrite", "cancelSerialUploadForSessionClose",
  "closeLocalTerminal", "closeRdpSession", "closeSerialSession", "closeSession", "closeTelnetSession",
  "confirmLocalTerminal", "confirmRecordingClearAll", "connectCancelled", "connectCardState", "connectLog",
  "connectLogsOpen", "connectSucceeded", "connectViaRdpDialog", "connected", "connection",
  "connectionAuthMethod", "connectionAuthMethodLabel", "connectionId", "connectionIdentity", "connectionInfoOpen",
  "connectionLatency", "connectionLatencyBusy", "connectionLatencyFailed", "connectionProtocol", "connectionReadOnly",
  "deleteRecording", "dismissRestoredLocalShell", "drainSerialFrames", "exportRecordingFromList",
  "exportRecordingGif", "formatRecordedAt", "handleVncFrame", "isLocalMode", "isRdpConnection",
  "isRdpMode", "isRdpResolutionActive", "isSerialMode", "isTelnetMode", "isVncConnection", "isVncMode",
  "loadRecordings", "refreshRecordings", "localExitCode", "localLastCwd", "localLastSequence", "localOpenConfirmOpen",
  "localPendingFrames", "localRecentCommands", "localReplayInFlight", "localReplayNoProgress", "localSession",
  "localShellCwdPref", "localShellEnvClean", "localShellEnvPref", "localShellIntegrationPref", "localShellPref",
  "localShellRestored", "localShellSurfaceOpen", "localShells", "localShellsLoading", "localState", "onVncSurfaceResize",
  "localUiMode", "locale", "markLocalExited", "markTelnetClosed", "onPaneActivated",
  "onShellTabCwd", "openConnectionSurface", "openLocalShellPrefs", "openReplay", "openSession",
  "openSessionShellPick", "openSessionShellPrefs", "paneContainer", "workbenchSplitRef", "paneLayout", "paneOrder",
  "paneSplitStyle", "panelCpuPercent", "panelMetricsFailed", "panelNetRates", "panelSchemeChoices",
  "panelSurface", "pickLocalShell", "rdpCertPrompt", "rdpCertRemaining", "rdpCertRemember",
  "rdpClipboardChunks", "rdpClosedTitle", "rdpCustomHeight", "rdpCustomWidth", "rdpDesktopSize",
  "rdpDialogOpen", "rdpDialogSeed", "rdpAutoFit", "enableRdpAutoFit", "onRdpSurfaceResize", "rdpPasteOpen", "rdpPasteText", "rdpPersistedSize", "rdpResizing", "rdpFileHintOpen",
  "rdpResolutionLabel", "rdpResolutionOpen", "rdpScaleMode", "rdpSession", "rdpState",
  "rdpSurface", "rdpTarget", "rdpTypeAsKeystrokes", "rdpTypingBusy", "rdpTypingSent",
  "rdpTypingTotal", "rdpWindowProportionPreset", "rdpWriteRemoteClipboard", "reattachLocalSession", "reconnect",
  "pasteRdpClipboard", "pasteVncClipboard", "receiveRdpClipboard", "receiveRdpClipboardFiles", "receiveRdpClipboardStatus",
  "startRdpRemoteDownload", "saveRdpRemoteArchive", "dismissRdpRemoteFiles",
  "rdpClipboardEnabled", "rdpClipboardStatus", "rdpFileBusy", "rdpFileProgress", "rdpRemoteFiles", "rdpArchiveReady", "rdpRemoteReady", "rdpNativeFileClipboard", "rdpServerFileClipboard", "rdpServerFileClipboardKnown", "rdpFileError",
  "reconnectAttempt", "reconnectCountdown", "reconnectRdpSession", "reconnectVncSession", "recordingActive", "recordingClearAllOpen",
  "recordingClearAllSubmitting", "recordingDeleteSubmitting", "recordingDeleteTarget", "recordingElapsedSec", "recordingExportingId",
  "recordingHits", "recordingSearchBusy", "recordingStartedAt", "recordingTarget", "recordings",
  "recordingsLoading", "recordingsOpen", "recordingsQuery", "replayDurationMs", "replayHost",
  "replayPlayheadMs", "replayPlaying", "rerunLocalCommand", "resetTransportReuseState", "resolveHostKey",
  "resolveRdpCertificate", "restartLocalTerminal", "restored", "revealRecording",
  "runRecordingSearch", "runtimeEndpoint", "scheduleReconnect", "sendRdpClipboard", "sendVncInput",
  "serialBinaryInput", "serialDialogOpen", "serialError", "serialInputQueue", "serialLastSequence",
  "serialPendingFrames", "serialReplayInFlight", "serialReplayNoProgress", "serialSession", "serialState",
  "serialTarget", "serialUnknownStreamFrames", "serialUpload", "serialUploadBusy", "serialUploadDialogOpen",
  "serialUploadOverlayVisible", "serialUploadPercentValue", "serialUploadStatusLabel", "session", "sessionMenuOpen",
  "sessionMenuShellOpen", "sessionMenuShellPickOpen", "sessionPillText", "sessionStatus", "setLocalShellCwd",
  "setLocalShellPref", "shellTabCwd", "shellTabSessions", "startRecordingClock", "startRecordingNow",
  "startSerialUpload", "startVncFromConnection", "startVncSession", "stopReplayLoop", "syncSftpToActivePane",
  "telnetDialogOpen", "telnetError", "telnetLastSequence", "telnetPendingFrames", "telnetReplayInFlight",
  "telnetReplayNoProgress", "telnetSession", "telnetState", "telnetTarget", "toggleReplayPlay",
  "vncDesktopSize", "VNC_RESOLUTION_PRESETS", "vncAutoFit", "vncResizing", "vncCanResize", "vncResolutionOpen",
  "vncCustomWidth", "vncCustomHeight", "vncResolutionLabel", "isVncResolutionActive", "toggleVncResolutionMenu",
  "applyVncResolution", "applyVncCustomResolution", "enableVncAutoFit",
  "receiveVncClipboard", "vncDialogOpen", "vncError", "vncScaleMode", "vncSession", "vncState",
  "vncSurface", "vncTarget", "writeTerminalOutput",
] as const;

export type SessionsWorkbench = Pick<WorkbenchApi, (typeof SESSIONS_KEYS)[number]>;

/** Flat sessions-domain API: only selects bindings, never creates state. */
export function useSessionWorkbench(api: WorkbenchApi): SessionsWorkbench {
  return pick(api, SESSIONS_KEYS);
}
