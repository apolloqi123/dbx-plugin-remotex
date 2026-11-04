<script setup lang="ts">
// RDP 文件/文件夹剪贴板浮层：远端复制文件的接收进度与 zip 保存入口
// （宿主拖拽由会话侧监听）。只做展示与动作转发，状态由
// useRdpFileClipboard 持有。
import { computed } from "vue";
import { Download, FileArchive, Loader2, X } from "@lucide/vue";
import type {
  ClipboardTransferProgress,
  RemoteClipboardFile,
} from "../../features/remote-desktop/lib/clipboardFiles";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  busy: boolean;
  progress: ClipboardTransferProgress | null;
  remoteFiles: { files: RemoteClipboardFile[]; totalBytes: number } | null;
  archiveReady: { name: string; size: number; fileCount: number } | null;
  remoteReady: { fileCount: number; pasteboard: boolean } | null;
  startRemoteDownload: () => void | Promise<boolean>;
  saveRemoteArchive: () => void | Promise<boolean>;
  dismissRemoteFiles: () => void | Promise<void>;
}>();

const emit = defineEmits<{ "focus-desktop": [] }>();

const transferLabel = computed(() => {
  const progress = props.progress;
  if (!progress) return "";
  if (progress.phase === "saving") return props.t("rdp.clipboardFiles.saving");
  if (progress.phase === "downloading") return props.t("rdp.clipboardFiles.downloading");
  return props.t("rdp.clipboardFiles.uploading");
});

const transferPercent = computed(() => props.progress?.percent ?? 0);
const fileCount = computed(
  () =>
    props.archiveReady?.fileCount ??
    props.remoteReady?.fileCount ??
    props.remoteFiles?.files.length ??
    0,
);
const cardVisible = computed(
  () => !!(props.archiveReady || props.remoteReady || props.remoteFiles),
);
const statusHint = computed(() => {
  if (props.remoteReady?.pasteboard) return props.t("rdp.clipboardFiles.pasteboardReadyShort");
  if (props.remoteReady) return props.t("rdp.clipboardFiles.downloaded");
  if (props.progress) return props.t("rdp.clipboardFiles.downloading");
  return props.t("rdp.clipboardFiles.available");
});

function startDownload() {
  void Promise.resolve(props.startRemoteDownload());
}

function saveAndFocus() {
  void Promise.resolve(props.saveRemoteArchive()).finally(() => emit("focus-desktop"));
}

function dismissAndFocus() {
  void Promise.resolve(props.dismissRemoteFiles()).finally(() => emit("focus-desktop"));
}
</script>

<template>
  <div class="rdp-files-overlay">
    <div v-if="progress" class="rdp-files-progress" role="status" aria-live="polite">
      <span>{{ transferLabel }}</span>
      <span
        class="rdp-files-bar"
        role="progressbar"
        :aria-label="transferLabel"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-valuenow="transferPercent"
      ><i :style="{ width: `${transferPercent}%` }" /></span>
      <span>{{ transferPercent }}%</span>
    </div>
    <div v-if="cardVisible" class="rdp-files-card">
      <FileArchive :size="14" aria-hidden="true" />
      <span class="rdp-files-meta">
        <strong>{{ t("rdp.clipboardFiles.count", { count: fileCount }) }}</strong>
        <small>{{ statusHint }}</small>
      </span>
      <button
        v-if="archiveReady || remoteReady"
        type="button"
        class="rdp-files-action"
        :disabled="busy"
        @click="saveAndFocus"
      >
        <Loader2 v-if="busy && progress?.phase === 'saving'" class="rdp-files-spin" :size="13" aria-hidden="true" />
        <Download v-else :size="13" aria-hidden="true" />
        {{ t("rdp.clipboardFiles.download") }}
      </button>
      <button
        v-else-if="remoteFiles && !progress"
        type="button"
        class="rdp-files-action"
        :disabled="busy"
        @click="startDownload"
      >
        <Download :size="13" aria-hidden="true" />
        {{ t("rdp.clipboardFiles.downloadFiles") }}
      </button>
      <button
        type="button"
        class="rdp-files-close"
        :disabled="busy"
        :title="t('rdp.clipboardFiles.dismiss')"
        :aria-label="t('rdp.clipboardFiles.dismiss')"
        @click="dismissAndFocus"
      >
        <X :size="12" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.rdp-files-overlay {
  position: absolute;
  z-index: 4;
  top: 8px;
  right: 8px;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
  pointer-events: none;
}
.rdp-files-overlay > * { pointer-events: auto; }
.rdp-files-action,
.rdp-files-close {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 4px 9px;
  background: color-mix(in srgb, var(--popover) 92%, transparent);
  color: var(--foreground);
  font-size: 11px;
  line-height: 1;
  box-shadow: 0 4px 14px color-mix(in srgb, #000 14%, transparent);
  cursor: pointer;
}
.rdp-files-action:hover:not(:disabled),
.rdp-files-close:hover { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.rdp-files-action:disabled,
.rdp-files-close:disabled { opacity: .42; cursor: default; }
.rdp-files-close { padding: 5px; border-radius: 50%; }
.rdp-files-close:disabled { pointer-events: none; }
.rdp-files-card {
  display: flex;
  align-items: center;
  gap: 7px;
  max-width: 340px;
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 6px 8px;
  background: color-mix(in srgb, var(--popover) 94%, transparent);
  box-shadow: 0 6px 18px color-mix(in srgb, #000 16%, transparent);
  font-size: 11px;
}
.rdp-files-meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.rdp-files-meta strong { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rdp-files-meta small { color: var(--muted-foreground); }
.rdp-files-progress {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 170px;
  border: 1px solid var(--border);
  border-radius: 999px;
  padding: 4px 9px;
  background: color-mix(in srgb, var(--popover) 92%, transparent);
  font-size: 11px;
}
.rdp-files-bar {
  flex: 1;
  height: 4px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--foreground) 14%, transparent);
}
.rdp-files-bar i {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--primary);
  transition: width .15s linear;
}
.rdp-files-spin { animation: rdp-files-spin 1s linear infinite; }
@keyframes rdp-files-spin { to { transform: rotate(360deg); } }
</style>