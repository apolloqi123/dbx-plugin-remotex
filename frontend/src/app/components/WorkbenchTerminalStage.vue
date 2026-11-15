<script setup lang="ts">
// 终端舞台子树（T21b 自 App.vue 模板迁出）：WorkbenchStage 及其全部插槽内容，
// 模板逐行保持原样，绑定来自聚合 API。
import CommandSuggestions from "../../features/completion/components/CommandSuggestions.vue";
import CompletionMenu from "../../features/completion/components/CompletionMenu.vue";
import ConnectingCard from "../../features/connections/components/ConnectingCard.vue";
import RdpClipboardFilesOverlay from "./RdpClipboardFilesOverlay.vue";
import RdpSurface from "../../features/remote-desktop/components/RdpSurface.vue";
import TerminalHistoryPanel from "../../features/terminal/components/TerminalHistoryPanel.vue";
import TerminalPromptHints from "../../features/terminal/components/TerminalPromptHints.vue";
import TerminalQuickSelectPanel from "../../features/terminal/components/TerminalQuickSelectPanel.vue";
import TerminalSearchPanel from "../../features/terminal/components/TerminalSearchPanel.vue";
import TerminalStatusOverlays from "./TerminalStatusOverlays.vue";
import TransferOverlays from "./TransferOverlays.vue";
import VncSurface from "../../features/remote-desktop/components/VncSurface.vue";
import WorkbenchStage from "./WorkbenchStage.vue";
import RecordingsPanel from "../../features/recording/components/RecordingsPanel.vue";
import { Loader2, X } from "@lucide/vue";
import { formatBytes } from "../../shared/lib/format";
import { formatCommandDuration } from "../../features/terminal/lib/panels/terminalCommandMarkers";
import { initialSerialUploadState } from "../../features/serial/lib/serialUpload";
import type { WorkbenchApi } from "../workbench/useWorkbench";

const props = defineProps<{ bindings: WorkbenchApi }>();

const {
  acceptCompletionRow, actionLinkHint, agentRunning, aiBridgeOk, aiFixBar,
  aiFixBarStyle, aiFixFromBar, aiSearch, aiSearchAnchor, aiSearchBarEl,
  aiSearchBarStyle, aiSearchPending, aiSearchPendingQuery, aiSearchState, aiSettingsState,
  bindReplayHost, cancelConnect, cancelRecordCountdown, cancelSerialUpload, cancelTrzszTransfer,
  cancelZmodemUpload, clearFixRecommendation, clearRecordingsSearch, clearTerminalSearch, closeHistoryPanel,
  closeLocalTerminal, closeQuickSelect, closeRdpSession, closeReplay, closeSerialSession,
  closeTelnetSession, closeTerminalSearch, closeVncSession, commandMarker, commandMarkerDetails,
  commandMarkerElapsed, completionActiveIndex, completionAnchor, completionItems, completionLoading,
  completionOpen, connectCardState, connectLogEntries, connectLogsOpen, connected,
  connection, connectionIdentity, copyQuickSelectHit, deleteRecording, dismissPromptHints,
  dismissRestoredLocalShell, dragActive, exportRecordingFromList, exportRecordingTranscript, exportReplayGif,
  fillSuggestion, filteredRecordings, formatDuration, formatRecordedAt, ghostAcceptKeyLabel,
  ghostAnchor, ghostKeycapMenuOpen, ghostMatch, ghostTabAccept, gutterPaneStyle,
  gutterPaneVisible, gutterRows, gutterVisible, gutterWidth, handleHistoryPanelPanelKey,
  handleTerminalMiddleClick, historyPanelActiveIndex, historyPanelAnchor, historyPanelEntries, historyPanelFocusSearch,
  historyPanelOpen, historyPanelQuery, ignoreSuggestion, inactiveWaiting, interruptAgentRun,
  isLocalMode, isRdpConnection, isRdpMode, isSerialMode, isTelnetMode,
  isVncMode, localCanSave, localExitCode, localShellRestored, localState,
  localUiMode, locale, onReplaySeek, openReplay, panelSurface,
  panelMetricsVisible, panelMetricsFailed, panelCpuPercent, panelMemPercent, panelNetRates, panelMetricsErrorTitle,
  promptHintsAnchor, promptHintsCompletionsKey, promptHintsHistoryKey, promptHintsVisible, quickSelectActive,
  quickSelectHits, quickSelectOpen, rdpAutoFit, rdpClosedTitle, rdpDesktopSize, rdpDialogOpen,
  rdpScaleMode, rdpState, rdpSurface, reconnect, reconnectCountdown,
  rdpClipboardEnabled, rdpFileBusy, rdpFileProgress, rdpRemoteFiles, rdpArchiveReady, rdpRemoteReady,
  startRdpRemoteDownload, saveRdpRemoteArchive, dismissRdpRemoteFiles,
  reconnectNow, reconnectPending, reconnectRdpSession, reconnectVncSession, onRdpSurfaceResize, recordCountdown, recordingActive,
  recordingClearAllOpen, recordingClearAllSubmitting, recordingElapsedSec, recordingExportingId, recordingHits, recordingSearchBusy, recordingTarget,
  recordings, recordingsLoading, recordingsOpen, recordingsQuery, refreshRecordings, replayDurationMs,
  replayExporting, replayPlayheadMs, replayPlaying, replaySpeed, replayState, replayMotionEnabled, toggleReplayMotion,
  restartLocalTerminal, revealRecording, runTerminalSearch, searchMatchState, searchOpen,
  searchResultCount, searchResultIndex, searchSeedOptions, searchSeedQuery, selectHistoryEntry,
  sendRdpClipboard, pasteRdpClipboard, sendRdpInput, sendVncClipboard, pasteVncClipboard, pasteVncWithFileClipboard, probeVncRemoteClipboard, sendVncInput, onVncSurfaceResize, serialError,
  serialState, serialUpload, serialUploadBusy, serialUploadOverlayVisible, serialUploadPercentValue,
  serialUploadStatusLabel, setGhostTabAccept, sftpViewVisible, showTerminalMenu, stage,
  startConnect, suggestionActiveIndex, suggestionAnchor, suggestionItems, suggestionOpen,
  suggestionViewport, t, telnetError, telnetState, terminal,
  terminalBellFlash, terminalDiag, terminalDiagVisible, terminalDragActive, terminalError,
  terminalErrorDetail, terminalErrorFriendly, terminalState, terminalTransferBusy, toggleRecording,
  toggleRecordings, toggleReplayPlay, trzszBusy, trzszFileCount, trzszFileIndex,
  trzszOverlayVisible, trzszPercent, trzszPhase, trzszSpeed, trzszStatusLabel,
  updateHistoryPanelFilter, vncError, vncScaleMode, vncState, vncSurface,
  wallpaperActive, wallpaperDataUrl, wallpaperOpacity, zmodemBusy, zmodemFileCount,
  zmodemFileIndex, zmodemPercent, zmodemSpeed, zmodemState, zmodemStatusLabel,
} = props.bindings;
</script>

