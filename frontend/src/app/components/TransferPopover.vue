<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import type { Component } from "vue";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "../../shared/ui/context-menu/index";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import { ArrowDown, ArrowUp, CheckCircle2, ExternalLink, FileUp, FolderOpen, FolderUp, ListFilter, Loader2, Pause, RefreshCw, Trash2 } from "@lucide/vue";
import TransferRow from "../../features/transfers/components/TransferRow.vue";
import {
  buildTransferRows,
  countTransferFilters,
  filterTransferRows,
  findHistoryEntry,
  type TransferFilterKey,
  type TransferRowView,
} from "../../features/transfers/lib/transferPanelRows";
import type { TransferHistoryEntry } from "../../features/transfers/lib/transferHistory";
import type { ResumableUploadTask } from "../../features/transfers/lib/transferResume";
import type { BatchUploadSummary, TransferTask } from "../../features/transfers/lib/transferQueue";
import type { FolderUploadProgress } from "../../features/sftp/lib/transfer/folderUpload";

/** 传输弹层（tab/panel 两表面共用单实例）：头部计数与筛选、活跃/历史/续传
 *  统一成同一种行，行内动作与展开详情；队列/历史状态仍由 App 的
 *  useTransferQueue / useTransferHistory 持有，本组件只渲染并上抛动作。
 *  Popover 的隐形锚点保留在组件内：父级 header 层只挂一个实例。 */
const props = defineProps<{
  transferPanelOpen: boolean;
  transferList: TransferTask[];
  transferHistory: TransferHistoryEntry[];
  resumableTasks: ResumableUploadTask[];
  folderUploadProgress: FolderUploadProgress | undefined;
  fileBatchProgress: BatchUploadSummary | undefined;
  pausedTaskIds: Set<string>;
  transferSpeeds: Record<string, number>;
  transferHistoryMenu: { taskId: string } | undefined;
  transferHistoryLoading: boolean;
  transferHistoryFailed: boolean;
  resumableLoading: boolean;
  folderUploadPercent: (state: FolderUploadProgress) => number;
  batchUploadPercent: (summary: BatchUploadSummary) => number;
  formatBytes: (value?: number) => string;
  transferPercent: (task: TransferTask) => number;
  transferShownBytes: (task: TransferTask) => number;
  transferPausable: (status: string) => boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:transferPanelOpen", value: boolean): void;
  (e: "close-history-menu"): void;
  (e: "show-history-menu", event: MouseEvent, entry: TransferHistoryEntry): void;
  (e: "toggle-pause", task: TransferTask): void;
  (e: "cancel", task: TransferTask): void;
  (e: "reveal", path: string): void;
  (e: "open-file", path: string): void;
  (e: "refresh"): void;
  (e: "open-clear"): void;
  (e: "begin-resume", task: ResumableUploadTask): void;
  (e: "resume-picked", event: Event): void;
}>();

const t = props.t;
const resumeInput = ref<HTMLInputElement | null>(null);
const filter = ref<TransferFilterKey>("all");
const expandedKey = ref("");
const scrollRef = ref<HTMLElement | null>(null);

watch(
  () => props.transferPanelOpen,
  (open) => {
    if (!open) return;
    filter.value = "all";
    expandedKey.value = "";
  },
);

const rows = computed(() =>
  buildTransferRows({
    transferList: props.transferList,
    history: props.transferHistory,
    resumableTasks: props.resumableTasks,
    pausedTaskIds: props.pausedTaskIds,
    speeds: props.transferSpeeds,
    formatBytes: props.formatBytes,
    transferPercent: props.transferPercent,
    transferShownBytes: props.transferShownBytes,
    transferPausable: props.transferPausable,
  }),
);

const counts = computed(() => countTransferFilters(rows.value));
const visibleRows = computed(() => filterTransferRows(rows.value, filter.value));

const filterOptions = computed<Array<{ key: TransferFilterKey; label: string; icon: Component; count: number }>>(() => [
  { key: "all", label: t("transferFilter.all"), icon: ListFilter, count: counts.value.all },
  { key: "upload", label: t("transferFilter.upload"), icon: ArrowUp, count: counts.value.upload },
  { key: "download", label: t("transferFilter.download"), icon: ArrowDown, count: counts.value.download },
  { key: "paused", label: t("transferFilter.paused"), icon: Pause, count: counts.value.paused },
  { key: "completed", label: t("transferFilter.completed"), icon: CheckCircle2, count: counts.value.completed },
]);

