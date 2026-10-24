<script setup lang="ts">
import { ref } from "vue";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "../../shared/ui/context-menu/index";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import { Download, FileText, FileUp, FolderOpen, FolderUp, Loader2, Pause, Play, RefreshCw, Trash2, X } from "@lucide/vue";
import type { TransferHistoryEntry } from "../../features/transfers/lib/transferHistory";
import type { ResumableUploadTask } from "../../features/transfers/lib/transferResume";
import type { BatchUploadSummary, TransferTask } from "../../features/transfers/lib/transferQueue";
import type { FolderUploadProgress } from "../../features/sftp/lib/folderUpload";

/** 传输弹层（tab/panel 两表面共用单实例）：活跃任务、文件夹/批次聚合卡、
 *  断点续传与落盘历史。队列/历史状态仍由 App 的 useTransferQueue /
 *  useTransferHistory 持有；本组件只渲染并上抛动作。
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
  transferBarValue: (task: TransferTask) => number | undefined;
  transferShownBytes: (task: TransferTask) => number;
  transferBytesTitle: (task: TransferTask) => string | undefined;
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
            <button v-if="transferPausable(task.status)" class="icon-button" :title="t(pausedTaskIds.has(task.taskId) ? 'transferResume' : 'transferPause')" :aria-label="t(pausedTaskIds.has(task.taskId) ? 'transferResume' : 'transferPause')" @click="emit('toggle-pause', task)"><Play v-if="pausedTaskIds.has(task.taskId)" /><Pause v-else /></button>
            <button v-if="task.status === 'queued' || task.status === 'running'" class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel', task)"><X /></button>
            <button v-if="task.localPath" class="icon-button" :title="t('revealInFolder')" :aria-label="t('revealInFolder')" @click="emit('reveal', task.localPath)"><FolderOpen /></button>
            <button v-if="task.localPath" class="icon-button" :title="t('openDownloadedFile')" :aria-label="t('openDownloadedFile')" @click="emit('open-file', task.localPath)"><FileText /></button>
          </div>
          <p v-if="task.error" class="task-error">{{ task.error }}</p>
        </article>
        <!-- 保留上传断点续传入口；文件选择器本身始终隐藏，仅由 Resume 按钮唤起。 -->
        <template v-if="resumableTasks.length">
          <h3 class="transfer-history-title">{{ t("resumableTitle") }}</h3>
          <article v-for="task in resumableTasks" :key="task.taskId" class="transfer-card">
            <div class="transfer-title"><FileUp /><span :title="task.remotePath">{{ task.fileName }}</span></div>
            <div class="transfer-meta"><span>{{ formatBytes(task.resumableBytes) }} / {{ formatBytes(task.size) }}</span></div>
            <div class="transfer-actions"><button class="icon-button" :title="t('resumableResume')" :aria-label="t('resumableResume')" @click="beginResume(task)"><Play /></button></div>
          </article>
        </template>
        <input ref="resumeInput" type="file" class="hidden" @change="emit('resume-picked', $event)" />
        <!-- 传输历史与活跃任务并列展示：历史是落盘快照，不应被当前传输状态遮住。 -->
        <div class="transfer-history-head">
          <h3 class="transfer-history-title">{{ t("transfersHistory.title") }}</h3>
          <span class="transfer-history-actions">
            <button type="button" class="icon-button" :title="t('refresh')" :aria-label="t('refresh')" :disabled="transferHistoryLoading || resumableLoading" @click.stop="emit('refresh')"><RefreshCw :class="{ spinning: transferHistoryLoading || resumableLoading }" /></button>
            <button type="button" class="icon-button" :title="t('transfersHistory.clear')" :aria-label="t('transfersHistory.clear')" :disabled="!transferHistory.length" @click.stop="emit('open-clear')"><Trash2 /></button>
          </span>
        </div>
        <div v-if="transferHistoryFailed" class="empty compact">
          <span>{{ t("transfersHistory.loadFailed") }}</span>
          <button type="button" class="link-button" @click.stop="emit('refresh')">{{ t("refresh") }}</button>
        </div>
        <div v-else-if="transferHistoryLoading && !transferHistory.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="!transferHistory.length" class="empty compact">{{ t("transfersHistory.empty") }}</div>
        <ContextMenu v-for="entry in transferHistory" :key="entry.taskId" :open="transferHistoryMenu?.taskId === entry.taskId" @update:open="(open) => { if (!open && transferHistoryMenu?.taskId === entry.taskId) emit('close-history-menu'); }">
          <ContextMenuTrigger as-child>
            <article class="transfer-card transfer-history-card" @contextmenu="emit('show-history-menu', $event, entry)">
              <div class="transfer-title"><FileUp v-if="entry.direction === 'upload'" /><Download v-else /><span :title="entry.fileName">{{ entry.fileName || entry.taskId }}</span></div>
              <div class="transfer-meta"><span>{{ t(`transferStatus.${entry.status}`) }}</span><span>{{ formatBytes(entry.size) }}</span></div>
              <p v-if="entry.localPath" class="transfer-path mono" :title="entry.localPath">{{ entry.localPath }}</p>
              <p v-if="entry.error" class="task-error">{{ entry.error }}</p>
            </article>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem @select="entry.localPath && emit('reveal', entry.localPath)"><FolderOpen />{{ t("revealInFolder") }}</ContextMenuItem>
            <ContextMenuItem @select="entry.localPath && emit('open-file', entry.localPath)"><FileText />{{ t("openDownloadedFile") }}</ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
        </PopoverContent>
  </Popover>
</template>

<style scoped>
/* 传输历史区标题：与任务卡片间的分隔线（根壳/卡片样式在 style.css 全局）。 */
.transfer-history-head { display: flex; align-items: center; justify-content: space-between; gap: 6px; border-top: 1px solid var(--border); margin-top: 10px; }
.transfer-history-title { margin: 0; padding-top: 8px; }
.transfer-history-actions { display: flex; gap: 2px; padding-top: 6px; }
</style>
