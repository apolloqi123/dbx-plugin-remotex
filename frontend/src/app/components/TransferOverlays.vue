<script setup lang="ts">
import type { ComponentPublicInstance } from "vue";
import { Loader2, Pause, Play, TriangleAlert, X } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../shared/ui/select/index";
import type { SerialUploadUiState } from "../../features/serial/lib/serialUpload";
import type { RecordingSummary, ReplayEvent } from "../../features/recording/lib/replayScheduler";

/** 传输/回放浮层（T20）：zmodem、trzsz、串口文件上传进度条与录制回放弹窗。
 *  组件只渲染并上抛动作；传输状态机与回放播放器仍由 App 持有。 */
const props = defineProps<{
  zmodemBusy: boolean;
  zmodemState: "idle" | "waiting" | "uploading" | "receiving";
  zmodemPercent: number;
  zmodemFileCount: number;
  zmodemFileIndex: number;
  zmodemSpeed: number;
  zmodemStatusLabel: string;
  trzszOverlayVisible: boolean;
  trzszPhase: "idle" | "waiting" | "transferring" | "success" | "failed";
  trzszStatusLabel: string;
  trzszPercent: number;
  trzszFileCount: number;
  trzszFileIndex: number;
  trzszSpeed: number;
  trzszBusy: boolean;
  serialUpload: SerialUploadUiState;
  serialUploadBusy: boolean;
  serialUploadOverlayVisible: boolean;
  serialUploadStatusLabel: string;
  serialUploadPercentValue: number;
  replayState: { summary: RecordingSummary; events: ReplayEvent[] } | null;
  replayPlaying: boolean;
  replaySpeed: number;
  replayPlayheadMs: number;
  replayDurationMs: number;
  replayExporting: boolean;
  formatBytes: (value: number) => string;
  formatDuration: (seconds: number) => string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "cancel-zmodem"): void;
  (e: "cancel-trzsz"): void;
  (e: "cancel-serial-upload"): void;
  (e: "reset-serial-upload"): void;
  (e: "replay-close"): void;
  (e: "replay-toggle-play"): void;
  (e: "update:replaySpeed", value: number): void;
  (e: "replay-seek", event: Event): void;
  (e: "replay-export-gif"): void;
  /** 回放宿主元素上抛：回放播放器（App 持有）需要在弹窗挂载后 open xterm。 */
  (e: "bind-replay-host", el: HTMLDivElement | null): void;
}>();

const { t } = props;

function bindReplayHost(el: Element | ComponentPublicInstance | null) {
  emit("bind-replay-host", el instanceof HTMLDivElement ? el : null);
}
</script>

