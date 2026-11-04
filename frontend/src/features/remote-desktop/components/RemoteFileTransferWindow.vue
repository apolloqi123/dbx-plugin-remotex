<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from "vue";
import {
  ArrowLeft,
  ArrowRight,
  Copy,
  Download,
  FolderOpen,
  Loader2,
  Server,
  Upload,
  X,
} from "@lucide/vue";
import FileTransferPane from "./FileTransferPane.vue";
import PasswordField from "../../../shared/components/PasswordField.vue";
import { formatBytes } from "../../../shared/lib/format";
import type {
  RemoteFileTransferController,
  TransferEntry,
  TransferPaneState,
} from "../composables/useRemoteFileTransfer";

const props = defineProps<{
  open: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  controller: RemoteFileTransferController;
}>();

const emit = defineEmits<{ "update:open": [boolean] }>();

const c = props.controller;
const t = props.t;

const connected = computed(() => c.session.value !== null);
const remoteTitle = computed(() =>
  c.session.value ? `${c.session.value.username}@${c.session.value.host}` : t("remoteFiles.remotePane"),
);
const progress = computed(() => c.progress.value);
const running = computed(() => progress.value?.status === "running");
const progressLabel = computed(() => {
  const current = progress.value;
  if (!current) return "";
  if (current.status === "done") return t("remoteFiles.done");
  if (current.status === "canceled") return t("remoteFiles.canceled");
  if (current.status === "error") return t("remoteFiles.failed");
  return current.direction === "upload" ? t("remoteFiles.uploading") : t("remoteFiles.downloading");
});
const progressDetail = computed(() => {
  const current = progress.value;
  if (!current) return "";
  const parts = [];
  if (current.file) parts.push(current.file);
  if (current.filesTotal > 0) parts.push(`${current.filesDone}/${current.filesTotal}`);
  if (current.bytesTotal > 0) {
    parts.push(`${formatBytes(current.bytesDone)} / ${formatBytes(current.bytesTotal)}`);
  }
  return parts.join(" · ");
});
const showClipboardAction = computed(
  () => c.clipboardSupported.value && c.lastDownloaded.value.length > 0,
);
const hasKeyAuth = computed(
  () => c.form.authentication === "private-key" || c.form.authentication === "private-key-password",
);

watch(
  () => props.open,
  (open) => {
    if (open) c.open();
  },
  { immediate: true },
);

function close() {
  emit("update:open", false);
}

function onKeydown(event: KeyboardEvent) {
  if (props.open && event.key === "Escape") {
    event.preventDefault();
    close();
  }
}

onMounted(() => document.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => document.removeEventListener("keydown", onKeydown));

function toggle(pane: TransferPaneState, entry: TransferEntry) {
  c.toggleSelection(pane, entry);
}

function upload() {
  c.upload();
}

