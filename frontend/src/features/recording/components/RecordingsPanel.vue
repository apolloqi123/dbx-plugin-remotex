<script setup lang="ts">
// 录制记录浮层：搜索（主机/内容）、回放、GIF、转录、定位文件、删除与清空。
// 数据面仍在 useRecording；本组件只渲染并上抛动作。
import { Disc, FileText, FolderOpen, ImagePlay, Loader2, Play, Search, Trash2, X } from "@lucide/vue";
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
</script>

<template>
  <section class="metrics-float recordings-float">
    <header class="recordings-head">
      <span class="recordings-head-icon" :class="{ live: recordingActive }"><Disc /></span>
      <div class="recordings-head-text">
        <strong>{{ t("recordingsTitle") }}</strong>
        <span v-if="recordingActive" class="recordings-live"><i />{{ t("recordingMenuActive") }} · {{ elapsedLabel(recordingElapsedSec) }}</span>
        <span v-else-if="recordCountdown !== null" class="recordings-countdown">{{ t("recordingMenuCountdown", { seconds: recordCountdown }) }}</span>
        <span v-else>{{ t("recordingsSummary", { count: recordings.length }) }}</span>
      </div>
      <button v-if="recordings.length" class="icon-button" :title="t('recordingsClear')" :aria-label="t('recordingsClear')" :disabled="clearAllBusy" @click="emit('clear-all')"><Trash2 /></button>
      <button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="emit('close')"><X /></button>
    </header>

    <div v-if="recordings.length" class="recordings-toolbar">
      <Search />
      <input :value="query" type="search" :placeholder="t('recordingsSearchPlaceholder')" :aria-label="t('recordingsSearchPlaceholder')" @input="emit('update:query', ($event.target as HTMLInputElement).value)" />
      <Loader2 v-if="searchBusy" class="spinning recordings-search-spin" />
      <button v-if="query" class="icon-button compact" :title="t('close')" :aria-label="t('close')" @click="emit('clear-search')"><X /></button>
    </div>

    <div class="recordings-body">
      <div v-if="loading && !recordings.length" class="recordings-empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
      <div v-else-if="!recordings.length" class="recordings-empty">
        <span class="recordings-empty-icon"><Disc /></span>
        <strong>{{ t("recordingsEmpty") }}</strong>
        <p>{{ t("recordingsEmptyHint") }}</p>
        <button v-if="canRecord" class="primary-button" @click="emit('start-recording')">{{ t("recordingMenuStart") }}</button>
      </div>
      <div v-else-if="!filteredRecordings.length" class="recordings-empty compact">
        <Search />
        <span>{{ t("recordingsNoMatch") }}</span>
      </div>
      <template v-else>
        <article v-for="item in filteredRecordings" :key="item.recordingId" class="recording-card">
          <span class="recording-icon"><Disc /></span>
          <div class="recording-text">
            <strong class="recording-host">{{ item.host || item.recordingId }}</strong>
            <span class="recording-meta">{{ formatRecordedAt(item.startedAt ?? 0) }}<template v-if="item.bytes"> · {{ formatBytes(item.bytes) }}</template></span>
            <span v-for="(hit, hitIndex) in hits[item.recordingId] ?? []" :key="hitIndex" class="recording-hit mono">{{ hit }}</span>
          </div>
          <span class="recording-duration mono">{{ formatDuration(item.durationSecs ?? 0) }}</span>
          <div class="recording-actions">
            <button class="recording-action primary" :title="t('replayOpen')" :aria-label="t('replayOpen')" @click="emit('open-replay', item)"><Play /></button>
            <button class="recording-action" :title="t('replayExportGif')" :aria-label="t('replayExportGif')" :disabled="replayExporting" @click="emit('export-gif', item)"><Loader2 v-if="recordingExportingId === item.recordingId" class="spinning" /><ImagePlay v-else /></button>
            <button class="recording-action" :title="t('recordingExportTranscript')" :aria-label="t('recordingExportTranscript')" :disabled="replayExporting" @click="emit('export-transcript', item)"><FileText /></button>
            <button v-if="localCanSave" class="recording-action" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click="emit('reveal', item)"><FolderOpen /></button>
            <button class="recording-action danger" :title="t('recordingDelete')" :aria-label="t('recordingDelete')" @click="emit('delete', item)"><Trash2 /></button>
          </div>
        </article>
      </template>
    </div>

    <footer v-if="recordings.length" class="recordings-footer">
      <span>{{ t("recordingsFooterHint") }}</span>
      <button v-if="canRecord" class="recordings-start-link" @click="emit('start-recording')">{{ t("recordingMenuStart") }}</button>
    </footer>
  </section>
</template>

