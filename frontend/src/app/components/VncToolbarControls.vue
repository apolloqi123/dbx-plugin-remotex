<script setup lang="ts">
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import type { VncToolbarActions } from "../lib/workbenchToolbarActions";

/** VNC 独立工具组（与 RdpToolbarControls 同构）：缩放三档 + 桌面分辨率 +
 *  断开。SSH 字号/分屏/命令类入口在 VNC 会话激活时整组隐藏。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  resolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  vncResizing: boolean;
  vncCanResize: boolean;
  vncAutoFit: boolean;
  vncDesktopSize: { width: number; height: number } | null;
  actions: VncToolbarActions;
}>();

const vncScaleMode = defineModel<"fit" | "stretch" | "actual">("vncScaleMode", { required: true });
const vncResolutionOpen = defineModel<boolean>("vncResolutionOpen", { required: true });
const vncCustomWidth = defineModel<string>("vncCustomWidth", { required: true });
const vncCustomHeight = defineModel<string>("vncCustomHeight", { required: true });

const t = props.t;
const VNC_RESOLUTION_PRESETS = props.resolutionPresets;
const {
  toggleVncResolutionMenu,
  applyVncResolution,
  vncResolutionLabel,
  isVncResolutionActive,
  enableVncAutoFit,
  applyVncCustomResolution,
  closeVncSession,
  openRemoteFileTransfer,
} = props.actions;
</script>

<template>
  <div class="vnc-scale-group" role="group" :aria-label="t('vnc.scaleMode')">
    <button class="icon-button vnc-scale-button" :class="{ 'is-active': vncScaleMode === 'fit' }" :title="t('vnc.scaleMode.fit')" :aria-label="t('vnc.scaleMode.fit')" :aria-pressed="vncScaleMode === 'fit'" @click="vncScaleMode = 'fit'"><WorkbenchGlyph name="rdp-fit" /></button>
    <button class="icon-button vnc-scale-button" :class="{ 'is-active': vncScaleMode === 'actual' }" :title="t('vnc.scaleMode.actual')" :aria-label="t('vnc.scaleMode.actual')" :aria-pressed="vncScaleMode === 'actual'" @click="vncScaleMode = 'actual'"><span class="font-step-label" aria-hidden="true">1:1</span></button>
    <button class="icon-button vnc-scale-button" :class="{ 'is-active': vncScaleMode === 'stretch' }" :title="t('vnc.scaleMode.stretch')" :aria-label="t('vnc.scaleMode.stretch')" :aria-pressed="vncScaleMode === 'stretch'" @click="vncScaleMode = 'stretch'"><WorkbenchGlyph name="rdp-stretch" /></button>
  </div>
  <div>
    <Popover :open="vncResolutionOpen" @update:open="(open) => { if (!open) vncResolutionOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-cyan vnc-resolution-trigger" :class="{ 'is-active': vncResolutionOpen }" :title="t('vnc.resolution.title')" :aria-label="t('vnc.resolution.title')" :aria-expanded="vncResolutionOpen" aria-haspopup="dialog" @pointerdown.stop @click.stop="toggleVncResolutionMenu"><WorkbenchGlyph name="rdp-resolution" /></button>
      </PopoverAnchor>
      <PopoverContent class="popover vnc-resolution-popover" align="start" :side-offset="5">
        <h3>{{ t("vnc.resolution.title") }}</h3>
        <button class="vnc-resolution-item vnc-resolution-auto" :class="{ 'is-active': vncAutoFit && vncScaleMode === 'fit' }" :aria-pressed="vncAutoFit && vncScaleMode === 'fit'" :disabled="!vncCanResize || vncResizing" @click="enableVncAutoFit">{{ t("vnc.resolution.window") }}</button>
        <div class="vnc-resolution-presets">
          <button v-for="preset in VNC_RESOLUTION_PRESETS" :key="`${preset.width}x${preset.height}`" class="vnc-resolution-item" :class="{ 'is-active': isVncResolutionActive(preset.width, preset.height) }" :aria-pressed="isVncResolutionActive(preset.width, preset.height)" :disabled="!vncCanResize || vncResizing" @click="applyVncResolution(preset.width, preset.height)">
            {{ vncResolutionLabel(preset.width, preset.height) }}
          </button>
        </div>
        <div class="vnc-resolution-custom">
          <input v-model="vncCustomWidth" class="mono" type="number" min="640" max="3840" :placeholder="t('vnc.width')" :aria-label="t('vnc.width')" />
          <span aria-hidden="true">×</span>
          <input v-model="vncCustomHeight" class="mono" type="number" min="480" max="2160" :placeholder="t('vnc.height')" :aria-label="t('vnc.height')" />
          <button class="primary-button" :disabled="!vncCanResize || vncResizing || !vncCustomWidth || !vncCustomHeight" @click="applyVncCustomResolution">{{ t("vnc.resolution.apply") }}</button>
        </div>
        <p v-if="vncDesktopSize" class="muted vnc-resolution-hint">{{ t("vnc.resolution.current", { width: vncDesktopSize.width, height: vncDesktopSize.height }) }}</p>
        <p class="muted vnc-resolution-hint">{{ t("vnc.resolution.note") }}</p>
      </PopoverContent>
    </Popover>
  </div>
  <button class="icon-button icon-cyan" :title="t('remoteFiles.open')" :aria-label="t('remoteFiles.open')" @click="openRemoteFileTransfer"><WorkbenchGlyph name="transfers" /></button>
  <button class="icon-button icon-emerald" :title="t('vnc.disconnect')" :aria-label="t('vnc.disconnect')" @click="closeVncSession"><WorkbenchGlyph name="power" /></button>
</template>

<style scoped>
/* 字号步进标签（工具栏 A+/A− 与 VNC 1:1 复用）。 */
.font-step-label { font-size: 11px; font-weight: 600; line-height: 1; letter-spacing: 0; }
/* VNC 缩放三档按钮组：与 RDP 组同密度、同激活态。 */
.vnc-scale-group { display: flex; align-items: center; gap: 1px; }
.vnc-scale-button.is-active, .vnc-resolution-trigger.is-active { color: var(--primary); background: color-mix(in srgb, var(--primary) 14%, transparent); box-shadow: none; }
.vnc-scale-button .font-step-label { font-size: 10px; letter-spacing: .02em; }
/* VNC 分辨率弹层：预设网格 + 自定义宽高行。 */
.vnc-resolution-popover h3 { margin: 2px 4px 8px; font-size: 12px; }
.vnc-resolution-presets { display: flex; flex-wrap: wrap; gap: 6px; }
.vnc-resolution-item {
  border: 1px solid var(--border);
  border-radius: 6px;
  background: transparent;
  color: var(--foreground);
  font-size: 11px;
  padding: 4px 8px;
  cursor: pointer;
}
.vnc-resolution-item:hover:not(:disabled) { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.vnc-resolution-item.is-active { color: var(--primary); border-color: var(--primary); background: color-mix(in srgb, var(--primary) 12%, transparent); }
.vnc-resolution-auto { display: block; width: 100%; margin-bottom: 6px; text-align: left; }
.vnc-resolution-custom { display: flex; align-items: center; gap: 6px; margin-top: 8px; }
.vnc-resolution-custom input {
  width: 72px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--background);
  color: var(--foreground);
  padding: 4px 6px;
  font-size: 11px;
}
.vnc-resolution-custom input:focus-visible {
  border-color: color-mix(in srgb, var(--primary) 70%, var(--border));
  outline: 2px solid color-mix(in srgb, var(--primary) 35%, transparent);
  outline-offset: 1px;
}
.vnc-resolution-hint { margin: 6px 4px 0; font-size: 11px; }
</style>