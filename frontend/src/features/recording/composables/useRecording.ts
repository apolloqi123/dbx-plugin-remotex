import { onBeforeUnmount, ref, watch, type Ref } from "vue";
import { nextCountdownValue, RECORD_COUNTDOWN_START } from "../lib/recordingCountdown";
import { rpc } from "../../../shared/lib/rpc";
import { mergeEventPages, type RecordingSummary, type ReplayEvent, type ReplayEventPage } from "../lib/replayScheduler";
import { type TerminalThemeLike } from "../../terminal/lib/appearance/terminalScheme";
import { type TerminalAppearanceState } from "../../terminal/lib/appearance/terminalAppearance";
import { type TerminalFontOverride } from "../../terminal/lib/appearance/terminalFont";
import { exportRecordingGif as runRecordingGifExport, exportRecordingTranscript as runRecordingTranscriptExport, type RecordingExportDeps } from "../lib/recordingExport";
import { useReplayPlayer } from "./useReplayPlayer";
import { useRecordingSearch } from "./useRecordingSearch";

/** F3：终端录制 + 回放（asciicast v2）———

录制开始倒计时（录制软件惯例）：点击后 3→2→1 动画，归零才真正
recording/start；Esc/点击遮罩取消。录制中工具栏按钮变红，终端区底部
 * 状态与录制/回放/GIF 导出交互收口在 composables/useRecording（原 F3 分节整体迁移）。
 * 拆分：回放播放器（useReplayPlayer）、列表搜索（useRecordingSearch）、
 * GIF/transcript 导出管线（lib/recordingExport）独立；导出锁与两条保存
 * 链的编排仍在本文件。 */
