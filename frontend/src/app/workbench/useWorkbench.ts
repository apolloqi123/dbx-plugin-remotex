/**
 * Workbench wiring: ordered installation of the extracted modules plus the four
 * domain aggregators (terminal / sessions / sftp / shell).
 *
 * The steps run in the exact order of the original App.vue setup body so every
 * composable keeps its registration order. Bindings that a step needs from a
 * later step are read through `late` getters (resolved after setup completes),
 * matching the TDZ-safe getter pattern the composables already use.
 */
import { installWorkbenchCore } from "./setup/step1-core";
import { installWorkbenchTerminal } from "./setup/step2-terminal";
import { installWorkbenchPrefs } from "./setup/step3-prefs";
import { installWorkbenchSessionIo } from "./setup/step4-sessions";
import { installWorkbenchAppearance } from "./setup/step5-appearance";
import { installWorkbenchConnections } from "./setup/step6-connections";
import { installWorkbenchSftp } from "./setup/step7-sftp";
import { installWorkbenchShell } from "./setup/step8-shell";
import { installWorkbenchBootstrap } from "./setup/step9-bootstrap";
import type { InstallWorkbenchCoreResult } from "./setup/step1-core";
import type { InstallWorkbenchTerminalResult } from "./setup/step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./setup/step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./setup/step4-sessions";
import type { InstallWorkbenchAppearanceResult } from "./setup/step5-appearance";
import type { InstallWorkbenchConnectionsResult } from "./setup/step6-connections";
import type { InstallWorkbenchSftpResult } from "./setup/step7-sftp";
import type { InstallWorkbenchShellResult } from "./setup/step8-shell";
import type { InstallWorkbenchBootstrapResult } from "./setup/step9-bootstrap";
import { useTerminalWorkbench } from "./useTerminalWorkbench";
import { useSessionWorkbench } from "./useSessionWorkbench";
import { useSftpWorkbench } from "./useSftpWorkbench";
import { useShellWorkbench } from "./useShellWorkbench";

export type WorkbenchApi = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & InstallWorkbenchAppearanceResult & InstallWorkbenchConnectionsResult & InstallWorkbenchSftpResult & InstallWorkbenchShellResult & InstallWorkbenchBootstrapResult;

