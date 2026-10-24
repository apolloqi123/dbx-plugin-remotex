<script setup lang="ts">
// OTP 编辑与删除对话框：新增/编辑草稿（密钥遮蔽可切换，扫码导入预填）、
// 保存与后端校验错误回显、删除确认。状态由本组件自持，父级通过 ref 打开，
// 保存/删除成功后再用事件把结果交回父级更新列表。
import { computed, reactive, ref } from "vue";
import { Eye, EyeOff, Loader2, Trash2, X } from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import {
  emptyOtpDraft,
  entryLabel,
  entryToDraft,
  otpDraftError,
  otpSaveParams,
  parseOtpEntries,
  type OtpDraft,
  type OtpEntryView,
} from "../lib/otpPanel";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (event: "saved", entry: OtpEntryView): void;
  (event: "deleted", id: string): void;
  (event: "error", cause: unknown): void;
}>();

const editor = reactive<{ open: boolean; draft: OtpDraft; showSecret: boolean; touched: boolean }>({
  open: false,
  draft: emptyOtpDraft(),
  showSecret: false,
  touched: false,
});
/** 后端校验错误的原始文案（对话框错误行展示，优先于本地校验文案）。 */
const draftBackendError = ref("");
const saving = ref(false);
const deleteTarget = ref<OtpEntryView | null>(null);

function openCreate() {
  editor.draft = emptyOtpDraft();
  editor.showSecret = false;
  editor.touched = false;
  // 上一次保存失败的文案不能跨对话框残留到新表单。
  draftBackendError.value = "";
  editor.open = true;
}

function openEdit(entry: OtpEntryView) {
  editor.draft = entryToDraft(entry);
  editor.showSecret = false;
  editor.touched = false;
  draftBackendError.value = "";
  editor.open = true;
}

/** 扫码导入：直接以解析好的草稿打开编辑框并展开密钥。 */
function applyQrDraft(draft: OtpDraft) {
  editor.draft = draft;
  editor.showSecret = true;
  editor.touched = false;
  editor.open = true;
}

const draftError = computed(() => {
  if (!editor.touched) return "";
  switch (otpDraftError(editor.draft)) {
    case "issuer":
      return props.t("otpPanel.error.issuer");
    case "secret":
      return props.t("otpPanel.error.secret");
    case "counter":
      return props.t("otpPanel.error.counter");
    case "counterNumber":
      return props.t("otpPanel.error.counterNumber");
    default:
      return "";
  }
});

async function saveDraft() {
  editor.touched = true;
  if (otpDraftError(editor.draft)) return;
  saving.value = true;
  try {
    const payload = await window.dbxPlugin.invoke<{ entry: unknown }>("otp/save", otpSaveParams(editor.draft));
    const saved = parseOtpEntries({ entries: [payload?.entry] })[0];
    if (saved) emit("saved", saved);
    editor.open = false;
  } catch (cause) {
    // 后端校验错误直接落在对话框错误行（原始文案，字段可读）。
    draftBackendError.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    saving.value = false;
  }
}

function askDelete(entry: OtpEntryView) {
  deleteTarget.value = entry;
}

async function confirmDelete() {
  const target = deleteTarget.value;
  if (!target) return;
  try {
    const payload = await window.dbxPlugin.invoke<{ deleted: boolean }>("otp/delete", { id: target.id });
    if (payload?.deleted) emit("deleted", target.id);
    deleteTarget.value = null;
  } catch (cause) {
    deleteTarget.value = null;
    emit("error", cause);
  }
}

defineExpose({ openCreate, openEdit, applyQrDraft, askDelete });
</script>

