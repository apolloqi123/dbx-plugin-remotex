<script setup lang="ts">
// 工作台内编辑器面板：紧凑页签条 + 当前页签正文。
//
// 纯展示/事件组件：状态与远端读写全部由 useEditorTabs 持有，App.vue 负责把
// tabs/activeId 绑定进来并转接 activate/close/save/update:text。正文复用
// TextPreview（异步按需加载，避免 CodeMirror 进入启动分包），图片页签使用
// 与预览弹窗同款缩放交互；加载/错误状态按页签展示。
import { computed, ref, watch } from "vue";
import { FileText, Image as ImageIcon, Loader2, Save, X } from "@lucide/vue";
import { createPreviewAsyncComponent } from "../../../shared/lib/asyncPreview";
import type { EditorTab } from "../composables/editor/useEditorTabs";

const props = defineProps<{
  tabs: EditorTab[];
  activeId: string;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  appearance: DbxPluginAppearance;
}>();

const emit = defineEmits<{
  activate: [id: string];
  close: [id: string];
  save: [id: string];
  "update:text": [value: { id: string; text: string }];
}>();

const TextPreview = createPreviewAsyncComponent(
  () => import("./TextPreview.vue"),
  () => ({
    loading: props.t("loading"),
    failed: props.t("previewDialog.loadFailed"),
    retry: props.t("previewDialog.retry"),
  }),
);

const activeTab = computed(() => props.tabs.find((tab) => tab.id === props.activeId));
// 图片缩放是页签内瞬时视图状态，不进入 EditorTab；切换页签时复位。
const imageZoomed = ref(false);
watch(
  () => props.activeId,
  () => {
    imageZoomed.value = false;
  },
);

function onTextChange(text: string) {
  if (!activeTab.value) return;
  emit("update:text", { id: activeTab.value.id, text });
}
</script>

<template>
  <section class="editor-pane" :lang="locale">
    <div class="editor-header">
      <div class="editor-tabs" role="tablist" aria-orientation="horizontal">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          class="editor-tab"
          :class="{ 'is-active': tab.id === activeId }"
          role="presentation"
        >
          <button
            :id="`editor-tab-${tab.id}`"
            type="button"
            class="editor-tab-main"
            role="tab"
            :aria-selected="tab.id === activeId"
            :aria-controls="`editor-panel-${tab.id}`"
            :title="tab.path"
            @click="emit('activate', tab.id)"
          >
            <ImageIcon v-if="tab.kind === 'image'" class="editor-tab-icon" aria-hidden="true" />
            <FileText v-else class="editor-tab-icon" aria-hidden="true" />
            <span class="editor-tab-name">{{ tab.name }}</span>
            <span
              v-if="tab.dirty"
              class="editor-tab-dirty"
              role="img"
              :title="t('editSave.unsaved')"
              :aria-label="t('editSave.unsaved')"
            />
          </button>
          <button
            type="button"
            class="editor-tab-close"
            :title="t('close')"
            :aria-label="t('close')"
            @click="emit('close', tab.id)"
          >
            <X aria-hidden="true" />
          </button>
        </div>
      </div>
      <button
        v-if="activeTab && activeTab.kind === 'text' && activeTab.editable"
        type="button"
        class="editor-save"
        :title="t('editSave.save')"
        :aria-label="t('editSave.save')"
        :disabled="!activeTab.dirty || activeTab.saving"
        @click="emit('save', activeTab.id)"
      >
        <Loader2 v-if="activeTab.saving" class="spinning" aria-hidden="true" />
        <Save v-else aria-hidden="true" />
        <span>{{ t("editSave.save") }}</span>
      </button>
    </div>

    <div
      v-if="activeTab"
      :id="`editor-panel-${activeId}`"
      class="editor-body"
      role="tabpanel"
      :aria-labelledby="`editor-tab-${activeId}`"
    >
      <div v-if="activeTab.loading" class="editor-state" role="status">
        <Loader2 class="spinning" aria-hidden="true" />{{ t("loading") }}
      </div>
      <div v-else-if="activeTab.error" class="editor-state editor-state--error" role="alert">
        {{ activeTab.error }}
      </div>
      <div v-else-if="activeTab.kind === 'image'" class="editor-image-stage">
        <img
          class="editor-image"
          :class="{ 'is-zoomed': imageZoomed }"
          :src="activeTab.imageUrl"
          :alt="activeTab.name"
          :title="imageZoomed ? t('imagePreview.zoomOut') : t('imagePreview.zoomIn')"
          @click="imageZoomed = !imageZoomed"
        />
      </div>
      <TextPreview
        v-else
        :text="activeTab.text"
        :file-name="activeTab.name"
        :appearance="appearance"
        :editable="activeTab.editable"
        :t="t"
        @change="onTextChange"
      />
    </div>
  </section>
