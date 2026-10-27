<script setup lang="ts">
import { Dialog, DialogContent, DialogTitle } from "../../shared/ui/dialog/index";
import { Switch } from "../../shared/ui/switch/index";
import { FolderOpen, Loader2, Trash2, X } from "@lucide/vue";
import { formatBytes } from "../../shared/lib/format";
import type { BatchProgressState } from "../../features/transfers/lib/sftpBatchProgress";

/** SFTP 文件操作弹窗组：新建目录/符号链接、外部编辑器回传、自定义编辑器命令、
 *  权限/删除/批量删除、下载目录与冲突询问、上传重复询问、新建文件、文件属性。
 *  状态与 RPC 仍由 App 的 SFTP composables 持有；弹窗壳/骨架走全局 modal 样式
 *  （reka portal），组件只渲染并上抛动作。 */
type PermRole = { readonly who: number; readonly key: string };
type PermColumn = { readonly bit: number; readonly key: string };
type SymlinkDialogState = { mode: "create" | "edit"; name: string; linkPath: string };
type DownloadPromptState = { fileName: string; dir: string; setDefault: boolean };
type DownloadConflictPromptState = { fileName: string; path: string };
type UploadDuplicatePromptState = { fileName: string; path: string };
type AttrStatView = { path?: string; kind?: string; size?: number; modifiedAt?: number; mode?: string };

const props = defineProps<{
  operationDialog: string | null;
  operationDraft: string;
  symlinkDialog: SymlinkDialogState | null | undefined;
  symlinkDraft: string;
  symlinkTargetDraft: string;
  symlinkSubmitting: boolean;
  watchModifiedQueue: ReadonlyArray<unknown>;
  watchModifiedPrompt: { name?: string } | null | undefined;
  externalEditBusy: boolean;
  customEditorOpen: boolean;
  customEditorName: string;
  customEditorCommand: string;
  customEditorRemember: boolean;
  customEditorPattern: string;
  chmodTarget: { name: string } | undefined;
  chmodDraft: string;
  chmodSubmitting: boolean;
  deleteTarget: { name: string } | undefined;
  deleteSubmitting: boolean;
  batchDeleteOpen: boolean;
  selectedEntries: ReadonlyArray<{ name: string }>;
  batchProgress: BatchProgressState | null | undefined;
  batchDeleteSubmitting: boolean;
  downloadPrompt: DownloadPromptState | null | undefined;
  localCanSave: boolean;
  downloadConflictPrompt: DownloadConflictPromptState | null | undefined;
  uploadDuplicatePrompt: UploadDuplicatePromptState | null | undefined;
  uploadDuplicateApplyAll: boolean;
  newFileDialog: boolean;
  newFileDraft: string;
  newFileSubmitting: boolean;
  attrsTarget: { name: string } | undefined;
  attrsLoading: boolean;
  attrsInfo: AttrStatView | undefined;
  attrsMode: string;
  attrsSubmitting: boolean;
  attrsOwnerDisplay: string;
  canWrite: boolean;
  permRoles: ReadonlyArray<PermRole>;
  permColumns: ReadonlyArray<PermColumn>;
  permBit: (raw: string, who: number, bit: number) => boolean;
  batchProgressPercent: (state: BatchProgressState) => number;
  formatModified: (value?: number) => string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:operationDialog", value: string | null): void;
  (e: "update:operationDraft", value: string): void;
  (e: "update:symlinkDialog", value: SymlinkDialogState | null): void;
  (e: "update:symlinkDraft", value: string): void;
  (e: "update:symlinkTargetDraft", value: string): void;
  (e: "update:customEditorOpen", value: boolean): void;
  (e: "update:customEditorName", value: string): void;
  (e: "update:customEditorCommand", value: string): void;
  (e: "update:customEditorRemember", value: boolean): void;
  (e: "update:chmodTarget", value: { name: string } | undefined): void;
  (e: "update:chmodDraft", value: string): void;
  (e: "update:deleteTarget", value: { name: string } | undefined): void;
  (e: "update:batchDeleteOpen", value: boolean): void;
  (e: "update:downloadPrompt", value: DownloadPromptState | undefined): void;
  (e: "update:uploadDuplicateApplyAll", value: boolean): void;
  (e: "update:newFileDialog", value: boolean): void;
  (e: "update:newFileDraft", value: string): void;
  (e: "update:attrsMode", value: string): void;
  (e: "create-directory"): void;
  (e: "commit-symlink"): void;
  (e: "dismiss-watch-modified"): void;
  (e: "upload-watched-always"): void;
  (e: "upload-watched-single"): void;
  (e: "commit-custom-editor"): void;
  (e: "confirm-chmod"): void;
  (e: "confirm-delete"): void;
  (e: "confirm-batch-delete"): void;
  (e: "resolve-download-prompt", value: { dir: string; setDefault: boolean } | undefined): void;
  (e: "resolve-download-conflict", value: "overwrite" | "rename" | undefined): void;
  (e: "resolve-upload-duplicate", value: "overwrite" | "rename" | undefined): void;
  (e: "open-folder-picker"): void;
  (e: "create-new-file"): void;
  (e: "close-attributes"): void;
  (e: "toggle-chmod-perm", who: number, bit: number, event: Event): void;
  (e: "toggle-attrs-perm", who: number, bit: number, event: Event): void;
  (e: "save-attributes-permissions"): void;
}>();

