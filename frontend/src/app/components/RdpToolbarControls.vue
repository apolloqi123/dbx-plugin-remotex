<script setup lang="ts">
import { ClipboardPaste, PlugZap, Proportions, Scan, StretchHorizontal } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import type { RdpToolbarActions } from "../lib/workbenchToolbarActions";

/** RDP 独立工具组（T19 自 App.vue 模板迁入）：缩放三档 + 桌面分辨率 +
 *  大文本粘贴 + 断开。与 SSH 组互斥，由父级按 isRdpMode 门控挂载；
 *  分辨率/粘贴状态与 rdp/resize、rdp/input 调用仍由 App 的 useRdpWorkbench 持有。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  /** `useRdpWorkbench` 的预设表（静态常量，无需响应式）。 */
  resolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  rdpResizing: boolean;
  rdpDesktopSize: { width: number; height: number } | null;
  rdpTypingBusy: boolean;
  rdpTypingSent: number;
  rdpTypingTotal: number;
  actions: RdpToolbarActions;
}>();

const rdpScaleMode = defineModel<"fit" | "stretch" | "actual">("rdpScaleMode", { required: true });
const rdpResolutionOpen = defineModel<boolean>("rdpResolutionOpen", { required: true });
const rdpCustomWidth = defineModel<string>("rdpCustomWidth", { required: true });
const rdpCustomHeight = defineModel<string>("rdpCustomHeight", { required: true });
const rdpPasteOpen = defineModel<boolean>("rdpPasteOpen", { required: true });
const rdpPasteText = defineModel<string>("rdpPasteText", { required: true });

const t = props.t;
const RDP_RESOLUTION_PRESETS = props.resolutionPresets;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  toggleRdpResolutionMenu,
  applyRdpResolution,
  rdpResolutionLabel,
  isRdpResolutionActive,
  rdpWindowProportionPreset,
  applyRdpCustomResolution,
  toggleRdpPasteMenu,
  rdpWriteRemoteClipboard,
  rdpTypeAsKeystrokes,
  closeRdpSession,
} = props.actions;
</script>

<template>
  <div class="rdp-scale-group" role="group" :aria-label="t('rdp.scaleMode')">
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'fit' }" :title="t('rdp.scaleMode.fit')" :aria-label="t('rdp.scaleMode.fit')" :aria-pressed="rdpScaleMode === 'fit'" @click="rdpScaleMode = 'fit'"><Scan /></button>
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'actual' }" :title="t('rdp.scaleMode.actual')" :aria-label="t('rdp.scaleMode.actual')" :aria-pressed="rdpScaleMode === 'actual'" @click="rdpScaleMode = 'actual'"><span class="font-step-label" aria-hidden="true">1:1</span></button>
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'stretch' }" :title="t('rdp.scaleMode.stretch')" :aria-label="t('rdp.scaleMode.stretch')" :aria-pressed="rdpScaleMode === 'stretch'" @click="rdpScaleMode = 'stretch'"><StretchHorizontal /></button>
  </div>
  <div>
    <Popover :open="rdpResolutionOpen" @update:open="(open) => { if (!open) rdpResolutionOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-cyan" :class="{ 'is-active': rdpResolutionOpen }" :title="t('rdp.resolution.title')" :aria-label="t('rdp.resolution.title')" :aria-expanded="rdpResolutionOpen" @pointerdown.stop @click.stop="toggleRdpResolutionMenu"><Proportions /></button>
      </PopoverAnchor>
      <PopoverContent class="popover rdp-resolution-popover" align="start" :side-offset="5">
        <h3>{{ t("rdp.resolution.title") }}</h3>
        <div class="rdp-resolution-presets">
          <button v-for="preset in RDP_RESOLUTION_PRESETS" :key="`${preset.width}x${preset.height}`" class="rdp-resolution-item" :class="{ 'is-active': isRdpResolutionActive(preset.width, preset.height) }" :disabled="rdpResizing" @click="applyRdpResolution(preset.width, preset.height)">
            {{ rdpResolutionLabel(preset.width, preset.height) }}
          </button>
          <button class="rdp-resolution-item" :disabled="rdpResizing" @click="applyRdpResolution(rdpWindowProportionPreset().width, rdpWindowProportionPreset().height)">{{ t("rdp.resolution.window") }}</button>
        </div>
        <div class="rdp-resolution-custom">
          <input v-model="rdpCustomWidth" class="mono" type="number" min="640" max="3840" :placeholder="t('rdp.width')" :aria-label="t('rdp.width')" />
          <span aria-hidden="true">×</span>
          <input v-model="rdpCustomHeight" class="mono" type="number" min="480" max="2160" :placeholder="t('rdp.height')" :aria-label="t('rdp.height')" />
          <button class="primary-button" :disabled="rdpResizing || !rdpCustomWidth || !rdpCustomHeight" @click="applyRdpCustomResolution">{{ t("rdp.resolution.apply") }}</button>
        </div>
        <p v-if="rdpDesktopSize" class="muted rdp-paste-hint">{{ t("rdp.resolution.current", { width: rdpDesktopSize.width, height: rdpDesktopSize.height }) }}</p>
        <p class="muted rdp-paste-hint">{{ t("rdp.resolution.note") }}</p>
      </PopoverContent>
    </Popover>
  </div>
  <div>
    <Popover :open="rdpPasteOpen" @update:open="(open) => { if (!open) rdpPasteOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-cyan" :class="{ 'is-active': rdpPasteOpen }" :title="t('rdp.paste.title')" :aria-label="t('rdp.paste.title')" :aria-expanded="rdpPasteOpen" @pointerdown.stop @click.stop="toggleRdpPasteMenu"><ClipboardPaste /></button>
      </PopoverAnchor>
      <PopoverContent class="popover rdp-paste-popover" align="start" :side-offset="5">
        <h3>{{ t("rdp.paste.title") }}</h3>
        <textarea v-model="rdpPasteText" class="rdp-paste-text mono" :placeholder="t('rdp.paste.placeholder')" rows="8" spellcheck="false" />
        <div class="rdp-paste-actions">
          <button class="primary-button" :disabled="!rdpPasteText" @click="rdpWriteRemoteClipboard">{{ t("rdp.paste.toClipboard") }}</button>
          <button class="link-button" :disabled="!rdpPasteText || rdpTypingBusy" @click="rdpTypeAsKeystrokes">{{ t("rdp.paste.type") }}</button>
        </div>
        <p class="muted rdp-paste-hint">{{ t("rdp.paste.toClipboardHint") }}</p>
        <p v-if="rdpTypingTotal > 0" class="muted rdp-paste-hint">{{ rdpTypingBusy ? t("rdp.paste.typing", { sent: rdpTypingSent, total: rdpTypingTotal }) : t("rdp.paste.typeDone", { count: rdpTypingSent }) }}</p>
      </PopoverContent>
    </Popover>
  </div>
  <button class="icon-button icon-emerald" :title="t('rdp.disconnect')" :aria-label="t('rdp.disconnect')" @click="closeRdpSession"><PlugZap /></button>