export function useWorkbench() {
  // Late-binding store: only read after every step has run (all setup-time sinks
  // of these getters are closures the composables invoke later).
  const values: Record<string, unknown> = {};
  const late = (keys: readonly string[]): Record<string, any> => {
    const view: Record<string, any> = {};
    for (const key of keys) {
      Object.defineProperty(view, key, { get: () => values[key], enumerable: true, configurable: true });
    }
    return view;
  };

  const step1 = installWorkbenchCore({ late: late(["commandMarker", "commandRunning", "completionOpen", "dismissPromptHints", "dropUploadPathInputEl", "historyPanelOpen", "isLocalMode", "localShellPref", "localShells", "pathBarInputEl", "quickSelectOpen", "readTerminalSuggestionAnchor", "replaceTerminalLineWith", "scheduleFit", "searchAddon", "sniffTerminalShell", "suggestionOpen", "suggestionViewport", "t", "terminalMenuOpen", "terminalTransferBusy"]) });
  Object.assign(values, step1);
  const api1 = { ...step1 };

  const step2 = installWorkbenchTerminal({ ...api1, late: late(["actionLinksEnabled", "activePaneCwd", "attachActionLinks", "attachGutterListeners", "attachHighlightRender", "batchQuickOpen", "clearRowSelection", "clearSftpSearch", "clipboardDeps", "closeMetrics", "closeToolbarPopovers", "commandMarker", "completionController", "connectionInfoOpen", "copyTerminalSelection", "copyTextToClipboard", "directoryParser", "dragActive", "feedTrzszTerminalInput", "handleEditorRename", "handleTerminalBell", "handleTerminalKey", "handleTerminalMouseDown", "handleTerminalMouseUp", "handleTerminalWheel", "hideGhostSuggestion", "highlightEnabled", "hostTerminalFontFamily", "hydrateLinkTargets", "interceptTerminalPaste", "isGutterActive", "isLocalMode", "isTelnetMode", "linkTargets", "localSession", "localUiMode", "markLocalExited", "measureLatency", "metrics", "openEditorTab", "openTerminalLink", "panelPaletteOpen", "pasteTerminal", "pathHistoryOpen", "persistState", "promptHintsLineEmpty", "rdpPasteOpen", "rdpResolutionOpen", "reconnectPending", "refreshConnectionAuthMethod", "refreshDiskUsage", "refreshGhostAfterInput", "refreshMetrics", "refreshResumableUploads", "refreshSftpAfterUpload", "refreshSuggestionsAfterInput", "refreshTransferHistory", "registerOscColorQueryHandlers", "rememberPathHistory", "resolveUploadDuplicateName", "scheduleReconnect", "selectFile", "serialBinaryInput", "serialSession", "serialUploadBusy", "setTrzszTerminalColumns", "sftpFiltersActive", "syncGhostAnchorOnSettle", "syncSuggestionAnchorsOnSettle", "t", "telnetSession", "terminalTheme", "terminalTransferBusy", "terminalTransportWatch", "trzszBusy", "wallpaperActive", "writeU64", "zmodemBusy"]) });
  Object.assign(values, step2);
  const api2 = { ...api1, ...step2 };

  const step3 = installWorkbenchPrefs({ ...api2, late: late(["actionLinksSettings", "batchBarOpen", "closeRdpSession", "closeRightPanel", "closeSession", "closeVncSession", "ctxSearchEnginesText", "dismissPromptHints", "ghostEnabled", "gutterSettings", "hideGhostSuggestion", "interactivePromptGatePending", "isVncMode", "localLastSequence", "localPendingFrames", "localReplayNoProgress", "persistState", "persistTerminalFeaturePrefs", "promptHintsLineEmpty", "quickSelectOpen", "reconnectPending", "requestLocalReplay", "resetCommandMarker", "sendConfirmedPaste", "serialLastSequence", "serialPendingFrames", "serialUnknownStreamFrames", "t", "telnetLastSequence", "telnetPendingFrames", "telnetReplayNoProgress", "terminalTransferBusy", "updateTerminalAppearance"]) });
  Object.assign(values, step3);
  const api3 = { ...api2, ...step3 };

  const step4 = installWorkbenchSessionIo({ ...api3, late: late(["actionLinksEnabled", "batchBarOpen", "closeRightPanel", "commandResult", "connectionAuthMethod", "consumeFrameViaZmodem", "drainLocalTerminalFrames", "drainSerialFrames", "drainTelnetFrames", "ensureTrzszFilter", "handleTrzszAnnounce", "hideGhostSuggestion", "hostTerminalFontFamily", "localLastSequence", "localPendingFrames", "markTerminalOutputActive", "reconnectPending", "rescanHighlightViewport", "resetGhostSuggestion", "resetGutterTimestamps", "resetPromptHintsQuiet", "scheduleActionLinkScan", "scheduleGutterRecompute", "scheduleReconnect", "serialLastSequence", "serialPendingFrames", "serialUnknownStreamFrames", "stampGutterWrittenRows", "syncGhostAnchorOnSettle", "telnetLastSequence", "telnetPendingFrames", "terminalTransportWatch", "trzszBusy", "zmodemBusy"]) });
  Object.assign(values, step4);
  const api4 = { ...api3, ...step4 };

  const step5 = installWorkbenchAppearance({ ...api4, late: late(["dismissPromptHints", "interactivePromptGatePending", "probeLocalCapabilities", "promptHintsVisible", "revealTransferTarget", "saveHostFile"]) });
  Object.assign(values, step5);
  const api5 = { ...api4, ...step5 };

  const step6 = installWorkbenchConnections({ ...api5, late: late(["activeExternalWatches", "cancelRecordCountdown", "closeToolbarPopovers", "probeLocalCapabilities", "recordingActive", "stopRecordingClock", "watchModifiedQueue"]) });
  Object.assign(values, step6);
  const api6 = { ...api5, ...step6 };

  const step7 = installWorkbenchSftp({ ...api6, late: late(["closeMetrics", "closeToolbarPopovers", "metricsOpen", "openMetrics", "openTransferPanel"]) });
  Object.assign(values, step7);
  const api7 = { ...api6, ...step7 };

  const step8 = installWorkbenchShell({ ...api7, late: late([]) });
  Object.assign(values, step8);
  const api8 = { ...api7, ...step8 };

  const step9 = installWorkbenchBootstrap({ ...api8, late: late([]) });
  Object.assign(values, step9);
  const api9 = { ...api8, ...step9 };

  const api = api9;
  return {
    ctx: api.ctx,
    terminal: useTerminalWorkbench(api),
    sessions: useSessionWorkbench(api),
    sftp: useSftpWorkbench(api),
    shell: useShellWorkbench(api),
  };
}

export type Workbench = ReturnType<typeof useWorkbench>;

/** Flattens the four domain slices back into the declaration-ordered workbench API. */
export function workbenchBindings(wb: Workbench): WorkbenchApi {
  return { ...wb.shell, ...wb.terminal, ...wb.sessions, ...wb.sftp };
}