<template>
  <div v-if="zmodemBusy" class="zmodem-status trzsz-status" role="status">
    <Loader2 class="spinning" />
    <span class="trzsz-label">{{ zmodemStatusLabel }}</span>
    <progress v-if="zmodemState === 'uploading' || zmodemState === 'receiving'" :value="zmodemPercent" max="100" />
    <span v-if="(zmodemState === 'uploading' || zmodemState === 'receiving') && zmodemFileCount > 1" class="trzsz-count mono">{{ zmodemFileIndex }}/{{ zmodemFileCount }}</span>
    <span v-if="zmodemSpeed">{{ formatBytes(zmodemSpeed) }}/s</span>
    <button class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel-zmodem')"><X /></button>
  </div>
  <div v-if="trzszOverlayVisible" class="zmodem-status trzsz-status" role="status" :class="{ 'trzsz-done': trzszPhase === 'success', 'trzsz-failed': trzszPhase === 'failed' }">
    <Loader2 v-if="trzszPhase === 'waiting' || trzszPhase === 'transferring'" class="spinning" />
    <TriangleAlert v-else-if="trzszPhase === 'failed'" />
    <span class="trzsz-label">{{ trzszStatusLabel }}</span>
    <progress v-if="trzszPhase === 'transferring'" :value="trzszPercent" max="100" />
    <span v-if="trzszPhase === 'transferring' && trzszFileCount > 1" class="trzsz-count mono">{{ trzszFileIndex }}/{{ trzszFileCount }}</span>
    <span v-if="trzszPhase === 'transferring' && trzszSpeed" class="trzsz-speed">{{ formatBytes(trzszSpeed) }}/s</span>
    <button v-if="trzszBusy" class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel-trzsz')"><X /></button>
  </div>
  <!-- 串口文件上传进度（NyaTerm 对齐 P0-3）：running 吞键入 + 可取消；
       complete/failed 保留展示，由用户点 × 收起。 -->
  <div v-if="serialUploadOverlayVisible" class="zmodem-status trzsz-status" role="status" :class="{ 'trzsz-done': serialUpload.phase === 'complete', 'trzsz-failed': serialUpload.phase === 'failed' }">
    <Loader2 v-if="serialUploadBusy" class="spinning" />
    <TriangleAlert v-else-if="serialUpload.phase === 'failed'" />
    <span class="trzsz-label">{{ serialUploadStatusLabel }}</span>
    <progress v-if="serialUploadBusy" :value="serialUploadPercentValue" max="100" />
    <span class="trzsz-count mono">{{ serialUpload.protocol.toUpperCase() }}</span>
    <button v-if="serialUploadBusy" class="trzsz-cancel" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel-serial-upload')"><X /></button>
    <button v-else class="trzsz-cancel" :title="t('close')" :aria-label="t('close')" @click="emit('reset-serial-upload')"><X /></button>
  </div>
  <!-- 回放弹窗：xterm 重放 + 倍速/进度/GIF 导出 -->
  <div v-if="replayState" class="replay-overlay" @click.self="emit('replay-close')">
    <section class="replay-modal">
      <header>
        <h2>{{ t("replayTitle") }}<span class="metrics-host"> · {{ replayState.summary.host || replayState.summary.recordingId }}</span></h2>
        <button class="icon-button" :title="t('replayClose')" :aria-label="t('replayClose')" @click="emit('replay-close')"><X /></button>
      </header>
      <div class="replay-terminal-wrap">
        <div :ref="bindReplayHost" class="replay-terminal"></div>
        <div v-if="!replayState.events.length" class="replay-empty">{{ t("replayEmpty") }}</div>
      </div>
      <div class="replay-controls">
        <button class="icon-button" :title="t(replayPlaying ? 'replayPause' : 'replayPlay')" :aria-label="t(replayPlaying ? 'replayPause' : 'replayPlay')" @click="emit('replay-toggle-play')"><Pause v-if="replayPlaying" /><Play v-else /></button>
        <Select :model-value="String(replaySpeed)" @update:model-value="(v) => emit('update:replaySpeed', Number(v))">
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
        <input class="replay-seek" type="range" min="0" :max="Math.max(1, replayDurationMs)" :value="replayPlayheadMs" step="100" @input="emit('replay-seek', $event)" />
        <span class="mono replay-time">{{ formatDuration(replayPlayheadMs / 1000) }} / {{ formatDuration(replayDurationMs / 1000) }}</span>
        <button class="link-button" :disabled="replayExporting" @click="emit('replay-export-gif')">{{ replayExporting ? t("replayExporting") : t("replayExportGif") }}</button>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* 回放弹窗并入模态体系：遮罩用 --overlay、z-index 走 80 梯队、圆角同 .modal。 */
.replay-overlay { position: fixed; inset: 0; background: var(--overlay); z-index: 80; display: flex; align-items: center; justify-content: center; }
.replay-modal { background: var(--popover); color: var(--foreground); border: 1px solid var(--border); border-radius: var(--radius-lg, 8px); padding: 16px; width: min(920px, 92vw); display: flex; flex-direction: column; gap: 10px; box-shadow: var(--shadow-modal); }
/* 标题行弹性布局：关闭按钮固定右上角（block 布局下按钮会掉到标题下一行）。 */
.replay-modal header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.replay-modal header h2 { margin: 0; min-width: 0; overflow: hidden; font-size: 14px; text-overflow: ellipsis; white-space: nowrap; }
/* 不固定高度：xterm 26 行实际渲染 442px，写死 420px 会让终端溢出压住下方控制条。 */
.replay-terminal { min-height: 44px; }
.replay-controls { display: flex; align-items: center; gap: 10px; }
.replay-seek { flex: 1; min-width: 0; height: 4px; accent-color: var(--primary); cursor: pointer; }
/* 空录制（00:00/00:00）在终端区中央给提示，不再黑屏干等。 */
.replay-terminal-wrap { position: relative; }
.replay-empty { position: absolute; inset: 0; display: grid; place-items: center; color: var(--muted-foreground); font-size: 12px; pointer-events: none; }
/* 控制行控件对齐：播放钮 24px / 倍速 26px（SelectTrigger xs）/ 导出按钮走次级按钮规范。 */
.replay-controls .link-button { display: inline-flex; height: 24px; align-items: center; align-self: center; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 11px; }
.replay-controls .link-button:hover:not(:disabled) { background: var(--accent); }
.replay-speed { flex: 0 0 auto; width: 76px; }
.replay-time { min-width: 110px; text-align: right; font-size: 12px; color: var(--muted-foreground); }
</style>