const t = props.t;
</script>

<template>
    <Dialog :open="operationDialog === 'mkdir'" @update:open="(open) => { if (!open) emit('update:operationDialog', null); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("newFolder") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:operationDialog', null)"><X /></button></header>
        <input :value="operationDraft" @input="emit('update:operationDraft', ($event.target as HTMLInputElement).value)" autofocus @keydown.enter="emit('create-directory')" />
        <footer><button @click="emit('update:operationDialog', null)">{{ t("cancel") }}</button><button class="primary-button" :disabled="!operationDraft.trim()" @click="emit('create-directory')">{{ t("confirm") }}</button></footer>      </DialogContent>
    </Dialog>

    <!-- 符号链接新建/改指向（P2-6）：target 允许相对路径（symlink 语义），
         编辑模式预填当前指向；改指向后端 readlink 比对做 no-op 兜底。 -->
    <Dialog :open="symlinkDialog !== null" @update:open="(open) => { if (!open) emit('update:symlinkDialog', null); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="symlinkDialog">
        <header><DialogTitle>{{ symlinkDialog.mode === "edit" ? t("symlink.editTitle", { name: symlinkDialog.name }) : t("symlink.createTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:symlinkDialog', null)"><X /></button></header>
        <p v-if="symlinkDialog.mode === 'edit'" class="muted mono">{{ symlinkDialog.linkPath }}</p>
        <input v-if="symlinkDialog.mode === 'create'" :value="symlinkDraft" @input="emit('update:symlinkDraft', ($event.target as HTMLInputElement).value)" autofocus :placeholder="t('symlink.namePlaceholder')" @keydown.enter="emit('commit-symlink')" />
        <input v-if="symlinkDialog.mode === 'create'" :value="symlinkTargetDraft" @input="emit('update:symlinkTargetDraft', ($event.target as HTMLInputElement).value)" class="mono" spellcheck="false" :placeholder="t('symlink.targetPlaceholder')" @keydown.enter="emit('commit-symlink')" />
        <input v-else :value="symlinkDraft" @input="emit('update:symlinkDraft', ($event.target as HTMLInputElement).value)" class="mono" spellcheck="false" autofocus :placeholder="t('symlink.targetPlaceholder')" @keydown.enter="emit('commit-symlink')" />
        <p class="muted">{{ t("symlink.hint") }}</p>
        <footer><button @click="emit('update:symlinkDialog', null)">{{ t("cancel") }}</button><button class="primary-button" :disabled="(symlinkDialog.mode === 'create' ? !symlinkDraft.trim() || !symlinkTargetDraft.trim() : !symlinkDraft.trim()) || symlinkSubmitting" @click="emit('commit-symlink')"><Loader2 v-if="symlinkSubmitting" class="spinning" />{{ t("confirm") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 外部编辑器保存回传确认（P2-5；M15 起逐文件排队）：队列头逐个弹，
    决议（上传一次 / 总是上传（记住 watchId）/ 取消）才出队，多文件互不顶替。 -->
    <Dialog :open="watchModifiedQueue.length > 0" @update:open="(open) => { if (!open) emit('dismiss-watch-modified'); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpEdit.modifiedTitle") }}</DialogTitle><Loader2 v-if="externalEditBusy" class="spinning" /><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('dismiss-watch-modified')"><X /></button></header>
        <p class="sftp-dialog-hint">{{ t("sftpEdit.modifiedMessage", { name: watchModifiedPrompt?.name || "" }) }}</p>
        <footer>
          <button @click="emit('dismiss-watch-modified')">{{ t("cancel") }}</button>
          <button @click="emit('upload-watched-always')">{{ t("sftpEdit.alwaysUpload") }}</button>
          <button class="primary-button" @click="emit('upload-watched-single')">{{ t("sftpEdit.uploadOnce") }}</button>
        </footer>
      </DialogContent>
    </Dialog>

    <!-- 自定义编辑器命令（编辑器映射增强）：name/command 记入 pluginStore
         单键配置，「记住为默认」写 {pattern → customId} 关联；确定即用该命令
         打开本次文件。命令串由 sidecar 分词成 argv 直接 spawn（不过 shell）。 -->
    <Dialog :open="customEditorOpen" @update:open="(open) => { if (!open) emit('update:customEditorOpen', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpEdit.customCommandTitle") }}</DialogTitle><button :title="t('close')" class="icon-button" @click="emit('update:customEditorOpen', false)"><X /></button></header>
        <input :value="customEditorName" @input="emit('update:customEditorName', ($event.target as HTMLInputElement).value)" :placeholder="t('sftpEdit.customEditorNamePlaceholder')" />
        <input :value="customEditorCommand" @input="emit('update:customEditorCommand', ($event.target as HTMLInputElement).value)" class="mono" spellcheck="false" autofocus :placeholder="t('sftpEdit.customEditorCommandPlaceholder')" />
        <p class="muted">{{ t("sftpEdit.customEditorCommandHint") }}</p>
        <label class="settings-switch-row">
          <Switch :model-value="customEditorRemember" size="sm" @update:model-value="emit('update:customEditorRemember', $event)" />
          <span>{{ t("sftpEdit.rememberAssociation", { pattern: customEditorPattern }) }}</span>
        </label>
        <footer><button @click="emit('update:customEditorOpen', false)">{{ t("cancel") }}</button><button class="primary-button" :disabled="!customEditorCommand.trim()" @click="emit('commit-custom-editor')">{{ t("sftpEdit.customCommandRun") }}</button></footer>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!chmodTarget" @update:open="(open) => { if (!open) emit('update:chmodTarget', undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="chmodTarget">
        <header><DialogTitle>{{ t("permissionsEdit") }} · {{ chmodTarget.name }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:chmodTarget', undefined)"><X /></button></header>
        <div class="perm-matrix" role="group" :aria-label="t('permissionsEdit')">
          <span></span>
          <span v-for="column in permColumns" :key="column.bit" class="perm-matrix-head">{{ t(column.key) }}</span>
          <template v-for="role in permRoles" :key="role.who">
            <span class="perm-matrix-role">{{ t(role.key) }}</span>
            <label v-for="column in permColumns" :key="column.bit" class="perm-matrix-cell">
              <input type="checkbox" :checked="permBit(chmodDraft, role.who, column.bit)" @change="emit('toggle-chmod-perm', role.who, column.bit, $event)" />
            </label>
          </template>
        </div>
        <input :value="chmodDraft" @input="emit('update:chmodDraft', ($event.target as HTMLInputElement).value)" class="mono" spellcheck="false" :placeholder="t('permissionsPlaceholder')" @keydown.enter="emit('confirm-chmod')" />
        <p class="muted">{{ t("permissionsHint") }}</p>
        <footer><button @click="emit('update:chmodTarget', undefined)">{{ t("cancel") }}</button><button class="primary-button" :disabled="!chmodDraft.trim() || chmodSubmitting" @click="emit('confirm-chmod')">{{ t("confirm") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <Dialog :open="!!deleteTarget" @update:open="(open) => { if (!open) emit('update:deleteTarget', undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="deleteTarget">
        <header><DialogTitle>{{ t("deleteTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:deleteTarget', undefined)"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ deleteTarget.name }}</strong><p class="muted">{{ t("deleteMessage") }}</p></div></div>
        <footer><button @click="emit('update:deleteTarget', undefined)">{{ t("cancel") }}</button><button class="danger-button" :disabled="deleteSubmitting" @click="emit('confirm-delete')"><Trash2 />{{ t("delete") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>

    <Dialog :open="batchDeleteOpen" @update:open="(open) => { if (!open) emit('update:batchDeleteOpen', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpBatch.deleteTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:batchDeleteOpen', false)"><X /></button></header>
        <div class="destructive-copy"><div><strong>{{ t("sftpBatch.selected", { count: selectedEntries.length }) }}</strong><p class="muted">{{ t("sftpBatch.deleteMessage") }}</p></div></div>
        <div v-if="batchProgress" class="batch-progress-row"><progress class="batch-progress-bar" :value="batchProgressPercent(batchProgress)" max="100" /><span class="batch-progress mono">{{ t("sftpBatch.progress", { done: batchProgress.done, total: batchProgress.total }) }}</span></div>
        <footer><button @click="emit('update:batchDeleteOpen', false)" :disabled="batchDeleteSubmitting">{{ t("cancel") }}</button><button class="danger-button" :disabled="batchDeleteSubmitting" @click="emit('confirm-batch-delete')"><Loader2 v-if="batchDeleteSubmitting" class="spinning" /><Trash2 v-else />{{ t("delete") }}</button></footer>
      </DialogContent>
    </Dialog>

    <!-- 录制删除确认：应用内弹窗替代 window.confirm（宿主沙箱 iframe 无 allow-modals，confirm 恒 false） -->
    <!-- 下载/导出前的保存目录选择（设置里开启「每次询问」时出现） -->
    <Dialog :open="!!downloadPrompt" @update:open="(open) => { if (!open) emit('resolve-download-prompt', undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="downloadPrompt">
        <header><DialogTitle>{{ t("downloadSettings.askTitle") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="emit('resolve-download-prompt', undefined)"><X /></button></header>
        <p class="muted mono">{{ downloadPrompt.fileName }}</p>
        <label class="settings-field">
          <span>{{ t("downloadSettings.directory") }}</span>
          <span class="settings-dir-row">
            <input :value="downloadPrompt.dir" @input="emit('update:downloadPrompt', { ...downloadPrompt, dir: ($event.target as HTMLInputElement).value })" class="mono" spellcheck="false" autofocus @keydown.enter="emit('resolve-download-prompt', { dir: downloadPrompt.dir, setDefault: downloadPrompt.setDefault })" />
            <button v-if="localCanSave" type="button" class="browse-button" :title="t('downloadSettings.browse')" :aria-label="t('downloadSettings.browse')" @click="emit('open-folder-picker')"><FolderOpen /></button>
          </span>
        </label>
        <label class="settings-field settings-switch-row">
          <input :checked="downloadPrompt.setDefault" @change="emit('update:downloadPrompt', { ...downloadPrompt, setDefault: ($event.target as HTMLInputElement).checked })" type="checkbox" />
          <span>{{ t("downloadSettings.setDefaultThisTime") }}</span>
        </label>
        <p class="muted settings-note">{{ t("downloadSettings.askHint") }}</p>
        <footer>
          <button @click="emit('resolve-download-prompt', undefined)">{{ t("cancel") }}</button>
          <button class="primary-button" @click="emit('resolve-download-prompt', { dir: downloadPrompt.dir, setDefault: downloadPrompt.setDefault })">{{ t("save") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>

    <!-- 「询问我」冲突策略：目标目录已有同名文件时的选择 -->
    <Dialog :open="!!downloadConflictPrompt" @update:open="(open) => { if (!open) emit('resolve-download-conflict', undefined); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="downloadConflictPrompt">
        <header><DialogTitle>{{ t("downloadConflict.title") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="emit('resolve-download-conflict', undefined)"><X /></button></header>
        <p>{{ t("downloadConflict.message", { name: downloadConflictPrompt.fileName }) }}</p>
        <p class="muted mono">{{ downloadConflictPrompt.path }}</p>
        <footer>
          <button @click="emit('resolve-download-conflict', undefined)">{{ t("cancel") }}</button>
          <button class="danger-button" @click="emit('resolve-download-conflict', 'overwrite')">{{ t("downloadSettings.conflict.overwrite") }}</button>
          <button class="primary-button" @click="emit('resolve-download-conflict', 'rename')">{{ t("downloadSettings.conflict.rename") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- 上传重复目标「询问我」（P1-5）：重命名 / 覆盖 / 取消，支持应用到本批次 -->
    <Dialog :open="!!uploadDuplicatePrompt" @update:open="(open) => { if (!open) emit('resolve-upload-duplicate', undefined); }">      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="uploadDuplicatePrompt">
        <header><DialogTitle>{{ t("transferCfg.duplicateTitle") }}</DialogTitle><button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="emit('resolve-upload-duplicate', undefined)"><X /></button></header>
        <p>{{ t("transferCfg.duplicateMessage", { name: uploadDuplicatePrompt.fileName }) }}</p>
        <p class="muted mono">{{ uploadDuplicatePrompt.path }}</p>
        <label class="settings-field settings-switch-row">
          <input :checked="uploadDuplicateApplyAll" @change="emit('update:uploadDuplicateApplyAll', ($event.target as HTMLInputElement).checked)" type="checkbox" />
          <span>{{ t("transferCfg.applyAll") }}</span>
        </label>
        <footer>
          <button @click="emit('resolve-upload-duplicate', undefined)">{{ t("cancel") }}</button>
          <button class="danger-button" @click="emit('resolve-upload-duplicate', 'overwrite')">{{ t("transferCfg.policy.overwrite") }}</button>
          <button class="primary-button" @click="emit('resolve-upload-duplicate', 'rename')">{{ t("transferCfg.policy.rename") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>
    <Dialog :open="newFileDialog" @update:open="(open) => { if (!open) emit('update:newFileDialog', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("sftpNewFile.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:newFileDialog', false)"><X /></button></header>
        <input :value="newFileDraft" @input="emit('update:newFileDraft', ($event.target as HTMLInputElement).value)" autofocus spellcheck="false" :placeholder="t('sftpNewFile.placeholder')" @keydown.enter="emit('create-new-file')" />
        <footer><button @click="emit('update:newFileDialog', false)">{{ t("cancel") }}</button><button class="primary-button" :disabled="!newFileDraft.trim() || newFileSubmitting" @click="emit('create-new-file')"><Loader2 v-if="newFileSubmitting" class="spinning" />{{ t("confirm") }}</button></footer>
      </DialogContent>
    </Dialog>
    <Dialog :open="!!attrsTarget" @update:open="(open) => { if (!open) emit('close-attributes'); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="attrsTarget">
        <header><DialogTitle>{{ t("sftpAttrs.title") }} · {{ attrsTarget.name }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('close-attributes')"><X /></button></header>
        <div v-if="attrsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <template v-else-if="attrsInfo">
          <dl class="attrs-grid">
            <dt>{{ t("sftpAttrs.path") }}</dt><dd class="mono">{{ attrsInfo.path }}</dd>
            <dt>{{ t("sftpAttrs.type") }}</dt><dd>{{ t(`sftpAttrs.kind.${attrsInfo.kind}`) }}</dd>
            <dt>{{ t("size") }}</dt><dd class="numeric">{{ attrsInfo.kind === "directory" ? "–" : formatBytes(attrsInfo.size || 0) }}</dd>
            <dt>{{ t("sftpAttrs.permissions") }}</dt><dd class="mono">{{ attrsInfo.mode || "–" }}</dd>
            <dt>{{ t("sftpAttrs.owner") }}</dt><dd>{{ attrsOwnerDisplay }}</dd>
            <dt>{{ t("sftpAttrs.modified") }}</dt><dd>{{ formatModified(attrsInfo.modifiedAt) || "–" }}</dd>
          </dl>
          <div class="perm-matrix" role="group" :aria-label="t('sftpAttrs.permissions')">
            <span></span>
            <span v-for="column in permColumns" :key="column.bit" class="perm-matrix-head">{{ t(column.key) }}</span>
            <template v-for="role in permRoles" :key="role.who">
              <span class="perm-matrix-role">{{ t(role.key) }}</span>
              <label v-for="column in permColumns" :key="column.bit" class="perm-matrix-cell">
                <input type="checkbox" :disabled="!canWrite" :checked="permBit(attrsMode, role.who, column.bit)" @change="emit('toggle-attrs-perm', role.who, column.bit, $event)" />
              </label>
            </template>
          </div>
          <label class="attrs-permissions-edit">
            <span>{{ t("sftpAttrs.permissions") }}</span>
            <input :value="attrsMode" @input="emit('update:attrsMode', ($event.target as HTMLInputElement).value)" class="mono" spellcheck="false" :placeholder="t('permissionsPlaceholder')" :disabled="!canWrite" @keydown.enter="emit('save-attributes-permissions')" />
          </label>
          <p class="muted">{{ t("permissionsHint") }}</p>
        </template>
        <footer>
          <button @click="emit('close-attributes')">{{ t("close") }}</button>
          <button class="primary-button" :disabled="!canWrite || !attrsMode.trim() || attrsSubmitting" @click="emit('save-attributes-permissions')"><Loader2 v-if="attrsSubmitting" class="spinning" />{{ t("sftpAttrs.save") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>
</template>

<style scoped>
/* 权限矩阵与属性行：仅文件属性/权限弹窗使用（原 App scoped 规则随 DOM 迁入）。 */
.attrs-grid { display: grid; grid-template-columns: auto 1fr; gap: 6px 14px; margin: 0; font-size: 12px; }
.attrs-grid dt { max-width: 16ch; overflow: hidden; color: var(--muted-foreground); text-overflow: ellipsis; white-space: nowrap; }
.attrs-grid dd { margin: 0; overflow-wrap: anywhere; }
.attrs-permissions-edit { display: flex; align-items: center; gap: 8px; font-size: 12px; }
.attrs-permissions-edit span { flex: 0 0 auto; color: var(--muted-foreground); }
.attrs-permissions-edit input { flex: 1; }
.perm-matrix { display: grid; grid-template-columns: minmax(56px, auto) repeat(3, 1fr); gap: 4px 6px; align-items: center; margin: 2px 0 8px; font-size: 12px; }
.perm-matrix-head { color: var(--muted-foreground); font-size: 11px; text-align: center; }
.perm-matrix-role { color: var(--muted-foreground); white-space: nowrap; }
.perm-matrix-cell { display: flex; justify-content: center; }
.perm-matrix-cell input { width: 13px; height: 13px; margin: 0; accent-color: var(--primary); }
/* 批量删除进度行（与 SFTP 面板批量条共用同名规则，App 侧保留）。 */
.batch-progress-bar { flex: 0 1 140px; height: 6px; min-width: 80px; accent-color: var(--primary); }
.batch-progress-row { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
.batch-progress-row .batch-progress-bar { flex: 1; }
</style>
