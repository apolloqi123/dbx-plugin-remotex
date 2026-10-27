<script setup lang="ts">
import {
  FileText,
  Loader2,
  Pencil,
  RefreshCw,
  Save,
  Trash2,
  X,
} from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../shared/ui/dialog/index";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../shared/ui/select/index";
import OtpPanel from "../../features/security/components/OtpPanel.vue";
import ImportWizard from "../../features/connections/components/ImportWizard.vue";
import { createPreviewAsyncComponent } from "../../shared/lib/asyncPreview";
import { formatBytes } from "../../shared/lib/format";
import { normalizeDropTargetDir } from "../../features/terminal/lib/behavior/terminalInteraction";
import { MAX_INLINE_PREVIEW_BYTES } from "../../features/sftp/composables/editor/useEditorTabs";
import type { JsonPreviewState } from "../../features/sftp/lib/editor/jsonPreview";
import {
  auditKindLabel,
  auditKindOptions,
  auditOutcomeLabel,
  type AuditEntry,
} from "../../features/security/lib/auditLog";

/** 工作台内联弹窗宿主（T21 自 App.vue 模板迁入）：SFTP 工具栏的 OTP/导入、
 *  文件预览（含编辑态与 JSON 面板）、录制删除/一键清空、传输历史清空、审计
 *  日志查看与清空、终端拖入上传落点询问。已抽出的 WorkbenchSftpDialogs /
 *  WorkbenchSessionDialogs 保持原样，不并入本组件。
 *
 *  弹窗壳仍走 reka Dialog（portal/遮罩/焦点陷阱/外点关闭由 reka 承担，Esc 由
 *  App 的 onDocumentKeydown 分层链独占，内容上一律 @escape-key-down.prevent）。
 *  可写开关（各弹层 open、审计筛选、拖放落点/路径）走 v-model 链，动作经显式
 *  emit 上抛；组件内不 inject、不读全局。 */
type DropUploadPrompt = { files: Array<{ name: string }> };
type RecordingDeleteTarget = {
  recordingId: string;
  host?: string;
  startedAt?: number;
  durationSecs?: number;
};

// 响应式 props 解构（Vue 3.5）：模板/闭包里沿用迁移前的绑定名，同时保持 prop
// 更新时的实时求值（编译器改写为 __props.xxx 访问）。
const {
  t,
  locale,
  appearance,
  previewTitle,
  previewText,
  previewLoading,
  previewSize,
  previewEditable,
  previewEditableAllowed,
  previewSaving,
  previewMode,
  previewImageUrl,
  previewTruncated,
  previewDirty,
  jsonPreviewState,
  recordingDeleteSubmitting,
  recordingsCount,
  recordingClearAllSubmitting,
  auditEntries,
  auditLoading,
  auditLoadFailed,
  auditTruncated,
  visibleAuditEntries,
  auditClearSubmitting,
  auditTime,
  auditRowKindClass,
  dropUploadPrompt,
  dropCwdTarget,
  setDropUploadPathInputEl,
  formatRecordedAt,
  formatDuration,
} = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  appearance: DbxPluginAppearance;
  // —— 文件预览 ——
  previewTitle: string;
  previewText: string;
  previewLoading: boolean;
  previewSize: number;
  previewEditable: boolean;
  previewEditableAllowed: boolean;
  previewSaving: boolean;
  previewMode: "text" | "image";
  previewImageUrl: string;
  previewTruncated: boolean;
  previewDirty: boolean;
  jsonPreviewState: JsonPreviewState;
  // —— 录制删除/清空 ——
  recordingDeleteSubmitting: boolean;
  recordingsCount: number;
  recordingClearAllSubmitting: boolean;
  // —— 审计日志 ——
  auditEntries: AuditEntry[];
  auditLoading: boolean;
  auditLoadFailed: boolean;
  auditTruncated: boolean;
  visibleAuditEntries: AuditEntry[];
  auditClearSubmitting: boolean;
  auditTime: (ts: number) => string;
  auditRowKindClass: (kind: string) => string;
  // —— 终端拖入上传落点 ——
  dropUploadPrompt: DropUploadPrompt | undefined;
  dropCwdTarget: string;
  /** 路径输入框以回调回填 App 的 composable ref，保持打开时聚焦的既有语义。 */
  setDropUploadPathInputEl?: (el: HTMLInputElement | null) => void;
  // —— 注入的时间/格式化闭包 ——
  formatRecordedAt: (startedAt?: number) => string;
  formatDuration: (secs: number) => string;
}>();