export function useRecording(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  /** 录制目标会话：SSH 会话或本地 PTY（dock 本地终端），缺省=无可录制会话。 */
  recordingTarget: Ref<{ kind: "ssh" | "local"; sessionId: string } | undefined>;
  appearance: Ref<{ terminal: { fontSize: number } }>;
  terminalTheme: () => TerminalThemeLike;
  openTransferTarget: (path: string) => Promise<void>;
  revealTransferTarget: (path: string) => Promise<void>;
  terminalFontOverride: Ref<TerminalFontOverride>;
  terminalAppearance: Ref<TerminalAppearanceState>;
  hostFontFamily: () => string;
  closeMetrics: () => void;
  loadDownloadDir: () => string;
  loadDownloadUseDefaultDir: () => boolean;
  askDownloadTarget: (fileName: string) => Promise<{ dir: string; setDefault: boolean } | undefined>;
  applyChosenDirAsDefault: (dir: string) => void;
  resolveDownloadConflictFor: (dir: string, fileName: string) => Promise<"rename" | "overwrite" | undefined>;
  probeLocalCapabilities: () => Promise<{ canSaveLocal: boolean; downloadsDir: string } | undefined>;
  saveHostFile: (chunks: Uint8Array[], fileName: string) => Promise<void>;
}) {
  const { t, showNotice, showError, recordingTarget, appearance, terminalTheme, openTransferTarget, revealTransferTarget, terminalFontOverride, terminalAppearance, hostFontFamily, closeMetrics, loadDownloadDir, loadDownloadUseDefaultDir, askDownloadTarget, applyChosenDirAsDefault, resolveDownloadConflictFor, probeLocalCapabilities, saveHostFile } = options;

const recordingActive = ref(false);
const recordingsOpen = ref(false);
const recordings = ref<RecordingSummary[]>([]);
const recordingsLoading = ref(false);
// 删除确认走应用内弹窗：工作台 iframe 是 sandbox="allow-scripts"（无
// allow-modals），window.confirm 恒返回 false——曾让删除按钮看起来完全失效。
const recordingDeleteTarget = ref<RecordingSummary | null>(null);
const recordingDeleteSubmitting = ref(false);
// 行内导出进行中的 recordingId：多条记录共用全局导出锁（replayExporting），
// 只有发起行显示 Encoding…，其余行仅禁用。
const recordingExportingId = ref<string | null>(null);
const replayExporting = ref(false);

// —— M14 录制增强：列表搜索（名称过滤 + 内容全文命中摘录）———
const {
  recordingsQuery,
  recordingHits,
  recordingSearchBusy,
  filteredRecordings,
  runRecordingSearch,
  clearRecordingsSearch,
  disposeRecordingSearch,
} = useRecordingSearch({ recordings });

const recordCountdown = ref<number | null>(null);
const recordingStartedAt = ref<number | null>(null);
const recordingElapsedSec = ref(0);
let recordCountdownTimer = 0;
let recordingElapsedTimer = 0;

function beginRecordCountdown() {
  if (!recordingTarget.value || recordCountdown.value !== null) return;
  recordCountdown.value = RECORD_COUNTDOWN_START;
  window.clearInterval(recordCountdownTimer);
  recordCountdownTimer = window.setInterval(() => {
    const next = nextCountdownValue(recordCountdown.value);
    recordCountdown.value = next;
    if (next === null) {
      window.clearInterval(recordCountdownTimer);
      void startRecordingNow();
    }
  }, 1000);
}

function cancelRecordCountdown() {
  window.clearInterval(recordCountdownTimer);
  recordCountdown.value = null;
}

function startRecordingClock() {
  recordingStartedAt.value = Date.now();
  recordingElapsedSec.value = 0;
  window.clearInterval(recordingElapsedTimer);
  recordingElapsedTimer = window.setInterval(() => {
    recordingElapsedSec.value = recordingStartedAt.value
      ? Math.floor((Date.now() - recordingStartedAt.value) / 1000)
      : 0;
  }, 1000);
}

function stopRecordingClock() {
  window.clearInterval(recordingElapsedTimer);
  recordingStartedAt.value = null;
  recordingElapsedSec.value = 0;
}

async function startRecordingNow() {
  const target = recordingTarget.value;
  if (!target || recordingActive.value) return;
  try {
    await rpc(
      target.kind === "local" ? "local/recording/start" : "ssh/recording/start",
      { sessionId: target.sessionId },
    );
    recordingActive.value = true;
    startRecordingClock();
    showNotice(t("recordingStarted"));
  } catch (cause) {
    showError(cause);
  }
}

async function toggleRecording() {
  const target = recordingTarget.value;
  if (!target) return;
  if (!recordingActive.value) {
    beginRecordCountdown();
    return;
  }
  try {
    await rpc(
      target.kind === "local" ? "local/recording/stop" : "ssh/recording/stop",
      { sessionId: target.sessionId },
    );
    recordingActive.value = false;
    stopRecordingClock();
    showNotice(t("recordingStopped"));
    if (recordingsOpen.value) await loadRecordings();
  } catch (cause) {
    showError(cause);
  }
}

// 目标会话消失（SSH 断开 / 本地 shell 退出）：sidecar 侧录制随会话槽位
// 终止，前端红点与计时器同步复位，不悬挂在已消失的会话上。
watch(recordingTarget, (target) => {
  if (!target && recordingActive.value) {
    recordingActive.value = false;
    stopRecordingClock();
  }
});

async function loadRecordings() {
  recordingsLoading.value = true;
  try {
    const result = await rpc<{ recordings: RecordingSummary[] }>("ssh/recording/list", {});
    recordings.value = result.recordings ?? [];
  } catch {
    recordings.value = [];
  } finally {
    recordingsLoading.value = false;
  }
  // 列表有增删（删除/清空/新录制）时同步刷新内容搜索命中。
  if (recordingsQuery.value.trim()) void runRecordingSearch();
}

/** Panel-facing alias so the redesigned header can emit a distinct refresh
 * action without changing the existing loadRecordings contract. */
const refreshRecordings = loadRecordings;

function toggleRecordings() {
  const next = !recordingsOpen.value;
  if (next) {
    // 与指标浮层同位渲染：打开录制列表时收起指标，防止叠压。
    closeMetrics();
    void probeLocalCapabilities();
    void loadRecordings();
  }
  recordingsOpen.value = next;
}

// 打开录制文件所在目录：仅在桌面端（sidecar 在本机）有意义，
// web/docker 的录制文件在远端服务器上。
async function revealRecording(item: RecordingSummary) {
  try {
    await rpc("ssh/recording/reveal", { recordingId: item.recordingId });
  } catch (cause) {
    showError(cause);
  }
}

function deleteRecording(item: RecordingSummary) {
  recordingDeleteTarget.value = item;
}

async function confirmRecordingDelete() {
  const item = recordingDeleteTarget.value;
  if (!item || recordingDeleteSubmitting.value) return;
  recordingDeleteSubmitting.value = true;
  try {
    await rpc("ssh/recording/delete", { recordingId: item.recordingId });
    recordingDeleteTarget.value = null;
    await loadRecordings();
  } catch (cause) {
    showError(cause);
  } finally {
    recordingDeleteSubmitting.value = false;
  }
}

// 一键清空：应用内确认弹窗（沙箱 iframe confirm 恒 false），确认后
// ssh/recording/clear 全删 .cast，重载列表并提示删除数量。
const recordingClearAllOpen = ref(false);
const recordingClearAllSubmitting = ref(false);

async function confirmRecordingClearAll() {
  if (recordingClearAllSubmitting.value) return;
  recordingClearAllSubmitting.value = true;
  try {
    const result = await rpc<{ deleted?: number }>("ssh/recording/clear", {});
    recordingClearAllOpen.value = false;
    await loadRecordings();
    showNotice(t("recordingsCleared", { count: result.deleted ?? 0 }));
  } catch (cause) {
    showError(cause);
  } finally {
    recordingClearAllSubmitting.value = false;
  }
}

// 回放：事件一次性拉全（分页合并，封顶 2 万事件），rAF 按时间轴推进。
const REPLAY_EVENT_CAP = 20000;

async function loadReplayEvents(recordingId: string): Promise<ReplayEvent[]> {
  const pages: ReplayEventPage[] = [];
  let offset = 0;
  for (;;) {
    const page = await rpc<ReplayEventPage>("ssh/recording/get", { recordingId, offset, limit: 500 });
    pages.push(page);
    offset += page.events.length;
    if (!page.hasMore || offset >= page.total || offset >= REPLAY_EVENT_CAP) break;
  }
  return mergeEventPages(pages);
}

const {
  replayState,
  replayPlaying,
  replaySpeed,
  replayPlayheadMs,
  replayHost,
  replayDurationMs,
  openReplay,
  closeReplay,
  stopReplayLoop,
  replayFrame,
  toggleReplayPlay,
  onReplaySeek,
  disposeReplay,
} = useReplayPlayer({
  appearance,
  terminalTheme,
  terminalFontOverride,
  terminalAppearance,
  hostFontFamily,
  showError,
  loadReplayEvents,
});

// 导出依赖：GIF/transcript 两条保存链（宿主 fileTransfer / sidecar 落盘 /
// 宿主 saveFile）保持原样，由 lib/recordingExport 的管线按调用方传入的回调
// 执行；导出锁（replayExporting / recordingExportingId）仍由本文件持有。
const exportDeps: RecordingExportDeps = {
  t,
  showNotice,
  showError,
  appearance,
  terminalTheme,
  terminalFontOverride,
  terminalAppearance,
  hostFontFamily,
  loadDownloadDir,
  loadDownloadUseDefaultDir,
  askDownloadTarget,
  applyChosenDirAsDefault,
  resolveDownloadConflictFor,
  probeLocalCapabilities,
  saveHostFile,
  openTransferTarget,
  revealTransferTarget,
  loadReplayEvents,
  recordingExportingId,
};

/** transcript 导出（M14）：管线在 lib/recordingExport，行内导出锁与错误呈现
 * 由该管线按既有顺序处理。 */
function exportRecordingTranscript(item: RecordingSummary) {
  return runRecordingTranscriptExport(item, exportDeps);
}

/** GIF 导出：离屏 xterm 逐事件重放（管线在 lib/recordingExport）。 */
function exportRecordingGif(summary: RecordingSummary, events: readonly ReplayEvent[]) {
  return runRecordingGifExport(summary, events, exportDeps);
}

async function exportReplayGif() {
  const state = replayState.value;
  if (!state || replayExporting.value || !state.events.length) return;
  replayExporting.value = true;
  try {
    await exportRecordingGif(state.summary, state.events);
  } catch (cause) {
    showError(cause);
  } finally {
    replayExporting.value = false;
  }
}

// 列表行内导出：按需拉取事件（回放窗不必先打开），再走同一导出管线。
async function exportRecordingFromList(item: RecordingSummary) {
  if (replayExporting.value) return;
  replayExporting.value = true;
  recordingExportingId.value = item.recordingId;
  try {
    const events = await loadReplayEvents(item.recordingId);
    if (!events.length) throw new Error(t("replayExportFailed"));
    await exportRecordingGif(item, events);
  } catch (cause) {
    showError(cause);
  } finally {
    recordingExportingId.value = null;
    replayExporting.value = false;
  }
}

function formatDuration(secs: number) {
  const total = Math.max(0, Math.round(secs));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

function formatRecordedAt(startedAt?: number) {
  if (!startedAt) return "";
  return new Date(startedAt * 1000).toLocaleString();
}

onBeforeUnmount(() => {
  disposeReplay();
  window.clearInterval(recordCountdownTimer);
  window.clearInterval(recordingElapsedTimer);
  // 搜索防抖：卸载后不再发起 invoke。
  disposeRecordingSearch();
});

  return {
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
    refreshRecordings,
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
  };
}