<template>
  <Dialog :open="editor.open" @update:open="(open) => (editor.open = open)">
    <DialogContent class="modal otp-editor-modal" @escape-key-down.prevent>
      <header>
        <DialogTitle>{{ editor.draft.id ? t("otpPanel.editor.edit") : t("otpPanel.editor.add") }}</DialogTitle>
        <button type="button" class="icon-button" :title="t('otpPanel.editor.cancel')" :aria-label="t('otpPanel.editor.cancel')" @click="editor.open = false"><X /></button>
      </header>
      <form class="otp-editor" @submit.prevent="() => void saveDraft()">
        <div class="otp-editor-row">
          <label><input v-model="editor.draft.otpType" type="radio" value="totp" />{{ t("otpPanel.totp") }}</label>
          <label><input v-model="editor.draft.otpType" type="radio" value="hotp" />{{ t("otpPanel.hotp") }}</label>
        </div>
        <label class="otp-editor-field">
          <span>{{ t("otpPanel.editor.issuer") }}</span>
          <input v-model="editor.draft.issuer" spellcheck="false" />
        </label>
        <label class="otp-editor-field">
          <span>{{ t("otpPanel.editor.username") }}</span>
          <input v-model="editor.draft.username" spellcheck="false" />
        </label>
        <label class="otp-editor-field">
          <span class="otp-editor-secret-label">
            {{ t("otpPanel.editor.secret") }}
            <button type="button" class="icon-button" :title="editor.showSecret ? t('otpPanel.editor.hide') : t('otpPanel.editor.show')" :aria-label="editor.showSecret ? t('otpPanel.editor.hide') : t('otpPanel.editor.show')" @click="editor.showSecret = !editor.showSecret">
              <EyeOff v-if="editor.showSecret" /><Eye v-else />
            </button>
          </span>
          <textarea v-if="editor.showSecret" v-model="editor.draft.secret" rows="2" class="mono" spellcheck="false" :placeholder="editor.draft.id ? t('otpPanel.editor.secretHint') : ''" />
          <input v-else v-model="editor.draft.secret" type="password" autocomplete="off" spellcheck="false" :placeholder="editor.draft.id ? t('otpPanel.editor.secretHint') : ''" />
        </label>
        <div class="otp-editor-grid">
          <label class="otp-editor-field">
            <span>{{ t("otpPanel.editor.algorithm") }}</span>
            <Select :model-value="editor.draft.algorithm" @update:model-value="(value) => { editor.draft.algorithm = typeof value === 'string' ? value : 'SHA1'; }">
              <SelectTrigger size="sm"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="algorithm in ['SHA1', 'SHA256', 'SHA512']" :key="algorithm" :value="algorithm">{{ algorithm }}</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label class="otp-editor-field">
            <span>{{ t("otpPanel.editor.digits") }}</span>
            <Select :model-value="String(editor.draft.digits)" @update:model-value="(value) => { editor.draft.digits = Number(value) || 6; }">
              <SelectTrigger size="sm"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="digits in [6, 7, 8]" :key="digits" :value="String(digits)">{{ digits }}</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label v-if="editor.draft.otpType === 'totp'" class="otp-editor-field">
            <span>{{ t("otpPanel.editor.period") }}</span>
            <input v-model.number="editor.draft.period" type="number" min="1" max="3600" />
          </label>
          <label v-if="editor.draft.otpType === 'hotp'" class="otp-editor-field">
            <span>{{ t("otpPanel.editor.counter") }}</span>
            <input v-model="editor.draft.counter" type="number" min="0" />
          </label>
        </div>
        <p v-if="draftError || draftBackendError" class="otp-error">{{ draftBackendError || draftError }}</p>
        <footer>
          <button type="button" class="otp-cancel" @click="editor.open = false">{{ t("otpPanel.editor.cancel") }}</button>
          <button type="submit" class="primary-button" :disabled="saving">
            <Loader2 v-if="saving" class="spinning" />{{ t("otpPanel.editor.save") }}
          </button>
        </footer>
      </form>
    </DialogContent>
  </Dialog>

  <Dialog :open="deleteTarget !== null" @update:open="(open) => { if (!open) deleteTarget = null; }">
    <DialogContent class="modal otp-delete-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("otpPanel.deleteConfirm.title") }}</DialogTitle></header>
      <p class="otp-delete-message">{{ t("otpPanel.deleteConfirm.message", { label: deleteTarget ? entryLabel(deleteTarget) : "" }) }}</p>
      <footer>
        <button type="button" class="otp-cancel" @click="deleteTarget = null">{{ t("otpPanel.deleteConfirm.cancel") }}</button>
        <button type="button" class="danger-button" @click="() => void confirmDelete()"><Trash2 />{{ t("otpPanel.deleteConfirm.confirm") }}</button>
      </footer>
    </DialogContent>
  </Dialog>
</template>

<style scoped>
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
.otp-editor-modal { width: min(420px, calc(100vw - 40px)); }
.otp-delete-modal { width: min(360px, calc(100vw - 40px)); }
.otp-editor {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.otp-editor-row {
  display: flex;
  gap: 14px;
  font-size: 12px;
}
.otp-editor-row label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.otp-editor-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
}
.otp-editor-field input, .otp-editor-field textarea {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 4px 8px;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
}
.otp-editor-secret-label {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
}
.otp-editor-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
.otp-editor footer, .otp-delete-modal footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.otp-cancel {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 4px 10px;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
  cursor: pointer;
}
.otp-cancel:hover { background: var(--accent); }
.otp-delete-message {
  margin: 0;
  font-size: 12px;
  color: var(--muted-foreground);
}
.danger-button {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid color-mix(in srgb, var(--destructive) 60%, var(--border));
  border-radius: var(--radius);
  padding: 4px 10px;
  background: color-mix(in srgb, var(--destructive) 16%, var(--popover));
  color: color-mix(in srgb, var(--destructive) 55%, var(--foreground));
  font-size: 12px;
  cursor: pointer;
}
.danger-button svg { width: 13px; height: 13px; }
</style>