const emit = defineEmits<{
  (e: "close-preview"): void;
  (e: "begin-preview-edit"): void;
  (e: "cancel-preview-edit"): void;
  (e: "save-preview"): void;
  (e: "confirm-recording-delete"): void;
  (e: "confirm-recording-clear"): void;
  (e: "confirm-transfer-history-clear"): void;
  (e: "load-audit-entries"): void;
  (e: "confirm-audit-clear"): void;
  (e: "confirm-drop-upload"): void;
  (e: "cancel-drop-upload"): void;
}>();

const otpPanelOpen = defineModel<boolean>("otpPanelOpen", { required: true });
const importPanelOpen = defineModel<boolean>("importPanelOpen", { required: true });
const previewOpen = defineModel<boolean>("previewOpen", { required: true });
const previewImageZoomed = defineModel<boolean>("previewImageZoomed", { required: true });
const previewDraft = defineModel<string>("previewDraft", { required: true });
const recordingDeleteTarget = defineModel<RecordingDeleteTarget | null>("recordingDeleteTarget", { required: true });
const recordingClearAllOpen = defineModel<boolean>("recordingClearAllOpen", { required: true });
const transferHistoryClearOpen = defineModel<boolean>("transferHistoryClearOpen", { required: true });
const auditOpen = defineModel<boolean>("auditOpen", { required: true });
const auditClearOpen = defineModel<boolean>("auditClearOpen", { required: true });
const auditKindFilter = defineModel<string>("auditKindFilter", { required: true });
const dropUploadTarget = defineModel<"cwd" | "custom">("dropUploadTarget", { required: true });
const dropUploadPathInput = defineModel<string>("dropUploadPathInput", { required: true });

const SELECT_EMPTY_SENTINEL = "__empty__";

function bindDropUploadPathInputEl(el: Element | { $el?: unknown } | null) {
  setDropUploadPathInputEl?.(el instanceof HTMLInputElement ? el : null);
}

// 预览面板按需加载（含 CodeMirror + language-data，数百 KB）：模块只在首次打开
// 预览弹窗时求值，工作台启动不再解析编辑器/语言包。loading/error 兜底：动态
// import 失败时渲染说明 + 重试，不再留空白正文（文案闭包按当前 locale 现取）。
const previewAsyncLabels = () => ({
  loading: t("loading"),
  failed: t("previewDialog.loadFailed"),
  retry: t("previewDialog.retry"),
});
const TextPreview = createPreviewAsyncComponent(
  () => import("../../features/sftp/components/TextPreview.vue"),
  previewAsyncLabels,
);
const JsonPreviewPanel = createPreviewAsyncComponent(
  () => import("../../features/sftp/components/JsonPreviewPanel.vue"),
  previewAsyncLabels,
);
</script>