interface BatchCard {
  key: string;
  icon: Component;
  title: string;
  percent: number;
  meta: string;
  currentFile?: string;
}

const showUploadBatches = computed(() => filter.value === "all" || filter.value === "upload");
const batches = computed<BatchCard[]>(() => {
  if (!showUploadBatches.value) return [];
  const cards: BatchCard[] = [];
  if (props.folderUploadProgress) {
    const progress = props.folderUploadProgress;
    cards.push({
      key: "folder-batch",
      icon: FolderUp,
      title: t("folderUpload.title"),
      percent: props.folderUploadPercent(progress),
      meta: `${t("folderUpload.progress", {
        dirs: progress.directoriesDone,
        dirsTotal: progress.directoriesTotal,
        files: progress.filesDone,
        filesTotal: progress.filesTotal,
      })} · ${props.formatBytes(progress.bytesDone)} / ${props.formatBytes(progress.totalBytes)}`,
      currentFile: progress.currentFile,
    });
  }
  if (props.fileBatchProgress) {
    const batch = props.fileBatchProgress;
    cards.push({
      key: "file-batch",
      icon: FileUp,
      title: t("uploadBatch.title"),
      percent: props.batchUploadPercent(batch),
      meta: `${t("uploadBatch.progress", { files: batch.filesDone, filesTotal: batch.filesTotal })}${
        batch.filesFailed ? ` · ${t("uploadBatch.failed", { count: batch.filesFailed })}` : ""
      } · ${props.formatBytes(batch.bytesDone)} / ${props.formatBytes(batch.totalBytes)}`,
      currentFile: batch.currentFile,
    });
  }
  return cards;
});

const showEmpty = computed(() => !visibleRows.value.length && !batches.value.length);
const historyEntryOf = (row: TransferRowView) => (row.source === "history" ? findHistoryEntry(props.transferHistory, row.taskId) : undefined);

function rowProps(row: TransferRowView) {
  return { row, t, expanded: expandedKey.value === row.key };
}

function rowEvents(row: TransferRowView) {
  return {
    toggle: () => {
      const opening = expandedKey.value !== row.key;
      expandedKey.value = opening ? row.key : "";
      if (!opening) return;
      // 详情卡可能落在滚动区下方：展开后把整行带入可视范围。
      void nextTick(() => {
        const node = [...(scrollRef.value?.querySelectorAll<HTMLElement>(".transfer-row") ?? [])].find(
          (element) => element.dataset.rowKey === row.key,
        );
        node?.scrollIntoView({ block: "nearest" });
      });
    },
    pause: () => {
      const task = props.transferList.find((item) => item.taskId === row.taskId);
      if (task) emit("toggle-pause", task);
    },
    cancel: () => {
      const task = props.transferList.find((item) => item.taskId === row.taskId);
      if (task) emit("cancel", task);
    },
    reveal: () => {
      if (row.localPath) emit("reveal", row.localPath);
    },
    "open-file": () => {
      if (row.localPath) emit("open-file", row.localPath);
    },
    resume: () => {
      const task = props.resumableTasks.find((item) => item.taskId === row.taskId);
      if (task) beginResume(task);
    },
    "context-menu": (event: MouseEvent) => {
      const entry = historyEntryOf(row);
      if (entry) emit("show-history-menu", event, entry);
    },
  };
}

function historyMenuOpen(row: TransferRowView) {
  return row.source === "history" && props.transferHistoryMenu?.taskId === row.taskId;
}

function onHistoryMenuUpdate(row: TransferRowView, open: boolean) {
  if (!open && historyMenuOpen(row)) emit("close-history-menu");
}

/** 续传入口：父级先记录目标 task，再点开本组件的隐藏文件选择器；
 *  选文件后把原生事件交回 useTransferHistory.onResumeFilePicked。 */
function beginResume(task: ResumableUploadTask) {
  emit("begin-resume", task);
  resumeInput.value?.click();
}
</script>

