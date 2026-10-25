<script setup lang="ts">
// 设置·传输分栏（下载目录/冲突策略、SFTP 外部编辑器、并发与重复目标、会话
// 并发深度/兼容模式/文件名编码、压缩传输）。草稿由弹窗持有并随主「保存」落库；
// 本组件只做表单渲染与行内校验。
import { computed } from "vue";
import { FolderOpen, Plus, Trash2 } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import { DOWNLOAD_CONFLICT_POLICIES, type DownloadConflictPolicy } from "../../sftp/lib/transfer/downloadPrefs";
import { type EditorAssociation, type EditorConfig, type KnownEditor } from "../../sftp/lib/editor/editorRules";
import { TRANSFER_DUPLICATE_POLICIES, type TransferDuplicatePolicy } from "../../transfers/lib/transferQueue";
import { SFTP_NAME_ENCODINGS, type SftpNameEncoding } from "../../sftp/lib/entries/sftpName";

/** 传输分栏的共享草稿（弹窗主保存链读取同一份响应式对象）。 */
interface TransferDrafts {
  downloadDir: string;
  downloadUseDefault: boolean;
  downloadConflict: DownloadConflictPolicy;
  editorConfig: EditorConfig;
  editorAssociationPattern: string;
  editorAssociationTarget: string;
  editorDefault: string;
  editorKnownEditors: KnownEditor[];
  concurrency: string;
  duplicatePolicy: TransferDuplicatePolicy;
  maxActive: string;
  downloadLimit: string;
  compatMode: boolean;
  nameEncoding: SftpNameEncoding;
  compressMode: string;
  compressThreshold: string;
}

const props = defineProps<{
  drafts: TransferDrafts;
  localDownloadDir: string;
  localCanSave: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "browse-download-dir"): void;
}>();

const editorTargetOptions = computed(() => [
  ...props.drafts.editorKnownEditors.filter((editor) => editor.available).map((editor) => ({ value: `known:${editor.id}`, label: editor.name })),
  ...props.drafts.editorConfig.customEditors.map((editor) => ({ value: `custom:${editor.id}`, label: editor.name })),
]);
function editorTargetName(target: string): string {
  if (target.startsWith("custom:")) {
    const id = target.slice(7);
    return props.drafts.editorConfig.customEditors.find((editor) => editor.id === id)?.name || id;
  }
  if (target.startsWith("known:")) {
    const id = target.slice(6);
    return props.drafts.editorKnownEditors.find((editor) => editor.id === id)?.name || id;
  }
  return target;
}
function associationTargetValue(assoc: EditorAssociation): string {
  return assoc.customId ? `custom:${assoc.customId}` : assoc.editorId ? `known:${assoc.editorId}` : "";
}
function addEditorAssociation() {
  const pattern = props.drafts.editorAssociationPattern.trim();
  const target = props.drafts.editorAssociationTarget;
  if (!pattern || !target) return;
  const assoc: EditorAssociation = target.startsWith("custom:")
    ? { pattern, customId: target.slice("custom:".length) }
    : { pattern, editorId: target.slice("known:".length) };
  props.drafts.editorConfig = { ...props.drafts.editorConfig, associations: [...props.drafts.editorConfig.associations, assoc] };
  props.drafts.editorAssociationPattern = "";
  props.drafts.editorAssociationTarget = "";
}
function removeEditorAssociation(index: number) {
  props.drafts.editorConfig = {
    ...props.drafts.editorConfig,
    associations: props.drafts.editorConfig.associations.filter((_, i) => i !== index),
  };
}
function removeCustomEditor(id: string) {
  props.drafts.editorConfig = {
    ...props.drafts.editorConfig,
    customEditors: props.drafts.editorConfig.customEditors.filter((editor) => editor.id !== id),
    associations: props.drafts.editorConfig.associations.filter((assoc) => assoc.customId !== id),
  };
  if (props.drafts.editorDefault === `custom:${id}`) props.drafts.editorDefault = "system";
}

