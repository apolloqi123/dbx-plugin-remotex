<script setup lang="ts">
// OTP 侧栏面板（IMPL_PLAN P2-1）：条目列表（TOTP 验证码 + 周期倒计时进度条 /
// HOTP 手动生成）、新增与编辑对话框（密钥遮蔽可切换）、删除确认、扫码导入
// （File API 读图 → otp/import-qr → 预填编辑框）、连接绑定管理（host.listConnections
// 下拉，宿主不支持时回退手输 connectionId）、把当前验证码写入终端输入行
// （不经 App.vue：自带 sequenced-input 队列走 ssh/terminal/in/<sessionId>，不回车）。
// 纯逻辑（协议解析 / 倒计时 / 草稿映射）在 lib/otpPanel.ts；数据与取码在
// useOtpCodes，对话框与绑定区分别下沉到 OtpEditorDialog / OtpBindSection。
import { onMounted, reactive, ref } from "vue";
import { Check, Copy, Link2, Loader2, Pencil, Plus, QrCode, RefreshCw, Send, Trash2 } from "@lucide/vue";
import { bytesToBase64, countdownPercent, entryLabel, formatCountdown, qrResponseToDraft, type OtpEntryView } from "../lib/otpPanel";
import { useOtpCodes } from "../composables/useOtpCodes";
import OtpBindSection from "./OtpBindSection.vue";
import OtpEditorDialog from "./OtpEditorDialog.vue";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

interface HostConnection {
  id: string;
  name: string;
}

const {
  loading,
  entries,
  bindings,
  codes,
  copiedId,
  sentId,
  actionError,
  generating,
  refresh,
  generateFor,
  generateAllTotp,
  copyCode,
  sendCode,
  showActionError,
  removeEntry,
} = useOtpCodes({ t: props.t });

const connections = ref<HostConnection[]>([]);
const connectionsUnavailable = ref(false);
const bindOpen = reactive<Record<string, boolean>>({});
const bindPick = reactive<Record<string, string>>({});
const bindManual = reactive<Record<string, string>>({});

const qrInput = ref<HTMLInputElement | null>(null);
const qrBusy = ref(false);
const editorRef = ref<InstanceType<typeof OtpEditorDialog> | null>(null);

// 验证码遮蔽用的占位（无码时展示，避免布局抖动）。
const CODE_PLACEHOLDER = "······";

async function loadConnections() {
  try {
    const payload = await window.dbxPlugin.request<{ connections?: Array<{ id?: unknown; name?: unknown }> }>("host.listConnections");
    connections.value = (payload?.connections ?? [])
      .map((connection) => ({ id: typeof connection.id === "string" ? connection.id : "", name: typeof connection.name === "string" ? connection.name : "" }))
      .filter((connection) => connection.id);
  } catch {
    // 老宿主没有 listConnections 扩展点：绑定降级为手输 connectionId。
    connectionsUnavailable.value = true;
  }
}

function onBindingsUpdate(next: Record<string, string>) {
  bindings.value = next;
}

function openCreate() {
  editorRef.value?.openCreate();
}

function openEdit(entry: OtpEntryView) {
  editorRef.value?.openEdit(entry);
}

function askDelete(entry: OtpEntryView) {
  editorRef.value?.askDelete(entry);
}

function onSaved(saved: OtpEntryView) {
  const existing = entries.value.findIndex((entry) => entry.id === saved.id);
  if (existing >= 0) entries.value.splice(existing, 1, saved);
  else entries.value.push(saved);
  if (saved.otpType === "totp") void generateFor(saved);
}

async function importQrImage(file: File) {
  qrBusy.value = true;
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const payload = await window.dbxPlugin.invoke("otp/import-qr", { imageBase64: bytesToBase64(bytes) });
    const draft = qrResponseToDraft(payload);
    if (!draft) {
      showActionError(new Error("otp/import-qr returned no secret"));
      return;
    }
    editorRef.value?.applyQrDraft(draft);
  } catch (cause) {
    showActionError(cause);
  } finally {
    qrBusy.value = false;
    if (qrInput.value) qrInput.value.value = "";
  }
}

function onQrFileChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (file) void importQrImage(file);
}

onMounted(() => {
  void refresh().then(() => generateAllTotp());
  void loadConnections();
});
</script>

<template>
  <div class="otp-panel">
    <div class="otp-toolbar">
      <button type="button" class="otp-add" @click="openCreate"><Plus />{{ t("otpPanel.add") }}</button>
      <button type="button" class="icon-button" :title="t('otpPanel.scanQr')" :aria-label="t('otpPanel.scanQr')" :disabled="qrBusy" @click="qrInput?.click()">
        <Loader2 v-if="qrBusy" class="spinning" /><QrCode v-else />
      </button>
      <button type="button" class="icon-button" :title="t('otpPanel.refresh')" :aria-label="t('otpPanel.refresh')" :disabled="loading" @click="() => { void refresh().then(() => generateAllTotp()); }">
        <RefreshCw />
      </button>
      <input ref="qrInput" type="file" accept="image/*" class="otp-file-input" @change="onQrFileChange" />
    </div>

    <p v-if="loading" class="otp-empty"><Loader2 class="spinning" />{{ t("otpPanel.loading") }}</p>
    <p v-else-if="!entries.length" class="otp-empty">{{ t("otpPanel.empty") }}</p>
    <ul v-else class="otp-list">
      <li v-for="entry in entries" :key="entry.id" class="otp-entry">
        <div class="otp-entry-head">
          <span class="otp-issuer" :title="entry.issuer">{{ entryLabel(entry) }}</span>
          <span class="otp-badge" :class="`otp-badge--${entry.otpType}`">{{ entry.otpType === "hotp" ? t("otpPanel.hotp") : t("otpPanel.totp") }}</span>
        </div>
        <div v-if="entry.username" class="otp-username">{{ entry.username }}</div>

        <template v-if="entry.otpType === 'totp'">
          <div class="otp-code-row">
            <span class="otp-code" :class="{ 'is-used': codes[entry.id]?.reused }">{{ codes[entry.id]?.code || CODE_PLACEHOLDER }}</span>
            <span class="otp-count" :title="t('otpPanel.countdown')">{{ formatCountdown(codes[entry.id]?.remaining ?? 0) }}</span>
          </div>
          <div class="otp-progress" role="presentation">
            <span class="otp-progress-fill" :style="{ width: `${countdownPercent(codes[entry.id]?.remaining ?? 0, entry.period)}%` }" />
          </div>
          <p v-if="codes[entry.id]?.reused" class="otp-reused">{{ t("otpPanel.reused", { seconds: codes[entry.id]?.remaining ?? 0 }) }}</p>
        </template>
        <template v-else>
          <div class="otp-code-row">
            <span class="otp-code">{{ codes[entry.id]?.code || CODE_PLACEHOLDER }}</span>
            <button type="button" class="otp-generate" :disabled="generating.has(entry.id)" @click="() => void generateFor(entry)">
              <Loader2 v-if="generating.has(entry.id)" class="spinning" />{{ t("otpPanel.generate") }}
            </button>
          </div>
        </template>

        <div class="otp-actions">
          <button type="button" class="icon-button" :title="copiedId === entry.id ? t('otpPanel.copied') : t('otpPanel.copy')" :aria-label="copiedId === entry.id ? t('otpPanel.copied') : t('otpPanel.copy')" :disabled="!codes[entry.id]?.code" @click="() => void copyCode(entry)">
            <Check v-if="copiedId === entry.id" /><Copy v-else />
          </button>
          <button type="button" class="icon-button" :title="sentId === entry.id ? t('otpPanel.sent') : t('otpPanel.send')" :aria-label="sentId === entry.id ? t('otpPanel.sent') : t('otpPanel.send')" :disabled="!codes[entry.id]?.code" @click="() => void sendCode(entry)">
            <Send />
          </button>
          <button type="button" class="icon-button" :title="t('otpPanel.bind')" :aria-label="t('otpPanel.bind')" @click="bindOpen[entry.id] = !bindOpen[entry.id]">
            <Link2 />
          </button>
          <span class="otp-actions-spacer" />
          <button type="button" class="icon-button" :title="t('otpPanel.edit')" :aria-label="t('otpPanel.edit')" @click="openEdit(entry)"><Pencil /></button>
          <button type="button" class="icon-button" :title="t('otpPanel.delete')" :aria-label="t('otpPanel.delete')" @click="askDelete(entry)"><Trash2 /></button>
        </div>

        <OtpBindSection
          v-if="bindOpen[entry.id]"
          :entry="entry"
          :bindings="bindings"
          :connections="connections"
          :pick="bindPick[entry.id] || ''"
          :manual="bindManual[entry.id] || ''"
          :t="t"
          @update:bindings="onBindingsUpdate"
          @update:pick="(value) => (bindPick[entry.id] = value)"
          @update:manual="(value) => (bindManual[entry.id] = value)"
          @error="showActionError"
        />
      </li>
    </ul>

    <p v-if="actionError" class="otp-error">{{ actionError }}</p>

    <OtpEditorDialog
      ref="editorRef"
      :t="t"
      @saved="onSaved"
      @deleted="removeEntry"
      @error="showActionError"
    />
  </div>
