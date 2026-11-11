<script lang="ts">
// 传输行展示组件：文件类型图标 + 名称 + 方向/协议徽标 + 状态与体积/时间 +
// 行内动作 + 2px 进度线；点击行展开详情（路径/错误）。动作由父级解析成
// 具体任务对象后执行，本组件只发意图事件。
import type { Component } from "vue";

export type { TransferRowView } from "../lib/transferPanelRows";
</script>

<script setup lang="ts">
import { computed } from "vue";
import {
  ArrowDown,
  ArrowUp,
  ExternalLink,
  File,
  FileArchive,
  FileCode2,
  FileImage,
  FileText,
  Folder,
  FolderOpen,
  Pause,
  Play,
  X,
} from "@lucide/vue";
import type { TransferRowView } from "../lib/transferPanelRows";

const props = defineProps<{
  row: TransferRowView;
  t: (key: string, values?: Record<string, string | number>) => string;
  expanded: boolean;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "pause"): void;
  (e: "cancel"): void;
  (e: "reveal"): void;
  (e: "open-file"): void;
  (e: "resume"): void;
  (e: "context-menu", event: MouseEvent): void;
}>();

const FILE_TYPES: Array<{ tone: string; icon: Component; pattern: RegExp }> = [
  { tone: "amber", icon: FileArchive, pattern: /\.(zip|tar|tgz|gz|bz2|xz|rar|7z)$/i },
  { tone: "violet", icon: FileImage, pattern: /\.(png|jpe?g|gif|webp|svg|bmp|ico|tiff?)$/i },
  {
    tone: "cyan",
    icon: FileCode2,
    pattern: /\.(json|ya?ml|toml|ini|conf|cfg|js|mjs|cjs|ts|tsx|vue|py|sh|bash|zsh|rs|go|java|k?t|c|cc|cpp|h|hpp|css|scss|html|xml|sql|service)$/i,
  },
  { tone: "blue", icon: FileText, pattern: /\.(md|txt|log|pdf|docx?|xlsx?|csv|pptx?|rtf)$/i },
];

const fileName = computed(() => {
  const name = props.row.fileName || props.row.taskId;
  return name.split(/[/\\]/).pop() || name;
});

const fileType = computed(() => {
  if (props.row.isFolder) return { tone: "amber", icon: Folder };
  return FILE_TYPES.find((entry) => entry.pattern.test(fileName.value)) ?? { tone: "neutral", icon: File };
});

const statusText = computed(() => {
  if (props.row.paused) return props.t("transferPaused");
  if (props.row.status === "resumable") return props.t("transferStatus.resumable");
  return props.t(`transferStatus.${props.row.status}`);
});

// 已完成只留圆点（参考面板的默认形态）；进行中/失败/取消/暂停显式写状态。
const statusVisible = computed(() =>
  props.row.paused || ["running", "queued", "staging", "failed", "cancelled", "resumable"].includes(props.row.status),
);

const statusClass = computed(() => (props.row.paused ? "is-paused" : `is-${props.row.status}`));
const detailTitle = computed(() =>
  props.row.timeLabel ? `${props.row.detailLabel} · ${props.row.timeLabel}` : props.row.detailLabel,
);
</script>

