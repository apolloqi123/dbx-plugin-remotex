<script setup lang="ts">
// 录制记录浮层：以一条连续时间脊线组织记录，动作常驻可达；数据面仍在
// useRecording，本组件只渲染并上抛动作。
import { Disc, FileText, FolderOpen, ImagePlay, Loader2, Play, RefreshCw, Search, Square, Trash2, X } from "@lucide/vue";
import type { RecordingSummary } from "../lib/replayScheduler";

const props = defineProps<{
  recordings: RecordingSummary[];
  filteredRecordings: RecordingSummary[];
  loading: boolean;
  query: string;
  hits: Record<string, string[]>;
  searchBusy: boolean;
  recordingActive: boolean;
  recordingElapsedSec: number;
  recordCountdown: number | null;
  replayExporting: boolean;
  recordingExportingId: string | null;
  localCanSave: boolean;
  canRecord: boolean;
  clearAllBusy: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  formatRecordedAt: (value: number) => string;
  formatDuration: (seconds: number) => string;
  formatBytes: (value?: number) => string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "update:query", value: string): void;
  (e: "clear-search"): void;
  (e: "refresh"): void;
  (e: "refresh-recording"): void;
  (e: "stop-recording"): void;
  (e: "cancel-countdown"): void;
  (e: "start-recording"): void;
  (e: "open-replay", item: RecordingSummary): void;
  (e: "export-gif", item: RecordingSummary): void;
  (e: "export-transcript", item: RecordingSummary): void;
  (e: "reveal", item: RecordingSummary): void;
  (e: "delete", item: RecordingSummary): void;
  (e: "clear-all"): void;
}>();

function elapsedLabel(seconds: number): string {
  return props.formatDuration(Math.max(0, seconds));
}

function openRow(item: RecordingSummary) {
  emit("open-replay", item);
}
</script>