</template>

<style scoped>
.otp-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
  height: 100%;
  overflow-y: auto;
}
.otp-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
}
.otp-add {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 3px 8px;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
  cursor: pointer;
}
.otp-add:hover { background: var(--accent); }
.otp-add svg, .icon-button svg { width: 14px; height: 14px; }
.otp-file-input { display: none; }
.otp-empty {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  padding: 8px 4px;
  color: var(--muted-foreground);
  font-size: 12px;
}
.otp-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.otp-entry {
  display: flex;
  flex-direction: column;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 8px;
}
.otp-entry-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.otp-issuer {
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 600;
}
.otp-badge {
  flex: 0 0 auto;
  border-radius: 999px;
  padding: 1px 6px;
  font-size: 10px;
  background: var(--accent);
  color: var(--muted-foreground);
}
.otp-badge--hotp { color: var(--primary); }
.otp-username {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted-foreground);
  font-size: 11px;
}
.otp-code-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.otp-code {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 18px;
  letter-spacing: 2px;
  color: var(--foreground);
}
.otp-code.is-used { color: var(--muted-foreground); }
.otp-count {
  margin-left: auto;
  font-size: 11px;
  color: var(--muted-foreground);
  font-variant-numeric: tabular-nums;
}
.otp-progress {
  height: 3px;
  border-radius: 2px;
  background: var(--accent);
  overflow: hidden;
}
.otp-progress-fill {
  display: block;
  height: 100%;
  background: var(--primary);
  transition: width 1s linear;
}
.otp-reused {
  margin: 0;
  font-size: 11px;
  color: var(--muted-foreground);
}
.otp-generate {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 2px 8px;
  background: transparent;
  color: var(--foreground);
  font-size: 11px;
  cursor: pointer;
}
.otp-generate:hover:not(:disabled) { background: var(--accent); }
.otp-generate svg { width: 12px; height: 12px; }
.otp-actions {
  display: flex;
  align-items: center;
  gap: 2px;
}
.otp-actions-spacer { flex: 1 1 auto; }
.otp-actions .icon-button {
  width: 24px;
  height: 24px;
}
.otp-error {
  margin: 0;
  border: 1px solid color-mix(in srgb, var(--destructive) 60%, var(--border));
  border-radius: var(--radius);
  padding: 4px 8px;
  background: color-mix(in srgb, var(--destructive) 16%, var(--popover));
  color: color-mix(in srgb, var(--destructive) 45%, var(--foreground));
  font-size: 11px;
  word-break: break-word;
}
</style>