<template>
      <WorkbenchStage
        ref="stage"
        :t="t"
        :terminal-drag-active="terminalDragActive"
        :command-marker="commandMarker"
        :gutter-pane-visible="gutterPaneVisible"
        :wallpaper-active="wallpaperActive"
        :terminal-transfer-busy="terminalTransferBusy"
        :gutter-pane-style="gutterPaneStyle"
        :wallpaper-data-url="wallpaperDataUrl"
        :wallpaper-opacity="wallpaperOpacity"
        :gutter-visible="gutterVisible"
        :gutter-rows="gutterRows"
        :gutter-width="gutterWidth"
        :terminal-bell-flash="terminalBellFlash"
        :action-link-hint="actionLinkHint"
        :terminal-diag-visible="terminalDiagVisible"
        :terminal-diag="terminalDiag"
        :ai-fix-bar="aiFixBar"
        :ai-fix-bar-style="aiFixBarStyle"
        :is-rdp-connection="isRdpConnection"
        :local-ui-mode="localUiMode"
        :drag-active="dragActive"
        :sftp-view-visible="sftpViewVisible"
        :command-marker-details="commandMarkerDetails"
        :command-marker-elapsed="commandMarkerElapsed"
        :agent-running="agentRunning"
        :format-command-duration="formatCommandDuration"
        :panel-metrics-visible="panelMetricsVisible"
        :panel-metrics-failed="panelMetricsFailed"
        :panel-cpu-percent="panelCpuPercent"
        :panel-mem-percent="panelMemPercent"
        :panel-net-rates="panelNetRates"
        :panel-metrics-error-title="panelMetricsErrorTitle"
        @contextmenu="showTerminalMenu"
        @middle-click="handleTerminalMiddleClick"
        @dismiss-diagnostics="terminalDiagVisible = false"
        @fix-with-ai="aiFixFromBar"
        @dismiss-fix="aiFixBar = null; clearFixRecommendation()"
        @focus-terminal="terminal?.focus()"
        @interrupt-agent="interruptAgentRun"
      >
        <template #surfaces>
        <!-- VNC 画布（nyaterm-parity P2 2d）：盖在 xterm 之上（z-index 2），
             帧从 vnc/frame/{id} 二进制通道进入；键盘/鼠标由画布采集后经
             keysym 映射发 vnc/input。连接态/退出覆盖层沿用 terminal-overlay。 -->
        <VncSurface v-if="isVncMode" ref="vncSurface" class="vnc-surface" :scale-mode="vncScaleMode" @input="sendVncInput" @clipboard-out="pasteVncClipboard" @paste-request="pasteVncWithFileClipboard" @copy-request="probeVncRemoteClipboard" @container-resize="onVncSurfaceResize" />
        <!-- RDP 画布（nyaterm-parity P3-4）：盖在 xterm 之上（z-index 2），帧从
             rdp/frame/{id} 二进制通道进入（与 vnc/frame 同一 44 字节 patch 头）；
             键盘经扫描码映射、鼠标/滚轮原样映射发 rdp/input，服务端光标形状
             （rdp/pointer）落到画布 CSS cursor。连接态/退出覆盖层沿用
             terminal-overlay。 -->
        <RdpSurface v-if="isRdpMode" ref="rdpSurface" class="vnc-surface" :scale-mode="rdpScaleMode" :clipboard-enabled="rdpClipboardEnabled" @input="sendRdpInput" @clipboard-out="pasteRdpClipboard" @paste-request="pasteRdpClipboard" @desktop-size="(size) => (rdpDesktopSize = size)" @container-resize="onRdpSurfaceResize" />
        <RdpClipboardFilesOverlay
          v-if="isRdpMode && rdpState.state === 'running'"
          :t="t"
          :busy="rdpFileBusy"
          :progress="rdpFileProgress"
          :remote-files="rdpRemoteFiles"
          :archive-ready="rdpArchiveReady"
          :remote-ready="rdpRemoteReady"
          :start-remote-download="startRdpRemoteDownload"
          :save-remote-archive="saveRdpRemoteArchive"
          :dismiss-remote-files="dismissRdpRemoteFiles"
          @focus-desktop="rdpSurface?.$el?.querySelector('canvas')?.focus()"
        />
        </template>
        <template #cursor>
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
        </template>
        <template #panels>
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
        </template>
        <template #status>
        <!-- 连接/本地/协议状态层：组件在 components/TerminalStatusOverlays.vue
             （T20 迁出；插槽仅保持原 DOM 位置）。 -->
        <TerminalStatusOverlays
          :reconnect-pending="reconnectPending"
          :reconnect-countdown="reconnectCountdown"
          :recording-active="recordingActive"
          :recording-elapsed-sec="recordingElapsedSec"
          :record-countdown="recordCountdown"
          :is-local-mode="isLocalMode"
          :local-shell-restored="localShellRestored"
          :local-state="localState"
          :local-exit-code="localExitCode"
          :is-telnet-mode="isTelnetMode"
          :telnet-state="telnetState"
          :telnet-error="telnetError"
          :is-serial-mode="isSerialMode"
          :serial-state="serialState"
          :serial-error="serialError"
          :is-vnc-mode="isVncMode"
          :vnc-state="vncState"
          :vnc-error="vncError"
          :is-rdp-mode="isRdpMode"
          :is-rdp-connection="isRdpConnection"
          :rdp-state="rdpState"
          :rdp-closed-title="rdpClosedTitle"
          :format-duration="formatDuration"
          :t="t"
          @reconnect-now="reconnectNow"
          @toggle-recording="toggleRecording"
          @cancel-record-countdown="cancelRecordCountdown"
          @restart-local="restartLocalTerminal()"
          @close-local="closeLocalTerminal()"
          @dismiss-restored-shell="dismissRestoredLocalShell()"
          @close-telnet="closeTelnetSession"
          @close-serial="closeSerialSession"
          @close-vnc="closeVncSession"
          @vnc-reconnect="reconnectVncSession"
          @rdp-reconnect="reconnectRdpSession"
          @rdp-edit-credentials="rdpDialogOpen = true"
          @rdp-close="closeRdpSession"
        />
        </template>
        <template #connection>
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
        </template>
        <template #transfer>
        <!-- 传输/回放浮层：组件在 components/TransferOverlays.vue（T20 迁出；
             zmodem/trzsz/串口上传进度 + 回放弹窗）。 -->
        <TransferOverlays
          :zmodem-busy="zmodemBusy"
          :zmodem-state="zmodemState"
          :zmodem-percent="zmodemPercent"
          :zmodem-file-count="zmodemFileCount"
          :zmodem-file-index="zmodemFileIndex"
          :zmodem-speed="zmodemSpeed"
          :zmodem-status-label="zmodemStatusLabel"
          :trzsz-overlay-visible="trzszOverlayVisible"
          :trzsz-phase="trzszPhase"
          :trzsz-status-label="trzszStatusLabel"
          :trzsz-percent="trzszPercent"
          :trzsz-file-count="trzszFileCount"
          :trzsz-file-index="trzszFileIndex"
          :trzsz-speed="trzszSpeed"
          :trzsz-busy="trzszBusy"
          :serial-upload="serialUpload"
          :serial-upload-busy="serialUploadBusy"
          :serial-upload-overlay-visible="serialUploadOverlayVisible"
          :serial-upload-status-label="serialUploadStatusLabel"
          :serial-upload-percent-value="serialUploadPercentValue"
          :replay-state="replayState"
          :replay-playing="replayPlaying"
          :replay-speed="replaySpeed"
          :replay-playhead-ms="replayPlayheadMs"
          :replay-duration-ms="replayDurationMs"
          :replay-exporting="replayExporting"
          :replay-motion-enabled="replayMotionEnabled"
          :format-bytes="formatBytes"
          :format-duration="formatDuration"
          :t="t"
          @cancel-zmodem="cancelZmodemUpload"
          @cancel-trzsz="cancelTrzszTransfer"
          @cancel-serial-upload="cancelSerialUpload"
          @reset-serial-upload="serialUpload = initialSerialUploadState()"
          @replay-close="closeReplay"
          @replay-toggle-play="toggleReplayPlay"
          @update:replay-speed="replaySpeed = $event"
          @replay-seek="onReplaySeek"
          @replay-export-gif="exportReplayGif"
          @replay-motion-toggle="toggleReplayMotion"
          @bind-replay-host="bindReplayHost"
        />
        </template>
        <template #recordings>
          <RecordingsPanel
            v-if="recordingsOpen"
            :recordings="recordings"
            :filtered-recordings="filteredRecordings"
            :loading="recordingsLoading"
            :query="recordingsQuery"
            :hits="recordingHits"
            :search-busy="recordingSearchBusy"
            :recording-active="recordingActive"
            :recording-elapsed-sec="recordingElapsedSec"
            :record-countdown="recordCountdown"
            :replay-exporting="replayExporting"
            :recording-exporting-id="recordingExportingId"
            :local-can-save="localCanSave"
            :can-record="!!recordingTarget"
            :clear-all-busy="recordingClearAllSubmitting"
            :t="t"
            :format-recorded-at="formatRecordedAt"
            :format-duration="formatDuration"
            :format-bytes="formatBytes"
            @close="toggleRecordings"
            @refresh="refreshRecordings"
            @refresh-recording="refreshRecordings"
            @stop-recording="toggleRecording"
            @cancel-countdown="cancelRecordCountdown"
            @update:query="recordingsQuery = $event"
            @clear-search="clearRecordingsSearch"
            @start-recording="toggleRecording"
            @open-replay="openReplay"
            @export-gif="exportRecordingFromList"
            @export-transcript="exportRecordingTranscript"
            @reveal="revealRecording"
            @delete="deleteRecording"
            @clear-all="recordingClearAllOpen = true"
          />
        </template>
      </WorkbenchStage>