<style scoped>
.recordings-float { width: min(520px, calc(100% - 16px)); }
.recordings-head { display: flex; align-items: center; gap: 8px; padding: 8px 8px 8px 10px; border-bottom: 1px solid var(--border); }
.recordings-head-icon { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex: 0 0 28px; border-radius: 8px; background: var(--tone-rose-soft); color: var(--tone-rose); }
.recordings-head-icon.live { background: color-mix(in srgb, var(--destructive) 14%, transparent); color: var(--destructive); }
.recordings-head-icon svg { width: 15px; height: 15px; }
.recordings-head-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 1px; }
.recordings-head-text strong { font-size: 12px; font-weight: 600; }
.recordings-head-text > span { overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.recordings-live { display: inline-flex; align-items: center; gap: 5px; color: var(--destructive) !important; }
.recordings-live i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; box-shadow: 0 0 0 3px color-mix(in srgb, var(--destructive) 16%, transparent); }
.recordings-countdown { color: var(--primary) !important; }
.recordings-toolbar { display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-bottom: 1px solid color-mix(in srgb, var(--border) 65%, transparent); }
.recordings-toolbar > svg:first-child { width: 13px; height: 13px; flex: 0 0 13px; color: var(--muted-foreground); }
.recordings-toolbar input { min-width: 0; flex: 1; height: 26px; border: 1px solid var(--border); border-radius: 7px; padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 11.5px; }
.recordings-toolbar input:focus { border-color: color-mix(in srgb, var(--primary) 65%, var(--border)); box-shadow: 0 0 0 2px color-mix(in srgb, var(--primary) 12%, transparent); outline: none; }
.recordings-search-spin { width: 12px; height: 12px; flex: 0 0 12px; color: var(--muted-foreground); }
.recordings-body { display: flex; min-height: 0; flex: 1 1 auto; flex-direction: column; gap: 0; overflow: auto; padding: 4px 6px; }
.recording-card { display: flex; align-items: flex-start; gap: 8px; border-radius: 8px; padding: 8px; }
.recording-card + .recording-card { border-top: 1px solid color-mix(in srgb, var(--border) 42%, transparent); }
.recording-card:hover { background: var(--accent); }
.recording-icon { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex: 0 0 28px; border-radius: 8px; background: var(--tone-rose-soft); color: var(--tone-rose); }
.recording-icon svg { width: 15px; height: 15px; }
.recording-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 1px; }
.recording-host { overflow: hidden; font-size: 12px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.recording-meta { overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.recording-hit { margin-top: 2px; border-left: 2px solid color-mix(in srgb, var(--primary) 42%, transparent); padding-left: 6px; color: var(--muted-foreground); font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.recording-duration { flex: 0 0 auto; margin-top: 3px; color: var(--muted-foreground); font-size: 10.5px; font-variant-numeric: tabular-nums; }
.recording-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 1px; opacity: .7; transition: opacity 100ms ease; }
.recording-card:hover .recording-actions { opacity: 1; }
.recording-action { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; border: 0; border-radius: 6px; padding: 0; background: transparent; color: var(--muted-foreground); cursor: pointer; }
.recording-action svg { width: 13px; height: 13px; }
.recording-action:hover:not(:disabled) { background: color-mix(in srgb, var(--foreground) 8%, transparent); color: var(--foreground); }
.recording-action.primary { color: var(--primary); }
.recording-action.danger:hover:not(:disabled) { background: color-mix(in srgb, var(--destructive) 12%, transparent); color: var(--destructive); }
.recording-action:disabled { opacity: .4; cursor: default; }
.recordings-footer { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--border); padding: 6px 10px; }
.recordings-footer > span { min-width: 0; flex: 1; overflow: hidden; color: var(--muted-foreground); font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }
.recordings-start-link { flex: 0 0 auto; border: 0; border-radius: 5px; padding: 2px 6px; background: transparent; color: var(--primary); font-size: 10px; cursor: pointer; }
.recordings-start-link:hover { background: color-mix(in srgb, var(--primary) 12%, transparent); }
.recordings-empty { display: flex; min-height: 190px; flex-direction: column; align-items: center; justify-content: center; gap: 6px; padding: 20px; color: var(--muted-foreground); text-align: center; }
.recordings-empty-icon { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: 12px; background: var(--tone-rose-soft); color: var(--tone-rose); }
.recordings-empty-icon svg { width: 22px; height: 22px; }
.recordings-empty strong { color: var(--foreground); font-size: 12px; }
.recordings-empty p { max-width: 300px; margin: 0; font-size: 10.5px; line-height: 1.5; }
.recordings-empty .primary-button { margin-top: 4px; }
.recordings-empty.compact { min-height: 0; flex-direction: row; gap: 6px; padding: 18px; }
.recordings-empty.compact svg { width: 14px; height: 14px; }
</style>