// M33：压缩传输策略与阈值草稿（阈值 0=不限下限，上限 65536 MiB）。
const TRANSFER_COMPRESS_MODES = ["auto", "on", "off"] as const;
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("downloadSettings.title") }}</h3>
    <label class="settings-field">
      <span>{{ t("downloadSettings.directory") }}</span>
      <span class="settings-dir-row">
        <input v-model="drafts.downloadDir" class="mono" spellcheck="false" :placeholder="localDownloadDir || t('downloadSettings.default')" />
        <button v-if="localCanSave" type="button" class="browse-button" :title="t('downloadSettings.browse')" :aria-label="t('downloadSettings.browse')" @click="emit('browse-download-dir')"><FolderOpen /></button>
      </span>
    </label>
    <label class="settings-field settings-switch-row">
      <Switch v-model="drafts.downloadUseDefault" size="sm" />
      <span>{{ t("downloadSettings.useDefaultDir") }}</span>
    </label>
    <p class="muted settings-note">{{ t("downloadSettings.useDefaultDirHint") }}</p>
    <h3 class="settings-section-title">{{ t("downloadSettings.conflictTitle") }}</h3>
    <label v-for="policy in DOWNLOAD_CONFLICT_POLICIES" :key="policy" class="settings-field settings-radio-row">
      <input v-model="drafts.downloadConflict" type="radio" name="download-conflict-policy" :value="policy" />
      <span>{{ t(`downloadSettings.conflict.${policy}`) }}</span>
    </label>
    <p class="muted settings-note">{{ t("downloadSettings.conflictHint") }}</p>
    <p class="muted settings-note">{{ t("downloadSettings.hint") }}</p>
    <h3 class="settings-section-title">{{ t("sftpEdit.settingsSection") }}</h3>
    <label class="settings-field">
      <span>{{ t("sftpEdit.defaultEditorTitle") }}</span>
      <Select :model-value="drafts.editorDefault" @update:model-value="(v) => (drafts.editorDefault = String(v))">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="system">{{ t("sftpEdit.defaultEditorSystem") }}</SelectItem>
          <SelectItem v-for="option in editorTargetOptions" :key="option.value" :value="option.value">{{ option.label }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ t("sftpEdit.defaultEditorHint") }}</p>
    <span class="settings-field"><span>{{ t("sftpEdit.uploadPolicyTitle") }}</span></span>
    <label class="settings-field settings-radio-row">
      <input v-model="drafts.editorConfig.uploadPolicy" type="radio" name="editor-upload-policy" value="auto" />
      <span>{{ t("sftpEdit.uploadPolicy.auto") }}</span>
    </label>
    <label class="settings-field settings-radio-row">
      <input v-model="drafts.editorConfig.uploadPolicy" type="radio" name="editor-upload-policy" value="ask" />
      <span>{{ t("sftpEdit.uploadPolicy.ask") }}</span>
    </label>
    <p class="muted settings-note">{{ t("sftpEdit.uploadPolicyHint") }}</p>
    <h3 class="settings-section-title">{{ t("sftpEdit.associationsTitle") }}</h3>
    <p v-if="!drafts.editorConfig.associations.length" class="muted settings-note">{{ t("sftpEdit.noAssociations") }}</p>
    <div v-for="(assoc, index) in drafts.editorConfig.associations" :key="`${assoc.pattern}-${index}`" class="flex items-center gap-2 py-0.5">
      <span class="mono">{{ assoc.pattern }}</span>
      <span class="muted truncate">→ {{ editorTargetName(associationTargetValue(assoc)) }}</span>
      <button type="button" class="icon-button" :title="t('delete')" :aria-label="t('delete')" @click="removeEditorAssociation(index)"><Trash2 /></button>
    </div>
    <div class="settings-dir-row">
      <input v-model="drafts.editorAssociationPattern" class="mono" spellcheck="false" :placeholder="t('sftpEdit.associationPatternPlaceholder')" />
      <Select :model-value="drafts.editorAssociationTarget" @update:model-value="(v) => (drafts.editorAssociationTarget = String(v))">
        <SelectTrigger size="xs"><SelectValue :placeholder="t('sftpEdit.associationTargetPlaceholder')" /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="option in editorTargetOptions" :key="option.value" :value="option.value">{{ option.label }}</SelectItem>
        </SelectContent>
      </Select>
    </div>
    <button type="button" :disabled="!drafts.editorAssociationPattern.trim() || !drafts.editorAssociationTarget" @click="addEditorAssociation"><Plus />{{ t("sftpEdit.addAssociation") }}</button>
    <p class="muted settings-note">{{ t("sftpEdit.associationsHint") }}</p>
    <h3 class="settings-section-title">{{ t("sftpEdit.customEditorsTitle") }}</h3>
    <p v-if="!drafts.editorConfig.customEditors.length" class="muted settings-note">{{ t("sftpEdit.noCustomEditors") }}</p>
    <div v-for="editor in drafts.editorConfig.customEditors" :key="editor.id" class="flex items-center gap-2 py-0.5">
      <span class="shrink-0">{{ editor.name }}</span>
      <span class="muted mono truncate">{{ editor.command }}</span>
      <button type="button" class="icon-button" :title="t('delete')" :aria-label="t('delete')" @click="removeCustomEditor(editor.id)"><Trash2 /></button>
    </div>
    <h3 class="settings-section-title">{{ t("transferCfg.title") }}</h3>
    <label class="settings-field">
      <span>{{ t("transferCfg.concurrency") }}</span>
      <input v-model="drafts.concurrency" type="number" min="1" max="10" step="1" @change="drafts.concurrency = String(Math.min(10, Math.max(1, Number.parseInt(drafts.concurrency, 10) || 3)))" />
    </label>
    <p class="muted settings-note">{{ t("transferCfg.concurrencyHint") }}</p>
    <label class="settings-field">
      <span>{{ t("transferCfg.duplicatePolicy") }}</span>
      <Select :model-value="drafts.duplicatePolicy" @update:model-value="(v) => (drafts.duplicatePolicy = String(v) as TransferDuplicatePolicy)">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="policy in TRANSFER_DUPLICATE_POLICIES" :key="policy" :value="policy">{{ t(`transferCfg.policy.${policy}`) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ t("transferCfg.duplicatePolicyHint") }}</p>
    <h3 class="settings-section-title">{{ t("transferCfg.pipelineTitle") }}</h3>
    <label class="settings-field">
      <span>{{ t("transferCfg.maxActive") }}</span>
      <input v-model="drafts.maxActive" type="number" min="1" max="8" step="1" @change="drafts.maxActive = String(Math.min(8, Math.max(1, Number.parseInt(drafts.maxActive, 10) || 3)))" />
    </label>
    <p class="muted settings-note">{{ t("transferCfg.maxActiveHint") }}</p>
    <label class="settings-field">
      <span>{{ t("transferCfg.downloadLimit") }}</span>
      <input v-model="drafts.downloadLimit" type="number" min="0" max="1048576" step="64" @change="drafts.downloadLimit = String(Math.min(1048576, Math.max(0, Number.parseInt(drafts.downloadLimit, 10) || 0)))" />
    </label>
    <p class="muted settings-note">{{ t("transferCfg.downloadLimitHint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch v-model="drafts.compatMode" size="sm" />
      <span>{{ t("transferCfg.compatMode") }}</span>
    </label>
    <p class="muted settings-note">{{ t("transferCfg.compatModeHint") }}</p>
    <label class="settings-field">
      <span>{{ t("transferCfg.nameEncoding") }}</span>
      <Select :model-value="drafts.nameEncoding" @update:model-value="(v) => (drafts.nameEncoding = String(v) as SftpNameEncoding)">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="encoding in SFTP_NAME_ENCODINGS" :key="encoding" :value="encoding">{{ t(`transferCfg.encoding.${encoding}`) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ t("transferCfg.nameEncodingHint") }}</p>
    <h3 class="settings-section-title">{{ t("transferCfg.compressTitle") }}</h3>
    <label class="settings-field">
      <span>{{ t("transferCfg.compressMode") }}</span>
      <Select :model-value="drafts.compressMode" @update:model-value="(v) => (drafts.compressMode = String(v))">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="mode in TRANSFER_COMPRESS_MODES" :key="mode" :value="mode">{{ t(`transferCfg.compressMode.${mode}`) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ t("transferCfg.compressModeHint") }}</p>
    <label class="settings-field">
      <span>{{ t("transferCfg.compressThreshold") }}</span>
      <input v-model="drafts.compressThreshold" type="number" min="0" max="65536" step="16" :disabled="drafts.compressMode === 'off'" @change="drafts.compressThreshold = String(Math.min(65536, Math.max(0, Number.parseInt(drafts.compressThreshold, 10) || 0)))" />
    </label>
    <p class="muted settings-note">{{ t("transferCfg.compressThresholdHint") }}</p>
  </div>
</template>