function download() {
  c.download();
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="rft-backdrop" @pointerdown.self="close">
      <section class="rft-window" role="dialog" aria-modal="true" :aria-label="t('remoteFiles.title')">
        <header class="rft-head">
          <span class="rft-mark"><Server :size="15" aria-hidden="true" /></span>
          <div class="rft-head-text">
            <strong>{{ t("remoteFiles.title") }}</strong>
            <small>{{ connected ? remoteTitle : t("remoteFiles.subtitle") }}</small>
          </div>
          <button type="button" class="icon-button" :title="t('close')" :aria-label="t('close')" @click="close"><X :size="14" /></button>
        </header>

        <div v-if="!connected" class="rft-connect">
          <p class="rft-intro">{{ t("remoteFiles.intro") }}</p>
          <div class="rft-grid">
            <label class="rft-field">
              <span>{{ t("remoteFiles.hostLabel") }}</span>
              <input v-model="c.form.host" class="mono" spellcheck="false" :placeholder="t('remoteFiles.hostPlaceholder')" @keydown.enter="c.connect()" />
            </label>
            <label class="rft-field">
              <span>{{ t("remoteFiles.portLabel") }}</span>
              <input v-model="c.form.port" class="mono" inputmode="numeric" spellcheck="false" @keydown.enter="c.connect()" />
            </label>
            <label class="rft-field">
              <span>{{ t("remoteFiles.usernameLabel") }}</span>
              <input v-model="c.form.username" class="mono" spellcheck="false" autocomplete="username" />
            </label>
            <label class="rft-field">
              <span>{{ t("remoteFiles.authentication") }}</span>
              <select v-model="c.form.authentication">
                <option value="password">{{ t("remoteFiles.auth.password") }}</option>
                <option value="private-key">{{ t("remoteFiles.auth.privateKey") }}</option>
                <option value="private-key-password">{{ t("remoteFiles.auth.privateKeyPassword") }}</option>
                <option value="agent">{{ t("remoteFiles.auth.agent") }}</option>
                <option value="none">{{ t("remoteFiles.auth.none") }}</option>
              </select>
            </label>
            <label v-if="c.form.authentication === 'password'" class="rft-field rft-field-wide">
              <span>{{ t("remoteFiles.password") }}</span>
              <PasswordField v-model="c.form.password" :t="t" :placeholder="t('remoteFiles.passwordPlaceholder')" />
            </label>
            <label v-if="hasKeyAuth" class="rft-field rft-field-wide">
              <span>{{ t("remoteFiles.privateKeyPath") }}</span>
              <input v-model="c.form.privateKeyPath" class="mono" spellcheck="false" placeholder="/home/user/.ssh/id_ed25519" />
            </label>
            <label v-if="c.form.authentication === 'private-key-password'" class="rft-field rft-field-wide">
              <span>{{ t("remoteFiles.passphrase") }}</span>
              <PasswordField v-model="c.form.privateKeyPassphrase" :t="t" :placeholder="t('remoteFiles.passphrasePlaceholder')" />
            </label>
          </div>
          <p v-if="c.connectError.value" class="rft-error">{{ c.connectError.value }}</p>
          <footer class="rft-connect-foot">
            <span class="rft-hint">{{ t("remoteFiles.sshHint") }}</span>
            <div class="rft-connect-actions">
              <button
                v-if="c.form.authentication === 'password'"
                type="button"
                class="rft-link"
                @click="c.forgetSavedPassword()"
              >
                {{ t("remoteFiles.forgetPassword") }}
              </button>
              <button type="button" class="primary-button" :disabled="c.connecting.value" @click="c.connect()">
                <Loader2 v-if="c.connecting.value" :size="13" class="rft-spin" aria-hidden="true" />
                {{ c.connecting.value ? t("remoteFiles.connecting") : t("remoteFiles.connect") }}
              </button>
            </div>
          </footer>
        </div>

        <template v-else>
          <div class="rft-session-bar">
            <span class="rft-session-dot" aria-hidden="true" />
            <span class="rft-session-label mono">{{ remoteTitle }}</span>
            <button type="button" class="rft-link" :disabled="running" @click="c.disconnect()">{{ t("remoteFiles.disconnect") }}</button>
          </div>

          <div v-if="c.pendingLocalPaths.value.length" class="rft-pending">
            <span>{{ t("remoteFiles.pending", { count: c.pendingLocalPaths.value.length }) }}</span>
            <button type="button" class="primary-button rft-mini" :disabled="!c.canUpload.value" @click="upload">
              <Upload :size="12" aria-hidden="true" />{{ t("remoteFiles.uploadPending") }}
            </button>
            <button type="button" class="rft-link" @click="c.clearPendingLocalPaths()">{{ t("remoteFiles.clear") }}</button>
          </div>

          <div class="rft-body">
            <FileTransferPane
              :t="t"
              :title="t('remoteFiles.localPane')"
              :subtitle="c.local.path"
              tone="host"
              :pane="c.local"
              :disabled="running"
              @navigate="(path) => c.navigate(c.local, path)"
              @up="c.goUp(c.local)"
              @enter="(entry) => c.enter(c.local, entry)"
              @toggle="(entry) => toggle(c.local, entry)"
            />
            <div class="rft-arrows">
              <button
                type="button"
                class="rft-arrow"
                :title="t('remoteFiles.upload')"
                :aria-label="t('remoteFiles.upload')"
                :disabled="!c.canUpload.value"
                @click="upload"
              >
                <Upload :size="14" aria-hidden="true" />
                <ArrowRight :size="14" aria-hidden="true" />
                <span v-if="c.local.selected.length">{{ c.local.selected.length }}</span>
              </button>
              <button
                type="button"
                class="rft-arrow"
                :title="t('remoteFiles.download')"
                :aria-label="t('remoteFiles.download')"
                :disabled="!c.canDownload.value"
                @click="download"
              >
                <ArrowLeft :size="14" aria-hidden="true" />
                <Download :size="14" aria-hidden="true" />
                <span v-if="c.remote.selected.length">{{ c.remote.selected.length }}</span>
              </button>
            </div>
            <FileTransferPane
              :t="t"
              :title="t('remoteFiles.remotePane')"
              :subtitle="c.remote.path"
              tone="remote"
              :pane="c.remote"
              :disabled="running"
              @navigate="(path) => c.navigate(c.remote, path)"
              @up="c.goUp(c.remote)"
              @enter="(entry) => c.enter(c.remote, entry)"
              @toggle="(entry) => toggle(c.remote, entry)"
            />
          </div>

          <footer class="rft-foot">
            <div v-if="progress" class="rft-progress" :class="`status-${progress.status}`">
              <span class="rft-progress-label">{{ progressLabel }}</span>
              <span class="rft-bar" role="progressbar" :aria-label="progressLabel" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="progress.percent"><i :style="{ width: `${progress.percent}%` }" /></span>
              <span class="rft-percent mono">{{ progress.percent }}%</span>
              <span class="rft-progress-detail" :title="progressDetail">{{ progressDetail }}</span>
              <button v-if="running" type="button" class="rft-link" @click="c.cancelTransfer()">{{ t("remoteFiles.cancel") }}</button>
            </div>
            <p v-if="progress?.status === 'error' && progress.error" class="rft-error">{{ progress.error }}</p>
            <div v-if="showClipboardAction" class="rft-clipboard">
              <span>{{ t("remoteFiles.downloadedCount", { count: c.lastDownloaded.value.length }) }}</span>
              <button type="button" class="rft-mini" @click="c.writeDownloadedToClipboard()"><Copy :size="12" aria-hidden="true" />{{ t("remoteFiles.copyToClipboard") }}</button>
              <button type="button" class="rft-mini" @click="c.revealDownloaded()"><FolderOpen :size="12" aria-hidden="true" />{{ t("remoteFiles.reveal") }}</button>
            </div>
            <p v-if="!progress && !showClipboardAction" class="rft-hint">{{ t("remoteFiles.footerHint") }}</p>
          </footer>
        </template>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.rft-backdrop {
  position: fixed;
  /* Below the shared dialog layer (z-80) so host-key / certificate prompts
     opened by the SSH companion connection stay above this window. */
  z-index: 70;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: color-mix(in srgb, #000 34%, transparent);
  backdrop-filter: blur(2px);
}
.rft-window {
  display: flex;
  flex-direction: column;
  width: min(960px, 94vw);
  height: min(640px, 86vh);
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: var(--popover);
  color: var(--foreground);
  box-shadow: 0 22px 60px color-mix(in srgb, #000 30%, transparent);
}
.rft-head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 11px 13px 9px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
}
.rft-mark {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  color: var(--primary);
}
.rft-head-text { display: flex; flex: 1; flex-direction: column; min-width: 0; }
.rft-head-text strong { font-size: 12.5px; }
.rft-head-text small { overflow: hidden; font-size: 10.5px; color: var(--muted-foreground); text-overflow: ellipsis; white-space: nowrap; }
.rft-connect { flex: 1; overflow: auto; padding: 13px; }
.rft-intro { margin: 0 0 10px; font-size: 11px; line-height: 1.55; color: var(--muted-foreground); }
.rft-grid { display: grid; grid-template-columns: 1fr 110px; gap: 8px 10px; }
.rft-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted-foreground); }
.rft-field-wide { grid-column: 1 / -1; }
.rft-field input,
.rft-field select {
  height: 28px;
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 0 8px;
  background: var(--background);
  color: var(--foreground);
  font-size: 11.5px;
}
.rft-field input:focus,
.rft-field select:focus { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); outline: none; }
.rft-error { margin: 8px 0 0; font-size: 11px; color: var(--danger, #dc2626); }
.rft-connect-foot { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-top: 12px; }
.rft-connect-actions { display: flex; align-items: center; gap: 10px; }
.rft-hint { margin: 0; font-size: 10.5px; line-height: 1.5; color: var(--muted-foreground); }
.rft-session-bar {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 6px 13px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  font-size: 11px;
}
.rft-session-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--success, #16a34a); }
.rft-session-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted-foreground); }
.rft-link {
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--primary);
  font-size: 11px;
  cursor: pointer;
}
.rft-link:disabled { opacity: .45; cursor: default; }
.rft-pending {
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 8px 13px 0;
  border: 1px solid color-mix(in srgb, var(--primary) 30%, var(--border));
  border-radius: 9px;
  padding: 6px 9px;
  background: color-mix(in srgb, var(--primary) 7%, transparent);
  font-size: 11px;
}
.rft-body {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 46px minmax(0, 1fr);
  gap: 6px;
  flex: 1;
  min-height: 0;
  padding: 9px 13px;
}
.rft-arrows { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; }
.rft-arrow {
  display: inline-flex;
  align-items: center;
  gap: 1px;
  border: 1px solid var(--border);
  border-radius: 9px;
  padding: 6px 4px;
  background: var(--background);
  color: var(--primary);
  cursor: pointer;
}
.rft-arrow span {
  margin-left: 1px;
  font-size: 9px;
  font-weight: 700;
  color: var(--foreground);
}
.rft-arrow:hover:not(:disabled) { border-color: color-mix(in srgb, var(--primary) 60%, var(--border)); }
.rft-arrow:disabled { opacity: .38; cursor: default; }
.rft-foot { padding: 6px 13px 10px; }
.rft-progress { display: flex; align-items: center; gap: 8px; font-size: 10.5px; }
.rft-progress-label { flex: 0 0 auto; font-weight: 600; }
.rft-bar { flex: 1; height: 5px; overflow: hidden; border-radius: 999px; background: color-mix(in srgb, var(--foreground) 13%, transparent); }
.rft-bar i { display: block; height: 100%; border-radius: inherit; background: var(--primary); transition: width .15s linear; }
.rft-percent { flex: 0 0 auto; }
.rft-progress-detail { flex: 0 1 auto; min-width: 0; overflow: hidden; color: var(--muted-foreground); text-overflow: ellipsis; white-space: nowrap; }
.rft-progress.status-error .rft-progress-label { color: var(--danger, #dc2626); }
.rft-progress.status-done .rft-progress-label { color: var(--success, #16a34a); }
.rft-clipboard { display: flex; align-items: center; gap: 8px; margin-top: 6px; font-size: 10.5px; color: var(--muted-foreground); }
.rft-mini {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 4px 7px;
  background: var(--background);
  color: var(--foreground);
  font-size: 10.5px;
  cursor: pointer;
}
.rft-mini:hover:not(:disabled) { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.rft-mini:disabled { opacity: .45; cursor: default; }
.rft-spin { animation: rft-spin 1s linear infinite; }
@keyframes rft-spin { to { transform: rotate(360deg); } }
</style>
