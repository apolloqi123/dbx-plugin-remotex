<script setup lang="ts">
// 容器文件页：面包屑 / 上一级 / 刷新 / 上传 + 条目列表（图标、名称、选中后的
// stat 详情）。目录双击进入；文件单击选中、双击下载（走 useContainerFileTransfer）。
// 传输入口：SSH 走 export → SFTP 队列 → cleanup / prepareUpload → 上传队列 →
// import；本机走 saveLocal 与分片上传（256 KiB）。只读连接与缺 ls 的容器禁用
// 上传/导入；512 MiB 上限在创建暂存前拒绝。
import { computed, onUnmounted, ref } from "vue";
import { ArrowUp, Download, File, Folder, Loader2, RefreshCw, Upload, X } from "@lucide/vue";
import type { DockerContainer } from "../lib/dockerPanelModel";
import { useContainerFiles } from "../composables/useContainerFiles";
import { useContainerFileTransfer, type ContainerTransferAction } from "../composables/useContainerFileTransfer";
import { getDockerTransferBridge } from "../lib/dockerFileTransfer";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import { formatBytes } from "../../../shared/lib/format";
import { confirmDialog } from "../../../shared/lib/confirmDialog";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
}>();

const files = useContainerFiles({
  t: props.t,
  containerId: () => props.container.id,
  sessionId: () => props.sessionId,
  connectionId: () => props.connectionId,
});

const transferError = ref("");
const notice = ref<{ message: string; actions: ContainerTransferAction[] }>();

function showError(cause: unknown): void {
  transferError.value = cause instanceof Error ? cause.message : String(cause);
}

function showNotice(message: string, actions?: ContainerTransferAction[]): void {
  notice.value = { message, actions: actions ?? [] };
}

/** 传输桥（SFTP 下载链 + 上传队列）由 setup 期登记；缺失时按钮保持禁用。 */
const bridge = getDockerTransferBridge();
const transferReady = computed(() => !!bridge);

const transfer = useContainerFileTransfer({
  t: props.t,
  sessionId: () => props.sessionId,
  containerId: () => props.container.id,
  currentPath: () => files.path.value,
  canWrite: () => props.canWrite !== false,
  showError,
  showNotice,
  downloadRemoteFile: bridge?.downloadRemoteFile ?? (async () => {
    throw new Error(props.t("docker.filesUnsupported"));
  }),
  uploadSource: bridge?.uploadSource ?? (async () => false),
  confirmDialog: (message) => confirmDialog(message, { danger: false }),
  engineParams: () => dockerEngineParams(loadDockerEngineSettings(props.connectionId || "")),
});

const canWrite = computed(() => props.canWrite !== false);
const uploadDisabled = computed(() => !canWrite.value || files.toolMissing.value || !transferReady.value);
const uploadTitle = computed(() => {
  if (!transferReady.value || !canWrite.value) return props.t("docker.filesUnsupported");
  return props.t("docker.filesUpload");
});
const uploadInput = ref<HTMLInputElement>();

function openEntry(entry: { name: string; path: string; kind: "dir" | "file" }): void {
  if (entry.kind === "dir") void files.enter(entry);
  else if (transferReady.value) void runDownload(entry);
  else void files.select(entry);
}

function downloadTitle(kind: "dir" | "file"): string {
  if (kind === "dir") return props.t("docker.filesDirNotDownloadable");
  return transferReady.value ? props.t("docker.filesDownload") : props.t("docker.filesUnsupported");
}

async function runDownload(entry: { name: string; path: string; kind?: "dir" | "file"; size?: number }): Promise<void> {
  transferError.value = "";
  try {
    await transfer.download(entry);
  } catch {
    // 失败已由 showError 进入横幅。
  }
}

function chooseUpload(): void {
  uploadInput.value?.click();
}

async function onUploadPicked(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement | null;
  const picked = input?.files?.[0];
  if (input) input.value = "";
  if (!picked) return;
  transferError.value = "";
  try {
    await transfer.upload(picked);
    await files.refresh();
  } catch {
    // 失败已由 showError 进入横幅。
  }
}

function selectRow(entry: { name: string; path: string; kind: "dir" | "file" }): void {
  transferError.value = "";
  void files.select(entry);
}

/** 选中文件的 stat 摘要；stat 缺失/失败显示 “—”。 */
function metaText(): string {
  const meta = files.selectedMeta.value;
  if (files.selectedMetaLoading.value) return "…";
  if (!meta || meta.unavailable) return "—";
  return meta.size === null ? "—" : formatBytes(meta.size);
}

onUnmounted(() => transfer.dispose());
</script>