<template>
  <section class="metrics-float recordings-float" :aria-label="t('recordingsTitle')" @keydown.esc="emit('close')">
    <header class="recordings-head">
      <span class="recordings-tape-node" :class="{ live: recordingActive }" aria-hidden="true"><i /><Disc /></span>
      <div class="recordings-head-text">
        <strong>{{ t("recordingsTitle") }}</strong>
        <span v-if="recordingActive" class="recordings-live">{{ t("recordingMenuActive") }} · {{ elapsedLabel(recordingElapsedSec) }}</span>
        <span v-else-if="recordCountdown !== null" class="recordings-countdown">{{ t("recordingMenuCountdown", { seconds: recordCountdown }) }}</span>
        <span v-else>{{ t("recordingsSummary", { count: recordings.length }) }}</span>
      </div>
      <button class="recordings-head-action" type="button" :title="t('recordingsRefresh')" :aria-label="t('recordingsRefresh')" @click="emit('refresh')"><RefreshCw /></button>
      <button v-if="recordingActive" class="recordings-head-action stop" type="button" :title="t('recordingsStop')" :aria-label="t('recordingsStop')" @click="emit('stop-recording')"><Square /><span>{{ t("recordingsStop") }}</span></button>
      <button v-else-if="recordCountdown !== null" class="recordings-head-action" type="button" :title="t('recordingMenuCancel')" :aria-label="t('recordingMenuCancel')" @click="emit('cancel-countdown')"><span>{{ t("recordingMenuCancel") }}</span></button>
      <button v-if="recordings.length" class="recordings-head-action" type="button" :title="t('recordingsClear')" :aria-label="t('recordingsClear')" :disabled="clearAllBusy" @click="emit('clear-all')"><Trash2 /></button>
      <button class="recordings-head-action" type="button" :title="t('close')" :aria-label="t('close')" @click="emit('close')"><X /></button>
    </header>

    <div class="recordings-subhead">
      <span>{{ t("recordingsSummary", { count: recordings.length }) }}</span>
      <span>{{ t("recordingsFooterHint") }}</span>
    </div>

    <div v-if="recordings.length" class="recordings-toolbar">
      <Search aria-hidden="true" />
      <input :value="query" type="search" :placeholder="t('recordingsSearchPlaceholder')" :aria-label="t('recordingsSearchPlaceholder')" @input="emit('update:query', ($event.target as HTMLInputElement).value)" />
      <Loader2 v-if="searchBusy" class="spinning recordings-search-spin" aria-hidden="true" />
      <button v-if="query" class="recordings-search-clear" type="button" :title="t('close')" :aria-label="t('close')" @click="emit('clear-search')"><X /></button>
    </div>

    <div class="recordings-body">
      <div v-if="loading && !recordings.length" class="recordings-state compact"><Loader2 class="spinning" aria-hidden="true" />{{ t("loading") }}</div>
      <div v-else-if="!recordings.length" class="recordings-state">
        <span class="recordings-state-icon" aria-hidden="true"><Disc /></span>
        <strong>{{ t("recordingsEmpty") }}</strong>
        <p>{{ t("recordingsEmptyHint") }}</p>
        <button v-if="canRecord" class="primary-button" type="button" @click="emit('start-recording')">{{ t("recordingMenuStart") }}</button>
      </div>
      <div v-else-if="!filteredRecordings.length" class="recordings-state compact">
        <Search aria-hidden="true" />
        <span>{{ t("recordingsNoMatch") }}</span>
        <button class="link-button" type="button" @click="emit('refresh-recording')">{{ t("recordingsRefresh") }}</button>
        <button class="link-button" type="button" @click="emit('clear-search')">{{ t("terminalHistory.clearSearch") }}</button>
      </div>
      <div v-else class="recordings-timeline" role="list">
        <article
          v-for="(item, index) in filteredRecordings"
          :key="item.recordingId"
          class="recording-row"
          role="listitem"
          tabindex="0"
          :aria-label="item.host || item.recordingId"
          @keydown.enter.self="openRow(item)"
        >
          <span class="recording-node" :class="{ live: recordingActive && index === 0 }" aria-hidden="true"></span>
          <div class="recording-text">
            <strong class="recording-host">{{ item.host || item.recordingId }}</strong>
            <span class="recording-meta mono">{{ formatRecordedAt(item.startedAt ?? 0) }}<template v-if="item.bytes"> · {{ formatBytes(item.bytes) }}</template></span>
            <span v-for="(hit, hitIndex) in hits[item.recordingId] ?? []" :key="hitIndex" class="recording-hit mono">{{ hit }}</span>
          </div>
          <span class="recording-duration mono">{{ formatDuration(item.durationSecs ?? 0) }}</span>
          <div class="recording-actions">
            <button class="recording-action primary" type="button" :title="t('replayOpen')" :aria-label="t('replayOpen')" @click.stop="emit('open-replay', item)"><Play /></button>
            <button class="recording-action" type="button" :title="t('replayExportGif')" :aria-label="t('replayExportGif')" :disabled="replayExporting" @click.stop="emit('export-gif', item)"><Loader2 v-if="recordingExportingId === item.recordingId" class="spinning" /><ImagePlay v-else /></button>
            <button class="recording-action" type="button" :title="t('recordingExportTranscript')" :aria-label="t('recordingExportTranscript')" :disabled="replayExporting" @click.stop="emit('export-transcript', item)"><FileText /></button>
            <button v-if="localCanSave" class="recording-action" type="button" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click.stop="emit('reveal', item)"><FolderOpen /></button>
            <button class="recording-action danger" type="button" :title="t('recordingDelete')" :aria-label="t('recordingDelete')" @click.stop="emit('delete', item)"><Trash2 /></button>
          </div>
        </article>
      </div>
    </div>

    <footer class="recordings-footer">
      <span>{{ t("recordingsKeyboardHint") }}</span>
      <button v-if="canRecord" class="recordings-start-link" type="button" @click="emit('start-recording')">{{ t("recordingMenuStart") }}</button>
    </footer>
  </section>