<template>
  <!-- 工具弹窗：OTP 验证码 / 会话导入（导入入口在顶部「会话」菜单，组件自带内部弹窗与状态）。 -->
  <Dialog :open="otpPanelOpen" @update:open="(open) => { if (!open) otpPanelOpen = false; }">
    <DialogContent class="modal sftp-tool-modal" @escape-key-down.prevent>
      <header>
        <DialogTitle>{{ t("otpPanel.title") }}</DialogTitle>
        <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="otpPanelOpen = false"><X /></button>
      </header>
      <OtpPanel :t="t" />
    </DialogContent>
  </Dialog>

  <Dialog :open="importPanelOpen" @update:open="(open) => { if (!open) importPanelOpen = false; }">
    <DialogContent class="modal sftp-tool-modal" @escape-key-down.prevent>
      <header>
        <DialogTitle>{{ t("importWizard.title") }}</DialogTitle>
        <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="importPanelOpen = false"><X /></button>
      </header>
      <ImportWizard :t="t" />
    </DialogContent>
  </Dialog>

  <!-- 弹层统一迁移 reka Dialog（Strategy B）：portal/遮罩/焦点陷阱/外点关闭由 reka
       承担；Esc 仍由 onDocumentKeydown 分层链独占（内容上一律 @escape-key-down.prevent），
       外点关闭经 update:open(false) 路由到各弹窗的语义取消函数。 -->
  <Dialog :open="previewOpen" @update:open="(open) => { if (!open) emit('close-preview'); }">
    <DialogContent class="modal preview-modal" @escape-key-down.prevent>
      <header>
        <DialogTitle>
          {{ previewTitle }}
          <span v-if="previewDirty" class="preview-dirty"><span class="preview-dirty-dot" />{{ t("editSave.unsaved") }}</span>
          <span v-else-if="previewTruncated" class="preview-truncated-badge">{{ t("previewDialog.truncated", { limit: formatBytes(MAX_INLINE_PREVIEW_BYTES), size: formatBytes(previewSize) }) }}</span>
        </DialogTitle>
        <div v-if="previewEditableAllowed" class="preview-actions">
          <template v-if="!previewEditable">
            <button :title="t('editSave.edit')" :aria-label="t('editSave.edit')" @click="emit('begin-preview-edit')"><Pencil />{{ t("editSave.edit") }}</button>
          </template>
          <template v-else>
            <button :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel-preview-edit')">{{ t("cancel") }}</button>
            <button :title="t('editSave.save')" :aria-label="t('editSave.save')" :disabled="previewSaving" @click="emit('save-preview')"><Loader2 v-if="previewSaving" class="spinning" /><Save v-else />{{ t("editSave.save") }}</button>
          </template>
        </div>
        <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('close-preview')"><X /></button>
      </header>
      <div v-if="previewLoading" class="empty"><Loader2 class="spinning" />{{ t("loading") }}</div>
      <div v-else-if="previewMode === 'image'" class="preview-image-stage">
        <img class="preview-image" :class="{ 'preview-image--full': previewImageZoomed }" :src="previewImageUrl" :alt="previewTitle" :title="previewImageZoomed ? t('imagePreview.zoomOut') : t('imagePreview.zoomIn')" @click="previewImageZoomed = !previewImageZoomed" />
      </div>
      <!-- JSON 候选且非编辑态走格式化预览面板（invalid/too-large 时面板内部降级为
           原始视图 + 提示条）；其余文件与编辑态保持既有 TextPreview。 -->
      <JsonPreviewPanel v-else-if="!previewEditable && jsonPreviewState.kind !== 'unavailable'" :state="jsonPreviewState" :text="previewText" :file-name="previewTitle" :locale="locale" :appearance="appearance" />
      <TextPreview v-else :text="previewText" :file-name="previewTitle" :appearance="appearance" :editable="previewEditable" :t="t" @change="previewDraft = $event" />
    </DialogContent>
  </Dialog>

  <Dialog :open="!!recordingDeleteTarget" @update:open="(open) => { if (!open) recordingDeleteTarget = null; }">
    <DialogContent class="modal small-modal" @escape-key-down.prevent>
      <template v-if="recordingDeleteTarget">
      <header><DialogTitle>{{ t("recordingDelete") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="recordingDeleteTarget = null"><X /></button></header>
      <div class="destructive-copy"><div><strong>{{ t("recordingDeleteConfirm", { host: recordingDeleteTarget.host || recordingDeleteTarget.recordingId }) }}</strong><p class="muted">{{ formatRecordedAt(recordingDeleteTarget.startedAt) }} · {{ formatDuration(recordingDeleteTarget.durationSecs ?? 0) }}</p></div></div>
      <footer><button @click="recordingDeleteTarget = null" :disabled="recordingDeleteSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="recordingDeleteSubmitting" @click="emit('confirm-recording-delete')"><Loader2 v-if="recordingDeleteSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
      </template>
    </DialogContent>
  </Dialog>

  <!-- 录制一键清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
  <Dialog :open="recordingClearAllOpen" @update:open="(open) => { if (!open) recordingClearAllOpen = false; }">
    <DialogContent class="modal small-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("recordingsClear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="recordingClearAllOpen = false"><X /></button></header>
      <div class="destructive-copy"><div><strong>{{ t("recordingsClearConfirm", { count: recordingsCount }) }}</strong></div></div>
      <footer><button @click="recordingClearAllOpen = false" :disabled="recordingClearAllSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="recordingClearAllSubmitting" @click="emit('confirm-recording-clear')"><Loader2 v-if="recordingClearAllSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
    </DialogContent>
  </Dialog>

  <!-- 传输历史清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
  <Dialog :open="transferHistoryClearOpen" @update:open="(open) => { if (!open) transferHistoryClearOpen = false; }">
    <DialogContent class="modal small-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("transfersHistory.clear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="transferHistoryClearOpen = false"><X /></button></header>
      <div class="destructive-copy"><div><strong>{{ t("transfersHistory.clearConfirm") }}</strong></div></div>
      <footer><button @click="transferHistoryClearOpen = false">{{ t("cancel") }}</button><button class="danger-button" @click="emit('confirm-transfer-history-clear')"><Trash2 />{{ t("delete") }}</button></footer>
    </DialogContent>
  </Dialog>

  <Dialog :open="auditOpen" @update:open="(open) => { if (!open) auditOpen = false; }">
    <DialogContent class="modal audit-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("auditLog.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="auditOpen = false"><X /></button></header>
      <div class="settings-body">
        <div class="audit-toolbar">
          <Select :model-value="auditKindFilter || SELECT_EMPTY_SENTINEL" @update:model-value="(v) => (auditKindFilter = v === SELECT_EMPTY_SENTINEL ? '' : String(v))">
            <SelectTrigger size="xs" class="audit-kind-select" :aria-label="t('auditLog.kindFilter')">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem :value="SELECT_EMPTY_SENTINEL">{{ t("auditLog.kindAll") }}</SelectItem>
              <SelectItem v-for="kind in auditKindOptions(auditEntries)" :key="kind" :value="kind">{{ auditKindLabel(kind, t) }}</SelectItem>
            </SelectContent>
          </Select>
          <div class="audit-actions">
            <button class="icon-button" :title="t('refresh')" :aria-label="t('refresh')" :disabled="auditLoading" @click="emit('load-audit-entries')"><RefreshCw :class="{ spinning: auditLoading }" /></button>
            <button class="icon-button" :title="t('auditLog.clear')" :aria-label="t('auditLog.clear')" :disabled="!auditEntries.length" @click="auditClearOpen = true"><Trash2 /></button>
          </div>
        </div>
        <div v-if="auditLoading && !auditEntries.length" class="empty compact audit-empty"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="auditLoadFailed" class="empty compact audit-empty">
          <span>{{ t("auditLog.loadFailed") }}</span>
          <button class="link-button" @click="emit('load-audit-entries')">{{ t("refresh") }}</button>
        </div>
        <div v-else-if="!visibleAuditEntries.length" class="audit-empty"><FileText /><span>{{ t("auditLog.empty") }}</span></div>
        <template v-else>
          <ul class="audit-list">
            <li v-for="(entry, index) in visibleAuditEntries" :key="`${entry.ts}-${entry.kind}-${index}`" class="audit-row">
              <span class="audit-time mono">{{ auditTime(entry.ts) }}</span>
              <span class="audit-kind-badge" :class="auditRowKindClass(entry.kind)">{{ auditKindLabel(entry.kind, t) }}</span>
              <span v-if="entry.connection || entry.sessionId" class="audit-connection mono" :title="entry.connection || entry.sessionId">{{ entry.connection || entry.sessionId }}</span>
              <span v-if="entry.command" class="audit-command mono" :title="entry.command">{{ entry.command }}</span>
              <span v-if="entry.output" class="audit-output mono" :title="entry.output">{{ entry.output }}</span>
              <span v-if="entry.gate" class="audit-gate mono">{{ entry.gate }}</span>
              <span v-if="auditOutcomeLabel(entry, t)" class="audit-outcome" :class="{ error: entry.outcome === 'error' || entry.decision === 'denied' || entry.decision === 'timeout', ok: entry.outcome === 'ok' || entry.decision === 'approved' }">{{ auditOutcomeLabel(entry, t) }}</span>
              <span v-if="entry.exitCode != null" class="audit-time">{{ t("auditLog.exitCode", { code: entry.exitCode }) }}</span>
            </li>
          </ul>
          <p v-if="auditTruncated" class="muted audit-truncated">{{ t("auditLog.truncated") }}</p>
        </template>
      </div>
      <footer><button @click="auditOpen = false">{{ t("close") }}</button></footer>
    </DialogContent>
  </Dialog>

  <!-- 审计日志清空确认：应用内弹窗（沙箱 iframe confirm 恒 false） -->
  <Dialog :open="auditClearOpen" @update:open="(open) => { if (!open) auditClearOpen = false; }">
    <DialogContent class="modal small-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("auditLog.clear") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="auditClearOpen = false"><X /></button></header>
      <div class="destructive-copy"><div><strong>{{ t("auditLog.clearConfirm") }}</strong></div></div>
      <footer><button @click="auditClearOpen = false" :disabled="auditClearSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="auditClearSubmitting" @click="emit('confirm-audit-clear')"><Loader2 v-if="auditClearSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
    </DialogContent>
  </Dialog>

  <Dialog :open="!!dropUploadPrompt" @update:open="(open) => { if (!open) emit('cancel-drop-upload'); }">
    <DialogContent class="modal small-modal" @escape-key-down.prevent>
      <template v-if="dropUploadPrompt">
      <header>
        <DialogTitle>{{ t("terminalDropPrompt.title") }}</DialogTitle>
        <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('cancel-drop-upload')"><X /></button>
      </header>
      <p class="muted">{{ t("terminalDropPrompt.summary", { count: dropUploadPrompt.files.length }) }}</p>
      <pre class="command-output mono drop-file-list">{{ dropUploadPrompt.files.map((file) => file.name).join("\n") }}</pre>
      <label class="drop-option">
        <input v-model="dropUploadTarget" type="radio" name="drop-upload-target" value="cwd" />
        <span>{{ t("terminalDropPrompt.toCurrent") }}</span>
        <code class="mono">{{ dropCwdTarget }}</code>
      </label>
      <label class="drop-option">
        <input v-model="dropUploadTarget" type="radio" name="drop-upload-target" value="custom" />
        <span>{{ t("terminalDropPrompt.toCustom") }}</span>
      </label>
      <input
        :ref="bindDropUploadPathInputEl"
        v-model="dropUploadPathInput"
        class="drop-path-input mono"
        type="text"
        spellcheck="false"
        :placeholder="t('terminalDropPrompt.pathPlaceholder')"
        :disabled="dropUploadTarget !== 'custom'"
        @keydown.enter.prevent="emit('confirm-drop-upload')"
      />
      <footer>
        <button @click="emit('cancel-drop-upload')">{{ t("cancel") }}</button>
        <button class="primary-button" :disabled="dropUploadTarget === 'custom' && !normalizeDropTargetDir(dropUploadPathInput)" @click="emit('confirm-drop-upload')">{{ t("upload") }}</button>
      </footer>
      </template>
    </DialogContent>
  </Dialog>
</template>

<style scoped>
/* 终端拖入上传落点询问：文件清单限高滚动，路径行对齐 radio 观感。
   （自 app.css 迁入：规则只服务本组件内联弹窗的 DOM。） */
.drop-file-list { max-height: 132px; margin: 0; overflow: auto; white-space: pre; }
.drop-option { display: flex; align-items: center; gap: 7px; margin: 2px 0; font-size: 12px; cursor: pointer; }
.drop-option input { accent-color: var(--primary); }
.drop-option code { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted-foreground); font-size: 11px; }
.drop-path-input { width: 100%; height: 26px; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 12px; }
.drop-path-input:focus { outline: none; border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
.drop-path-input:disabled { opacity: .5; }
</style>