</template>

<style scoped>
/* 终端行内 ghost 自动建议（对标 Warp/fish）：灰字盖在光标格上，无交互
   （pointer-events:none），不参与选区/搜索。终端主题色在 xterm 内部渲染，
   CSS 拿不到，故用全局次级前景色 + 低透明度近似 fish 的 dim 灰。 */
.terminal-ghost {
  position: absolute;
  z-index: 3;
  max-width: calc(100% - 16px);
  overflow: visible;
  white-space: pre;
  line-height: 1;
  color: var(--muted-foreground);
  opacity: 0.55;
  pointer-events: none;
}
/* 键位胶囊（批 4e，Warp accept-autosuggestion-hint 同位）：灰字尾巴上的
   小键帽，pointer-events 独立放开；下拉菜单盖在其上方（z-index 高于浮层
   互不干扰）。 */
.terminal-ghost-keycap {
  pointer-events: auto;
  display: inline-flex;
  align-items: center;
  margin-left: 8px;
  padding: 1px 6px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 10px;
  line-height: 1.4;
  cursor: pointer;
  vertical-align: 1px;
}

.terminal-ghost-keycap:hover {
  background: var(--accent);
}

.terminal-ghost-keymenu {
  pointer-events: auto;
  position: absolute;
  left: 0;
  bottom: calc(100% + 4px);
  z-index: 31;
  display: flex;
  flex-direction: column;
  min-width: 120px;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--popover);
  box-shadow: var(--shadow-popover);
}

.terminal-ghost-keyoption {
  display: flex;
  align-items: center;
  padding: 4px 8px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  white-space: nowrap;
}

.terminal-ghost-keyoption.active,
.terminal-ghost-keyoption:hover {
  background: var(--accent);
}

.terminal-ghost-keymenu-note {
  padding: 4px 8px 2px;
  color: var(--muted-foreground);
  font-size: 10px;
  white-space: nowrap;
}
</style>