<template>
  <article
    class="transfer-row"
    :class="[statusClass, { 'is-expanded': expanded }]"
    @contextmenu="emit('context-menu', $event)"
  >
    <div class="transfer-row-summary">
      <button
        type="button"
        class="transfer-row-toggle"
        :aria-expanded="expanded"
        :aria-label="t(expanded ? 'transferCollapseRow' : 'transferExpandRow')"
        @click="emit('toggle')"
      >
        <span class="transfer-file-icon" :data-tone="fileType.tone">
          <component :is="fileType.icon" />
        </span>
        <span class="transfer-row-main">
          <span class="transfer-row-name" :title="row.fileName || row.taskId">{{ row.fileName || row.taskId }}</span>
          <span class="transfer-row-badges">
            <span class="transfer-dir" :class="`is-${row.direction}`">
              <ArrowUp v-if="row.direction === 'upload'" />
              <ArrowDown v-else />
              {{ t(`transferDirection.${row.direction}`) }}
            </span>
            <span class="transfer-proto">SFTP</span>
            <span v-if="row.compression" class="transfer-proto is-compressed" :title="t('transferCompress.badgeHint')">{{ t("transferCompress.badge") }}</span>
          </span>
          <span class="transfer-row-meta">
            <span class="transfer-row-status" :title="statusText">
              <i class="transfer-row-dot" />
              <template v-if="statusVisible">{{ statusText }}</template>
            </span>
            <span class="transfer-row-detail" :title="detailTitle">
              {{ row.detailLabel }}<template v-if="row.timeLabel"> · {{ row.timeLabel }}</template>
            </span>
            <span v-if="row.speedLabel" class="transfer-row-speed">{{ row.speedLabel }}</span>
          </span>
        </span>
      </button>
      <div class="transfer-row-actions" @click.stop>
        <button v-if="row.canOpenFile" type="button" class="icon-button" :title="t('openDownloadedFile')" :aria-label="t('openDownloadedFile')" @click="emit('open-file')"><ExternalLink /></button>
        <button v-if="row.canReveal" type="button" class="icon-button" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click="emit('reveal')"><FolderOpen /></button>
        <button v-if="row.canResume" type="button" class="icon-button" :title="t('resumableResume')" :aria-label="t('resumableResume')" @click="emit('resume')"><Play /></button>
        <button v-if="row.canPause" type="button" class="icon-button" :title="t(row.paused ? 'transferResume' : 'transferPause')" :aria-label="t(row.paused ? 'transferResume' : 'transferPause')" @click="emit('pause')"><Play v-if="row.paused" /><Pause v-else /></button>
        <button v-if="row.canCancel" type="button" class="icon-button is-danger" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel')"><X /></button>
      </div>
      <span class="transfer-row-line" :class="statusClass" :style="{ width: `${row.percent}%` }" />
    </div>

    <div v-if="expanded" class="transfer-detail">
      <div class="transfer-detail-head">
        <span class="transfer-detail-name" :title="row.fileName || row.taskId">{{ row.fileName || row.taskId }}</span>
        <span class="transfer-detail-state" :class="statusClass">{{ statusText }}<template v-if="row.timeLabel"> · {{ row.timeLabel }}</template></span>
      </div>
      <div class="transfer-detail-meta">{{ row.sizeLabel }}<template v-if="row.timeLabel"> · {{ row.timeLabel }}</template></div>
      <div v-if="row.localPath" class="transfer-detail-path">
        <span class="transfer-detail-path-label">{{ t("transferLocalPath") }}</span>
        <span class="transfer-detail-path-value mono" :title="row.localPath">{{ row.localPath }}</span>
      </div>
      <div v-if="row.remotePath" class="transfer-detail-path">
        <span class="transfer-detail-path-label">{{ t("transferRemotePath") }}</span>
        <span class="transfer-detail-path-value mono" :title="row.remotePath">{{ row.remotePath }}</span>
      </div>
      <div v-if="row.currentFile" class="transfer-detail-path">
        <span class="transfer-detail-path-label">{{ t("transferCurrentFile") }}</span>
        <span class="transfer-detail-path-value mono" :title="row.currentFile">{{ row.currentFile }}</span>
      </div>
      <p v-if="row.error" class="transfer-detail-error">{{ row.error }}</p>
    </div>
  </article>
</template>

<style scoped>
.transfer-row { position: relative; }
.transfer-row + .transfer-row { border-top: 1px solid color-mix(in srgb, var(--card-border) 72%, transparent); }
.transfer-row-summary {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 3px;
  padding: 5px 3px 6px 2px;
  transition: background var(--transition-fast);
}
.transfer-row:hover .transfer-row-summary { background: color-mix(in srgb, var(--card-surface-raised) 62%, transparent); }
.transfer-row.is-expanded .transfer-row-summary { background: color-mix(in srgb, var(--card-surface-raised) 72%, transparent); }
.transfer-row-toggle {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 7px;
  border: 0;
  border-radius: var(--radius-sm);
  padding: 0;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}