<template>
  <div class="container-files-tab" data-tab-panel="files">
    <div class="docker-files-toolbar">
      <nav class="docker-files-breadcrumb">
        <button
          v-for="crumb in files.breadcrumbs.value"
          :key="crumb.path"
          type="button"
          class="docker-file-crumb"
          :data-crumb="crumb.path"
          @click="void files.enter(crumb.path)"
        >
          {{ crumb.name }}
        </button>
      </nav>
      <span class="docker-files-spacer" />
      <button
        type="button"
        class="icon-button compact"
        data-action="parent"
        :title="t('docker.filesParent')"
        :aria-label="t('docker.filesParent')"
        :disabled="files.path.value === '/'"
        @click="void files.parent()"
      >
        <ArrowUp />
      </button>
      <button
        type="button"
        class="icon-button compact"
        data-action="refresh"
        :title="t('docker.filesRefresh')"
        :aria-label="t('docker.filesRefresh')"
        :disabled="files.loading.value"
        @click="void files.refresh()"
      >
        <RefreshCw />
      </button>
      <button
        type="button"
        class="icon-button compact"
        data-action="upload"
        :title="uploadTitle"
        :aria-label="t('docker.filesUpload')"
        :disabled="uploadDisabled"
        @click="chooseUpload"
      >
        <Upload />
      </button>
      <input
        ref="uploadInput"
        type="file"
        class="docker-files-upload-input"
        data-files-upload-input
        @change="onUploadPicked"
      />
    </div>

    <p v-if="files.error.value" class="docker-files-error" data-files-error role="alert">
      {{ files.error.value }}
    </p>
    <p v-else-if="transferError" class="docker-files-error" data-files-transfer-error role="alert">
      {{ t("docker.filesTransferFailed") }}: {{ transferError }}
    </p>
    <p v-else-if="files.loading.value" class="docker-files-status" data-files-loading>
      <Loader2 class="spin" />
      {{ t("docker.filesDownloading") }}
    </p>
    <p v-else-if="!files.entries.value.length" class="docker-files-empty" data-files-empty>
      {{ t("docker.filesEmpty") }}
    </p>
    <ul v-else class="docker-files-list">
      <li
        v-for="entry in files.entries.value"
        :key="entry.path"
        class="docker-file-row"
        :class="{ 'is-dir': entry.kind === 'dir', 'is-selected': files.selected.value?.path === entry.path }"
        :data-name="entry.name"
        :data-kind="entry.kind"
        :title="entry.name"
        @click="selectRow(entry)"
        @dblclick="openEntry(entry)"
      >
        <Folder v-if="entry.kind === 'dir'" class="docker-file-icon" />
        <File v-else class="docker-file-icon" />
        <span class="docker-file-name">{{ entry.name }}</span>
        <span v-if="entry.kind === 'file' && files.selected.value?.path === entry.path" class="docker-file-meta">
          {{ metaText() }}
        </span>
        <button
          type="button"
          class="icon-button compact docker-file-download"
          :data-download="entry.name"
          :title="downloadTitle(entry.kind)"
          :aria-label="t('docker.filesDownload')"
          :disabled="entry.kind === 'dir' || !transferReady"
          @click.stop="void runDownload(entry)"
        >
          <Loader2 v-if="transfer.busyPath.value === entry.path" class="spin" />
          <Download v-else />
        </button>
      </li>
    </ul>

    <div v-if="transfer.uploading.value" class="docker-files-progress" data-upload-progress>
      <Loader2 class="spin" />
      <span class="docker-files-progress-name">{{ t("docker.filesUploading") }} · {{ transfer.uploading.value.name }}</span>
      <progress :value="transfer.uploading.value.percent" max="100" />
      <span>{{ transfer.uploading.value.percent }}%</span>
      <button
        type="button"
        class="icon-button compact"
        data-action="cancel-upload"
        :title="t('docker.filesCancel')"
        :aria-label="t('docker.filesCancel')"
        @click="void transfer.cancelUpload()"
      >
        <X />
      </button>
    </div>

    <div v-if="notice" class="docker-files-notice" data-files-notice>
      <span class="docker-files-notice-text">{{ notice.message }}</span>
      <button
        v-for="action in notice.actions"
        :key="action.label"
        type="button"
        class="docker-files-notice-action"
        @click="action.run()"
      >
        {{ action.label }}
      </button>
      <button
        type="button"
        class="icon-button compact"
        data-action="dismiss-notice"
        :title="t('docker.close')"
        :aria-label="t('docker.close')"
        @click="notice = undefined"
      >
        <X />
      </button>
    </div>
  </div>
</template>

<style scoped>
.container-files-tab {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.docker-files-toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
  padding: 4px 8px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.docker-files-breadcrumb {
  display: flex;
  align-items: center;
  gap: 2px;
  min-width: 0;
  overflow: auto hidden;
  white-space: nowrap;
}

.docker-file-crumb {
  padding: 1px 4px;
  border: 0;
  border-radius: 3px;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
  opacity: 0.8;
}

.docker-file-crumb:hover {
  background: var(--muted);
  opacity: 1;
}

.docker-files-spacer {
  flex: 1;
}

.docker-files-status,
.docker-files-empty {
  margin: auto;
  padding: 12px;
  font-size: 12px;
  opacity: 0.65;
}

.docker-files-error {
  margin: 8px;
  padding: 6px 8px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--destructive) 12%, transparent);
  color: var(--destructive);
  font-size: 12px;
}

.docker-files-list {
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 0;
  overflow: auto;
  list-style: none;
}

.docker-file-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  font-size: 12px;
  cursor: default;
}

.docker-file-row:hover {
  background: var(--muted);
}

.docker-file-row.is-selected {
  background: color-mix(in srgb, var(--primary) 14%, transparent);
}

.docker-file-icon {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  opacity: 0.75;
}

.docker-file-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.docker-file-meta {
  flex: 0 0 auto;
  font-size: 11px;
  opacity: 0.6;
}

.docker-file-download {
  flex: 0 0 auto;
}

.docker-files-upload-input {
  display: none;
}

.docker-files-progress {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
  padding: 4px 8px;
  border-top: 1px solid var(--border);
  font-size: 11px;
}

.docker-files-progress-name {
  max-width: 45%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.docker-files-progress progress {
  flex: 1;
  height: 6px;
}

.docker-files-notice {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
  padding: 4px 8px;
  border-top: 1px solid var(--border);
  font-size: 11px;
}

.docker-files-notice-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.docker-files-notice-action {
  border: 0;
  background: none;
  color: var(--primary);
  font: inherit;
  cursor: pointer;
}

.spin {
  animation: docker-files-spin 1s linear infinite;
}

@keyframes docker-files-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
