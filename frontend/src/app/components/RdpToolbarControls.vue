<script setup lang="ts">
import { computed } from "vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import type { RdpToolbarActions } from "../lib/workbenchToolbarActions";

/** RDP 独立工具组（T19 自 App.vue 模板迁入）：缩放三档 + 桌面分辨率 +
 *  大文本粘贴 + 文件传输提示 + 断开。与 SSH 组互斥，由父级按 isRdpMode
 *  门控挂载；分辨率/粘贴状态与 rdp/resize、rdp/input 调用仍由 App 的
 *  useRdpWorkbench 持有。文件传输入口只弹剪贴板提示：RDP 的 CLIPRDR
 *  文件通道直接传文件，双栏 SFTP 窗口留作 VNC 入口。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  /** `useRdpWorkbench` 的预设表（静态常量，无需响应式）。 */
  resolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  /** 宿主是否 macOS：文件管理器名称与修饰键随平台不同。 */
  applePlatform: boolean;
  /** sidecar 是否能读写宿主系统文件剪贴板（macOS/Windows/Linux helper）。 */
  nativeFileClipboard: boolean;
  /** 远端服务器是否接受 CLIPRDR 文件流（Windows 服务器；GNOME 等仅文本）。 */
  serverFileClipboard: boolean;
  /** 远端文件能力是否已知（CLIPRDR Ready/unavailable 之后）。 */
  serverFileClipboardKnown: boolean;
  /** 连接表单的剪贴板共享开关；关闭时 RDP 文件通道完全不可用。 */
  clipboardEnabled: boolean;
  rdpResizing: boolean;
  rdpAutoFit: boolean;
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
/** 文件传输提示弹层：受控 open 由 App 持有，经 closeToolbarPopovers 统一收口。 */
const rdpFileHintOpen = defineModel<boolean>("rdpFileHintOpen", { required: true });

const t = props.t;
const RDP_RESOLUTION_PRESETS = props.resolutionPresets;
// macOS 始终有原生文件剪贴板；其余平台以 sidecar 的能力探测为准（Linux 需要
// xclip 或 wl-clipboard）。没有原生能力时回落到桌面浮层的下载/发送按钮。
const nativeFiles = computed(() => props.applePlatform || props.nativeFileClipboard);
// GNOME Remote Desktop 等 Linux RDP 服务端只支持文本剪贴板；此时 Ctrl+C/V
// 传文件没有通道，改为引导到 SSH/SFTP 双栏窗口。连接关闭剪贴板共享时同理。
const fileTransferBlocked = computed(
  () =>
    !props.clipboardEnabled ||
    (props.serverFileClipboardKnown && !props.serverFileClipboard),
);
const blockedHint = computed(() =>
  props.clipboardEnabled
    ? "rdp.clipboardFiles.serverUnsupported"
    : "rdp.clipboardFiles.disabled",
);
const remoteToHostHint = computed(() =>
  !nativeFiles.value
    ? "rdp.clipboardFiles.hintRemoteToHostOther"
    : props.applePlatform
      ? "rdp.clipboardFiles.hintRemoteToHost"
      : "rdp.clipboardFiles.hintRemoteToHostNative",
);
const hostToRemoteHint = computed(() =>
  !nativeFiles.value
    ? "rdp.clipboardFiles.hintHostToRemoteOther"
    : props.applePlatform
      ? "rdp.clipboardFiles.hintHostToRemote"
      : "rdp.clipboardFiles.hintHostToRemoteNative",
);
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  toggleRdpResolutionMenu,
  applyRdpResolution,
  rdpResolutionLabel,
  isRdpResolutionActive,
  enableRdpAutoFit,
  applyRdpCustomResolution,
  toggleRdpPasteMenu,
  rdpWriteRemoteClipboard,
  rdpTypeAsKeystrokes,
  toggleRdpFileHintMenu,
  openRemoteFileTransfer,
  closeRdpSession,
} = props.actions;
</script>