</template>

<style scoped>
.editor-pane {
  display: flex;
  width: 100%;
  min-width: 0;
  min-height: 0;
  height: 100%;
  flex-direction: column;
  overflow: hidden;
  background: var(--background);
  color: var(--foreground);
}

.editor-header {
  display: flex;
  /* 2026-10-28：24 → 17px（约 2/3）。页签内容（10px 字/10px 图标）仍完整。 */
  min-height: 17px;
  flex: 0 0 auto;
  align-items: stretch;
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--background) 96%, var(--foreground));
}

.editor-tabs {
  display: flex;
  min-width: 0;
  flex: 1;
  overflow-x: auto;
  overflow-y: hidden;
}

.editor-tab {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  border-right: 1px solid var(--border);
}

.editor-tab.is-active {
  background: var(--ssh-terminal-background);
  box-shadow: inset 0 -2px 0 var(--primary);
}

.editor-tab-main {
  display: inline-flex;
  min-width: 0;
  max-width: 220px;
  min-height: 16px;
  align-items: center;
  gap: 4px;
  border: 0;
  padding: 1px 4px 1px 8px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 10.5px;
  cursor: pointer;
}

.editor-tab-main:hover,
.editor-tab.is-active .editor-tab-main {
  color: var(--foreground);
}

.editor-tab-icon {
  width: 10px;
  height: 10px;
  flex: 0 0 10px;
}

.editor-tab-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.editor-tab-dirty {
  width: 5px;
  height: 5px;
  flex: 0 0 5px;
  border-radius: 50%;
  background: var(--primary);
}

.editor-tab-close {
  display: inline-flex;
  width: 16px;
  height: 16px;
  flex: 0 0 16px;
  align-items: center;
  justify-content: center;
  margin-right: 2px;
  border: 0;
  border-radius: 3px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.editor-tab-close:hover {
  background: var(--accent);
  color: var(--foreground);
}

.editor-tab-close svg {
  width: 10px;
  height: 10px;
}

.editor-save {
  display: inline-flex;
  min-height: 15px;
  flex: 0 0 auto;
  align-self: center;
  align-items: center;
  gap: 3px;
  margin: 0 4px;
  border: 1px solid var(--border);
  border-radius: 3px;
  padding: 0 6px;
  background: var(--background);
  color: var(--foreground);
  font-size: 10px;
  cursor: pointer;
}

.editor-save:hover:not(:disabled) {
  background: var(--accent);
}

.editor-save:disabled {
  cursor: default;
  opacity: 0.42;
}

.editor-save svg {
  width: 10px;
  height: 10px;
  flex: 0 0 10px;
}

.editor-body {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.editor-body :deep(.preview-editor) {
  width: 100%;
  height: 100%;
}

.editor-state {
  display: flex;
  min-width: 0;
  flex: 1;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 16px;
  color: var(--muted-foreground);
  font-size: 12px;
  text-align: center;
}

.editor-state svg {
  width: 14px;
  height: 14px;
  flex: 0 0 14px;
}

.editor-state--error {
  color: var(--destructive);
  overflow-wrap: anywhere;
}

.editor-image-stage {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
  overflow: auto;
  background: repeating-conic-gradient(
      color-mix(in srgb, var(--muted-foreground) 22%, var(--background)) 0% 25%,
      var(--background) 0% 50%
    )
    0 0 / 20px 20px;
}

.editor-image {
  display: block;
  max-width: 100%;
  max-height: 100%;
  margin: auto;
  cursor: zoom-in;
}

.editor-image.is-zoomed {
  max-width: none;
  max-height: none;
  cursor: zoom-out;
}
</style>