</template>

<style scoped>
/* 字号步进标签（工具栏 A+/A- 与 RDP 1:1 复用；原 App.vue scoped 样式随 DOM 迁入）。 */
.font-step-label { font-size: 11px; font-weight: 600; line-height: 1; letter-spacing: 0; }
/* RDP 缩放三档按钮组（isRdpMode 工具组）：与 icon-button 同密度，激活态
   走 primary 描边与 icon-emerald 同款强调。 */
.rdp-scale-group { display: flex; align-items: center; gap: 1px; }
.rdp-scale-button.is-active { color: var(--primary); background: color-mix(in srgb, var(--primary) 14%, transparent); }
.rdp-scale-button .font-step-label { font-size: 10px; letter-spacing: .02em; }
/* RDP 大文本粘贴面板（工具栏下拉）：textarea 主输入 + 双通道投递动作。 */
.rdp-paste-popover h3 { margin: 2px 4px 8px; font-size: 12px; }
.rdp-paste-text {
  width: 100%;
  min-height: 140px;
  max-height: 260px;
  resize: vertical;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--background);
  color: var(--foreground);
  padding: 6px 8px;
  font-size: 11px;
  line-height: 1.5;
  outline: none;
}
.rdp-paste-text:focus { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
.rdp-paste-actions { display: flex; align-items: center; gap: 10px; margin-top: 8px; }
.rdp-paste-hint { margin: 6px 4px 0; font-size: 11px; }
/* RDP 分辨率弹层（工具栏下拉）：预设网格 + 自定义宽高行。 */
.rdp-resolution-popover h3 { margin: 2px 4px 8px; font-size: 12px; }
.rdp-resolution-presets { display: flex; flex-wrap: wrap; gap: 6px; }
.rdp-resolution-item {
  border: 1px solid var(--border);
  border-radius: 6px;
  background: transparent;
  color: var(--foreground);
  font-size: 11px;
  padding: 4px 8px;
  cursor: pointer;
}
.rdp-resolution-item:hover { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.rdp-resolution-item.is-active { color: var(--primary); border-color: var(--primary); background: color-mix(in srgb, var(--primary) 12%, transparent); }
.rdp-resolution-custom { display: flex; align-items: center; gap: 6px; margin-top: 8px; }
.rdp-resolution-custom input {
  width: 72px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--background);
  color: var(--foreground);
  font-size: 11px;
  padding: 4px 6px;
}
.rdp-resolution-custom input:focus { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
</style>