<template>
  <div class="rdp-scale-group" role="group" :aria-label="t('rdp.scaleMode')">
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'fit' }" :title="t('rdp.scaleMode.fit')" :aria-label="t('rdp.scaleMode.fit')" :aria-pressed="rdpScaleMode === 'fit'" @click="rdpScaleMode = 'fit'"><WorkbenchGlyph name="rdp-fit" /></button>
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'actual' }" :title="t('rdp.scaleMode.actual')" :aria-label="t('rdp.scaleMode.actual')" :aria-pressed="rdpScaleMode === 'actual'" @click="rdpScaleMode = 'actual'"><span class="font-step-label" aria-hidden="true">1:1</span></button>
    <button class="icon-button rdp-scale-button" :class="{ 'is-active': rdpScaleMode === 'stretch' }" :title="t('rdp.scaleMode.stretch')" :aria-label="t('rdp.scaleMode.stretch')" :aria-pressed="rdpScaleMode === 'stretch'" @click="rdpScaleMode = 'stretch'"><WorkbenchGlyph name="rdp-stretch" /></button>
  </div>
  <div>
    <Popover :open="rdpResolutionOpen" @update:open="(open) => { if (!open) rdpResolutionOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-cyan rdp-resolution-trigger" :class="{ 'is-active': rdpResolutionOpen }" :title="t('rdp.resolution.title')" :aria-label="t('rdp.resolution.title')" :aria-expanded="rdpResolutionOpen" aria-haspopup="dialog" @pointerdown.stop @click.stop="toggleRdpResolutionMenu"><WorkbenchGlyph name="rdp-resolution" /></button>
      </PopoverAnchor>
      <PopoverContent class="popover rdp-resolution-popover" align="start" :side-offset="5">
        <h3>{{ t("rdp.resolution.title") }}</h3>
        <div class="rdp-resolution-presets">
          <button class="rdp-resolution-item rdp-resolution-auto" :class="{ 'is-active': rdpAutoFit && rdpScaleMode === 'fit' }" :aria-pressed="rdpAutoFit && rdpScaleMode === 'fit'" :disabled="rdpResizing" @click="enableRdpAutoFit">{{ t("rdp.resolution.window") }}</button>
          <button v-for="preset in RDP_RESOLUTION_PRESETS" :key="`${preset.width}x${preset.height}`" class="rdp-resolution-item" :class="{ 'is-active': isRdpResolutionActive(preset.width, preset.height) }" :aria-pressed="isRdpResolutionActive(preset.width, preset.height)" :disabled="rdpResizing" @click="applyRdpResolution(preset.width, preset.height)">
            {{ rdpResolutionLabel(preset.width, preset.height) }}
          </button>
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
        <button class="icon-button icon-cyan" :class="{ 'is-active': rdpPasteOpen }" :title="t('rdp.paste.title')" :aria-label="t('rdp.paste.title')" :aria-expanded="rdpPasteOpen" @pointerdown.stop @click.stop="toggleRdpPasteMenu"><WorkbenchGlyph name="paste" /></button>
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
  <div>
    <Popover :open="rdpFileHintOpen" @update:open="(open) => { if (!open) rdpFileHintOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-cyan rdp-file-hint-trigger" :class="{ 'is-active': rdpFileHintOpen }" :title="t('rdp.clipboardFiles.hintTitle')" :aria-label="t('rdp.clipboardFiles.hintTitle')" :aria-expanded="rdpFileHintOpen" aria-haspopup="dialog" @pointerdown.stop @click.stop="toggleRdpFileHintMenu"><WorkbenchGlyph name="transfers" /></button>
      </PopoverAnchor>
      <PopoverContent class="popover rdp-file-hint-popover" align="start" :side-offset="5">
        <h3>{{ t("rdp.clipboardFiles.hintTitle") }}</h3>
        <template v-if="fileTransferBlocked">
          <p class="muted rdp-file-hint-intro">{{ t(blockedHint) }}</p>
          <button class="primary-button rdp-file-hint-ssh" @click="rdpFileHintOpen = false; openRemoteFileTransfer()">{{ t("rdp.clipboardFiles.openSshTransfer") }}</button>
        </template>
        <template v-else>
          <p class="muted rdp-file-hint-intro">{{ t("rdp.clipboardFiles.hintIntro") }}</p>
          <p class="rdp-file-hint-row">{{ t(remoteToHostHint) }}</p>
          <p class="rdp-file-hint-row">{{ t(hostToRemoteHint) }}</p>
          <p v-if="nativeFiles" class="muted rdp-file-hint-note">{{ t("rdp.clipboardFiles.hintDrag") }}</p>
        </template>
      </PopoverContent>
    </Popover>
  </div>
  <button class="icon-button icon-emerald" :title="t('rdp.disconnect')" :aria-label="t('rdp.disconnect')" @click="closeRdpSession"><WorkbenchGlyph name="power" /></button>
</template>

<style scoped>
/* 字号步进标签（工具栏 A+/A- 与 RDP 1:1 复用；原 App.vue scoped 样式随 DOM 迁入）。 */
.font-step-label { font-size: 11px; font-weight: 600; line-height: 1; letter-spacing: 0; }
/* RDP 缩放三档按钮组（isRdpMode 工具组）：与 icon-button 同密度，激活态
   走 primary 描边与 icon-emerald 同款强调。 */
.rdp-scale-group { display: flex; align-items: center; gap: 1px; }
.rdp-scale-button.is-active, .rdp-resolution-trigger.is-active { color: var(--primary); background: color-mix(in srgb, var(--primary) 14%, transparent); box-shadow: none; }
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
.rdp-resolution-item:hover:not(:disabled) { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.rdp-resolution-item.is-active { color: var(--primary); border-color: var(--primary); background: color-mix(in srgb, var(--primary) 12%, transparent); }
.rdp-resolution-auto { display: block; width: 100%; margin-bottom: 6px; text-align: left; }
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
/* RDP 文件传输提示弹层（工具栏下拉）：说明纯剪贴板通道，不打开 SFTP 窗口。
   宽度沿用 PopoverContent 的 w-72 壳尺寸（teleport 后父级 scoped 宽度规则不生效）。 */
.rdp-file-hint-popover h3 { margin: 2px 4px 8px; font-size: 12px; }
.rdp-file-hint-intro { margin: 0 4px 8px; font-size: 11px; line-height: 1.5; }
.rdp-file-hint-row { margin: 0 4px 6px; font-size: 11px; line-height: 1.5; }
.rdp-file-hint-note { margin: 8px 4px 0; font-size: 11px; line-height: 1.5; }
.rdp-file-hint-ssh { margin-top: 8px; align-self: flex-start; }
</style>