<template>
  <Popover :open="transferPanelOpen" @update:open="(value) => { if (!value && !transferHistoryMenu) emit('update:transferPanelOpen', false); }">
    <PopoverAnchor as-child>
      <span class="transfer-popover-anchor" aria-hidden="true" />
    </PopoverAnchor>
    <PopoverContent class="popover transfer-popover" align="end" :side-offset="5">
      <header class="transfer-panel-head">
        <span class="transfer-panel-icon"><WorkbenchGlyph name="transfers" /></span>
        <h3>{{ t("transfers") }}</h3>
        <span class="transfer-panel-count">{{ visibleRows.length }}/{{ rows.length }}</span>
        <span class="transfer-panel-actions">
          <button type="button" class="icon-button" :title="t('refresh')" :aria-label="t('refresh')" :disabled="transferHistoryLoading || resumableLoading" @click.stop="emit('refresh')"><RefreshCw :class="{ spinning: transferHistoryLoading || resumableLoading }" /></button>
          <button type="button" class="icon-button" :title="t('transfersHistory.clear')" :aria-label="t('transfersHistory.clear')" :disabled="!transferHistory.length" @click.stop="emit('open-clear')"><Trash2 /></button>
        </span>
      </header>

      <nav class="transfer-filters" :aria-label="t('transferFilter.label')">
        <button
          v-for="option in filterOptions"
          :key="option.key"
          type="button"
          class="transfer-filter"
          :class="{ 'is-active': filter === option.key }"
          :title="option.label"
          :aria-pressed="filter === option.key"
          @click="filter = option.key"
        >
          <component :is="option.icon" />
          <span class="transfer-filter-label">{{ option.label }}</span>
          <b>{{ option.count }}</b>
        </button>
      </nav>

      <div ref="scrollRef" class="transfer-scroll">
        <article v-for="batch in batches" :key="batch.key" class="transfer-batch">
          <span class="transfer-batch-icon" :class="batch.key === 'folder-batch' ? 'is-folder' : 'is-file'">
            <component :is="batch.icon" />
          </span>
          <div class="transfer-batch-body">
            <div class="transfer-batch-head">
              <span class="transfer-batch-title">{{ batch.title }}</span>
              <b class="transfer-batch-percent">{{ batch.percent }}%</b>
            </div>
            <div class="transfer-batch-meta">{{ batch.meta }}</div>
            <div v-if="batch.currentFile" class="transfer-batch-file mono" :title="batch.currentFile">{{ batch.currentFile }}</div>
          </div>
          <span class="transfer-batch-line" :style="{ width: `${batch.percent}%` }" />
        </article>

        <div v-if="transferHistoryFailed" class="transfer-banner">
          <span>{{ t("transfersHistory.loadFailed") }}</span>
          <button type="button" class="link-button" @click.stop="emit('refresh')">{{ t("refresh") }}</button>
        </div>
        <div v-if="transferHistoryLoading && !rows.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>

        <template v-for="row in visibleRows" :key="row.key">
          <ContextMenu v-if="row.source === 'history'" :open="historyMenuOpen(row)" @update:open="(open) => onHistoryMenuUpdate(row, open)">
            <ContextMenuTrigger as-child>
              <TransferRow v-bind="rowProps(row)" :data-row-key="row.key" v-on="rowEvents(row)" />
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem @select="row.localPath && emit('reveal', row.localPath)"><FolderOpen />{{ t("revealInFolder") }}</ContextMenuItem>
              <ContextMenuItem @select="row.localPath && emit('open-file', row.localPath)"><ExternalLink />{{ t("openDownloadedFile") }}</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
          <TransferRow v-else v-bind="rowProps(row)" :data-row-key="row.key" v-on="rowEvents(row)" />
        </template>

        <div v-if="showEmpty" class="transfer-empty">
          <span class="transfer-empty-icon"><ListFilter /></span>
          <p>{{ rows.length ? t("transferFilter.empty") : t("noTransfers") }}</p>
        </div>
      </div>

      <input ref="resumeInput" type="file" class="hidden" @change="emit('resume-picked', $event)" />
    </PopoverContent>
  </Popover>
</template>