.transfer-row-toggle:focus-visible { outline: 1px solid var(--primary); outline-offset: 2px; }

.transfer-file-icon {
  display: inline-flex;
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--card-border);
  border-radius: 7px;
  background: var(--card-surface-raised);
  color: var(--muted-foreground);
}
.transfer-file-icon > svg { width: 12px; height: 12px; }
.transfer-file-icon[data-tone="amber"] { border-color: color-mix(in srgb, var(--tone-amber) 34%, transparent); background: var(--tone-amber-soft); color: var(--tone-amber); }
.transfer-file-icon[data-tone="blue"] { border-color: color-mix(in srgb, var(--tone-blue) 34%, transparent); background: var(--tone-blue-soft); color: var(--tone-blue); }
.transfer-file-icon[data-tone="cyan"] { border-color: color-mix(in srgb, var(--tone-cyan) 34%, transparent); background: var(--tone-cyan-soft); color: var(--tone-cyan); }
.transfer-file-icon[data-tone="violet"] { border-color: color-mix(in srgb, var(--tone-violet) 34%, transparent); background: var(--tone-violet-soft); color: var(--tone-violet); }

.transfer-row-main { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.transfer-row-name {
  overflow: hidden;
  color: var(--foreground);
  font-size: var(--text-sm);
  font-weight: 600;
  letter-spacing: -0.005em;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.transfer-row-badges { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
.transfer-dir {
  display: inline-flex;
  height: 15px;
  align-items: center;
  gap: 2px;
  border: 1px solid;
  border-radius: var(--radius-pill);
  padding: 0 5px;
  font-size: 9.5px;
  font-weight: 600;
}
.transfer-dir > svg { width: 9px; height: 9px; }
.transfer-dir.is-upload { border-color: color-mix(in srgb, var(--tone-emerald) 45%, transparent); background: var(--tone-emerald-soft); color: var(--tone-emerald); }
.transfer-dir.is-download { border-color: color-mix(in srgb, var(--tone-blue) 45%, transparent); background: var(--tone-blue-soft); color: var(--tone-blue); }
.transfer-proto {
  display: inline-flex;
  height: 15px;
  align-items: center;
  border: 1px solid var(--card-border);
  border-radius: 4px;
  padding: 0 5px;
  color: var(--muted-foreground);
  font-size: 9.5px;
  letter-spacing: 0.02em;
}
.transfer-proto.is-compressed { border-color: color-mix(in srgb, var(--tone-violet) 40%, transparent); color: var(--tone-violet); }
.transfer-row-meta {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px 6px;
  color: var(--muted-foreground);
  font-size: var(--text-2xs);
  font-variant-numeric: tabular-nums;
}
.transfer-row-status { display: inline-flex; align-items: center; gap: 4px; white-space: nowrap; }
.transfer-row-dot { width: 5px; height: 5px; flex: 0 0 5px; border-radius: 50%; background: var(--muted-foreground); }
.transfer-row.is-running .transfer-row-dot { background: var(--tone-blue); animation: transfer-pulse 1.9s ease-out infinite; }
.transfer-row.is-staging .transfer-row-dot { background: var(--tone-violet); }
.transfer-row.is-completed .transfer-row-dot { background: var(--tone-emerald); }
.transfer-row.is-failed .transfer-row-dot { background: var(--tone-rose); }
.transfer-row.is-cancelled .transfer-row-dot { background: color-mix(in srgb, var(--muted-foreground) 70%, transparent); }
.transfer-row.is-paused .transfer-row-dot,
.transfer-row.is-resumable .transfer-row-dot { background: var(--tone-amber); }
.transfer-row.is-failed .transfer-row-status { color: var(--tone-rose); }
.transfer-row-detail { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.transfer-row-speed { color: var(--tone-blue); }

.transfer-row-actions {
  display: flex;
  align-items: center;
  gap: 1px;
  opacity: 0.72;
  transition: opacity var(--transition-fast);
}
.transfer-row:hover .transfer-row-actions,
.transfer-row:focus-within .transfer-row-actions { opacity: 1; }
.transfer-row-actions .icon-button {
  width: 20px;
  height: 20px;
  min-width: 20px;
  padding: 3px;
  border-radius: var(--radius-xs);
  color: var(--muted-foreground);
}
.transfer-row-actions .icon-button:hover:not(:disabled) { color: var(--foreground); background: color-mix(in srgb, var(--foreground) 10%, transparent); box-shadow: none; }
.transfer-row-actions .icon-button.is-danger:hover:not(:disabled) { color: var(--tone-rose); background: var(--tone-rose-soft); }
.transfer-row-actions svg { width: 11px; height: 11px; }

.transfer-row-line {
  position: absolute;
  bottom: 0;
  left: 0;
  height: 2px;
  border-radius: var(--radius-pill);
  background: var(--tone-blue);
  transition: width 320ms cubic-bezier(0.4, 0, 0.2, 1);
}
/* 已完成的行只需要一个“收尾”提示：细线 + 低饱和，避免整列 100% 绿条抢走
   进行中任务的注意力（进度线的主语是运行中的传输）。 */
.transfer-row-line.is-completed {
  height: 1px;
  background: color-mix(in srgb, var(--tone-emerald) 42%, transparent);
}
.transfer-row-line.is-failed { background: var(--tone-rose); }
.transfer-row-line.is-cancelled { background: color-mix(in srgb, var(--muted-foreground) 40%, transparent); }
.transfer-row-line.is-paused,
.transfer-row-line.is-resumable { background: var(--tone-amber); }
.transfer-row-line.is-staging { background: var(--tone-violet); }

.transfer-detail {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin: 0 2px 7px 31px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  padding: 7px 9px;
  background: var(--card-surface-inset);
}
.transfer-detail-head { display: flex; min-width: 0; align-items: center; gap: 7px; }
.transfer-detail-name { min-width: 0; flex: 1; overflow: hidden; color: var(--foreground); font-size: var(--text-xs); font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.transfer-detail-state {
  flex: 0 0 auto;
  border-radius: var(--radius-pill);
  padding: 1px 6px;
  background: var(--tone-blue-soft);
  color: var(--tone-blue);
  font-size: 9.5px;
  font-variant-numeric: tabular-nums;
}
.transfer-detail-state.is-completed { background: var(--tone-emerald-soft); color: var(--tone-emerald); }
.transfer-detail-state.is-failed { background: var(--tone-rose-soft); color: var(--tone-rose); }
.transfer-detail-state.is-cancelled { background: color-mix(in srgb, var(--muted-foreground) 14%, transparent); color: var(--muted-foreground); }
.transfer-detail-state.is-paused,
.transfer-detail-state.is-resumable { background: var(--tone-amber-soft); color: var(--tone-amber); }
.transfer-detail-meta { color: var(--muted-foreground); font-size: var(--text-2xs); font-variant-numeric: tabular-nums; }
.transfer-detail-path {
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  padding: 5px 7px;
  background: var(--card-surface);
}
.transfer-detail-path-label { color: var(--muted-foreground); font-size: 9.5px; letter-spacing: 0.04em; text-transform: uppercase; }
.transfer-detail-path-value { color: var(--foreground); font-size: var(--text-2xs); overflow-wrap: anywhere; }
.transfer-detail-error { margin: 0; color: var(--tone-rose); font-size: var(--text-2xs); overflow-wrap: anywhere; }

@keyframes transfer-pulse {
  0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--tone-blue) 35%, transparent); }
  70% { box-shadow: 0 0 0 4px transparent; }
  100% { box-shadow: 0 0 0 0 transparent; }
}
@media (prefers-reduced-motion: reduce) {
  .transfer-row.is-running .transfer-row-dot { animation: none; }
  .transfer-row-line { transition: none; }
}
@media (max-width: 340px) {
  .transfer-detail { margin-left: 0; }
}
</style>