</template>

<style scoped>
.recordings-float { width: min(520px, calc(100% - 16px)); font-family: var(--ui-font-family); }
.recordings-head { display: flex; align-items: center; gap: 6px; padding: 8px 8px 8px 10px; border-bottom: 1px solid var(--card-border); }
.recordings-tape-node { position: relative; display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex: 0 0 28px; border-radius: 50%; background: color-mix(in srgb, var(--tone-violet, var(--primary)) 12%, var(--card-surface)); color: var(--tone-violet, var(--primary)); }
.recordings-tape-node.live { background: color-mix(in srgb, var(--destructive) 14%, transparent); color: var(--destructive); }
.recordings-tape-node svg { width: 14px; height: 14px; }
.recordings-tape-node i { position: absolute; inset: -3px; border-radius: 50%; border: 1px solid currentColor; opacity: .35; }
.recordings-tape-node.live i { animation: recordings-pulse 1.8s ease-in-out infinite; }
@keyframes recordings-pulse { 50% { opacity: .1; transform: scale(1.12); } }
.recordings-head-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 1px; }
.recordings-head-text strong { font-size: 15px; font-weight: 600; }
.recordings-head-text > span { overflow: hidden; color: var(--muted-foreground); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.recordings-live, .recordings-countdown { color: var(--destructive) !important; }
.recordings-countdown { color: var(--primary) !important; }
.recordings-head-action { display: inline-flex; min-width: 28px; min-height: 28px; align-items: center; justify-content: center; gap: 4px; border: 1px solid var(--card-border); border-radius: var(--radius); padding: 0 6px; background: var(--card-surface); color: var(--foreground); font-size: 11px; cursor: pointer; }
.recordings-head-action svg { width: 14px; height: 14px; }
.recordings-head-action.stop { color: var(--destructive); }
.recordings-head-action:hover:not(:disabled) { background: var(--accent); }
.recordings-head-action:disabled { opacity: .4; cursor: default; }
.recordings-subhead { display: flex; gap: 8px; padding: 5px 10px; border-bottom: 1px solid color-mix(in srgb, var(--card-border) 60%, transparent); color: var(--muted-foreground); font-size: 11px; }
.recordings-subhead > span:first-child { flex: 0 0 auto; color: var(--foreground); }
.recordings-toolbar { display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-bottom: 1px solid color-mix(in srgb, var(--card-border) 65%, transparent); }
.recordings-toolbar > svg:first-child { width: 13px; height: 13px; flex: 0 0 13px; color: var(--muted-foreground); }
.recordings-toolbar input { min-width: 0; flex: 1; height: 28px; border: 1px solid var(--card-border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 12px; }
.recordings-toolbar input:focus-visible, .recordings-search-clear:focus-visible, .recordings-head-action:focus-visible, .recording-action:focus-visible, .recording-row:focus-visible, .recordings-footer button:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.recordings-search-spin { width: 12px; height: 12px; flex: 0 0 12px; color: var(--muted-foreground); }
.recordings-search-clear { display: inline-flex; min-width: 28px; min-height: 28px; align-items: center; justify-content: center; border: 0; border-radius: var(--radius); background: transparent; color: var(--muted-foreground); cursor: pointer; }
.recordings-search-clear svg { width: 14px; height: 14px; }
.recordings-body { display: flex; min-height: 0; flex: 1 1 auto; flex-direction: column; overflow: auto; padding: 8px 10px 10px 18px; }
.recordings-timeline { position: relative; display: flex; flex-direction: column; gap: 2px; }
.recordings-timeline::before { content: ""; position: absolute; left: 4px; top: 12px; bottom: 12px; width: 2px; background: color-mix(in srgb, var(--tone-violet, var(--primary)) 35%, var(--card-border)); }
.recording-row { position: relative; display: flex; align-items: flex-start; gap: 10px; padding: 8px 0 8px 18px; border-radius: var(--radius); cursor: default; }
.recording-row:hover { background: var(--accent); }
.recording-node { position: absolute; left: 0; top: 14px; width: 10px; height: 10px; border: 2px solid var(--tone-violet, var(--primary)); border-radius: 50%; background: var(--card-surface); }
.recording-node.live { border-color: var(--destructive); background: var(--destructive); box-shadow: 0 0 0 3px color-mix(in srgb, var(--destructive) 16%, transparent); }
.recording-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 2px; }
.recording-host { overflow: hidden; font-size: 12px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.recording-meta { overflow: hidden; color: var(--muted-foreground); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; font-family: var(--terminal-font-family); font-variant-numeric: tabular-nums; }
.recording-hit { margin-top: 2px; border-left: 2px solid color-mix(in srgb, var(--primary) 42%, transparent); padding-left: 6px; color: var(--muted-foreground); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.recording-duration { flex: 0 0 auto; margin-top: 2px; color: var(--muted-foreground); font-size: 11px; font-family: var(--terminal-font-family); font-variant-numeric: tabular-nums; }
.recording-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 2px; }
.recording-action { display: inline-flex; min-width: 28px; min-height: 28px; align-items: center; justify-content: center; border: 0; border-radius: var(--radius); padding: 0; background: transparent; color: var(--muted-foreground); cursor: pointer; }
.recording-action svg { width: 14px; height: 14px; }
.recording-action:hover:not(:disabled) { background: color-mix(in srgb, var(--foreground) 8%, transparent); color: var(--foreground); }
.recording-action.primary { color: var(--primary); }
.recording-action.danger:hover:not(:disabled) { background: color-mix(in srgb, var(--destructive) 12%, transparent); color: var(--destructive); }
.recording-action:disabled { opacity: .4; cursor: default; }
.recordings-footer { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--card-border); padding: 6px 10px; }
.recordings-footer > span { min-width: 0; flex: 1; overflow: hidden; color: var(--muted-foreground); font-size: var(--text-2xs); text-overflow: ellipsis; white-space: nowrap; }
.recordings-start-link { flex: 0 0 auto; min-height: 28px; border: 0; border-radius: var(--radius); padding: 0 8px; background: transparent; color: var(--primary); font-size: 11px; cursor: pointer; }
.recordings-start-link:hover { background: color-mix(in srgb, var(--primary) 12%, transparent); }
.recordings-state { display: flex; min-height: 190px; flex-direction: column; align-items: center; justify-content: center; gap: 6px; padding: 20px; color: var(--muted-foreground); text-align: center; }
.recordings-state-icon { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: var(--radius-lg, 12px); background: color-mix(in srgb, var(--tone-violet, var(--primary)) 12%, transparent); color: var(--tone-violet, var(--primary)); }
.recordings-state-icon svg { width: 22px; height: 22px; }
.recordings-state strong { color: var(--foreground); font-size: 12px; }
.recordings-state p { max-width: 300px; margin: 0; font-size: 11px; line-height: 1.5; }
.recordings-state .primary-button { margin-top: 4px; min-height: 28px; }
.recordings-state.compact { min-height: 0; flex-direction: row; gap: 6px; padding: 18px; }
.recordings-state.compact svg { width: 14px; height: 14px; }
.recordings-state .link-button { min-height: 28px; }
@media (max-width: 480px) {
  .recordings-head { flex-wrap: wrap; }
  .recordings-head-text { flex-basis: calc(100% - 40px); }
  .recording-row { flex-wrap: wrap; }
  .recording-duration { order: 3; margin-left: 18px; }
  .recording-actions { order: 4; flex-wrap: wrap; }
  .recordings-subhead { flex-wrap: wrap; }
}
@media (prefers-reduced-motion: reduce) {
  .recordings-tape-node.live i { animation: none; }
}
</style>