<style scoped>
.transfer-panel-head {
  display: flex;
  min-height: 42px;
  flex: 0 0 auto;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid var(--card-border);
  padding: 6px 8px 6px 12px;
  background: color-mix(in srgb, var(--card-surface-raised) 55%, transparent);
}
.transfer-panel-icon {
  display: inline-flex;
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  background: var(--tone-blue-soft);
  color: var(--tone-blue);
}
.transfer-panel-icon :deep(svg) { width: 14px; height: 14px; }
.transfer-panel-head h3 { margin: 0; overflow: hidden; flex: 0 1 auto; font-size: 12px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.transfer-panel-count {
  flex: 0 0 auto;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-pill);
  padding: 1px 7px;
  color: var(--muted-foreground);
  font-size: 9.5px;
  font-variant-numeric: tabular-nums;
}
.transfer-panel-actions { display: inline-flex; margin-left: auto; flex: 0 0 auto; align-items: center; gap: 2px; }
.transfer-panel-actions .icon-button { width: 26px; height: 26px; padding: 5px; color: var(--muted-foreground); }
.transfer-panel-actions svg { width: 14px; height: 14px; }

.transfer-filters {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  border-bottom: 1px solid var(--card-border);
  padding: 7px 10px;
  overflow-x: auto;
  scrollbar-width: none;
}
.transfer-filters::-webkit-scrollbar { display: none; }
.transfer-filter {
  display: inline-flex;
  height: 22px;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-pill);
  padding: 0 7px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 9.5px;
  transition: background var(--transition-fast), color var(--transition-fast), border-color var(--transition-fast);
}
.transfer-filter > svg { width: 11px; height: 11px; }
.transfer-filter b { font-size: 9px; font-variant-numeric: tabular-nums; }
.transfer-filter:hover { background: var(--card-surface-raised); color: var(--foreground); }
.transfer-filter.is-active {
  border-color: color-mix(in srgb, var(--primary) 45%, transparent);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  color: var(--foreground);
}
.transfer-filter.is-active > svg { color: var(--primary); }
.transfer-filter-label { line-height: 1; }

.transfer-scroll { min-height: 0; flex: 1 1 auto; overflow-y: auto; padding: 2px 8px 8px; }
.transfer-scroll::-webkit-scrollbar { display: block; width: 8px; }
.transfer-scroll::-webkit-scrollbar-thumb { border-radius: 4px; background: color-mix(in srgb, var(--foreground) 20%, transparent); }

.transfer-batch {
  position: relative;
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  margin: 6px 0;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-md);
  padding: 10px 12px 11px;
  background: var(--card-surface-inset);
  overflow: hidden;
}
.transfer-batch-icon {
  display: inline-flex;
  width: 34px;
  height: 34px;
  align-items: center;
  justify-content: center;
  border: 1px solid color-mix(in srgb, var(--tone-violet) 34%, transparent);
  border-radius: 9px;
  background: var(--tone-violet-soft);
  color: var(--tone-violet);
}
.transfer-batch-icon.is-file { border-color: color-mix(in srgb, var(--tone-blue) 34%, transparent); background: var(--tone-blue-soft); color: var(--tone-blue); }
.transfer-batch-icon > svg { width: 16px; height: 16px; }
.transfer-batch-body { display: flex; min-width: 0; flex-direction: column; gap: 4px; }
.transfer-batch-head { display: flex; min-width: 0; align-items: center; gap: 8px; }
.transfer-batch-title { min-width: 0; flex: 1; overflow: hidden; color: var(--foreground); font-size: 12px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.transfer-batch-percent { flex: 0 0 auto; color: var(--foreground); font-size: 10.5px; font-variant-numeric: tabular-nums; }
.transfer-batch-meta { color: var(--muted-foreground); font-size: 9.5px; font-variant-numeric: tabular-nums; }
.transfer-batch-file { overflow: hidden; color: var(--muted-foreground); font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
.transfer-batch-line { position: absolute; bottom: 0; left: 0; height: 2px; border-radius: var(--radius-pill); background: var(--tone-violet); transition: width 320ms cubic-bezier(0.4, 0, 0.2, 1); }

.transfer-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin: 8px 0 4px;
  border: 1px dashed color-mix(in srgb, var(--tone-amber) 45%, var(--card-border));
  border-radius: var(--radius-sm);
  padding: 6px 9px;
  color: var(--muted-foreground);
  font-size: 10px;
}
.transfer-empty { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 26px 12px 30px; color: var(--muted-foreground); text-align: center; }
.transfer-empty-icon { display: inline-flex; width: 34px; height: 34px; align-items: center; justify-content: center; border-radius: 10px; background: var(--card-surface-raised); color: var(--muted-foreground); }
.transfer-empty-icon > svg { width: 16px; height: 16px; }
.transfer-empty p { margin: 0; font-size: 10.5px; }
</style>
