<script setup lang="ts">
import { computed, ref } from "vue";
import { FileUp, Info, Loader2, TriangleAlert } from "@lucide/vue";
import TerminalGutter from "../../features/terminal/components/TerminalGutter.vue";
import { transferBusyPaneClass } from "../../features/terminal/lib/behavior/terminalInteraction";
import type { GutterRow } from "../../features/terminal/lib/appearance/terminalGutter";

/** 终端舞台（T20）：`.terminal-pane` 外壳 + xterm 宿主 + 壁纸/行号 gutter +
 *  诊断层/投放遮罩/AI 修复条/命令标记/AI 执行横幅，并为 App 仍持有的浮层
 *  （VNC/RDP 画布、光标锚点浮层、搜索面板、状态层、连接卡、传输层、录制
 *  列表）提供插槽出口，保持原 DOM 顺序。
 *
 *  宿主 ref 经 defineExpose 回传给 App 的 terminalHost（不创建第二个宿主）：
 *  App 用 useTemplateRef 读取后在 computed 里桥接给 xterm 实例等 composable。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  terminalDragActive: boolean;
  commandMarker: {
    installed: boolean;
    active: boolean;
    command: string;
    exitCode: number | null;
    durationMs: number | null;
    cwd: string;
  };
  gutterPaneVisible: boolean;
  wallpaperActive: boolean;
  terminalTransferBusy: boolean;
  gutterPaneStyle: Record<string, string>;
  wallpaperDataUrl: string;
  wallpaperOpacity: number;
  gutterVisible: boolean;
  gutterRows: GutterRow[];
  gutterWidth: number;
  terminalBellFlash: boolean;
  actionLinkHint: { x: number; y: number; text: string } | null;
  terminalDiagVisible: boolean;
  terminalDiag: { keys: number; sends: number; acks: number; errors: number; swallowed: number };
  aiFixBar: { command: string; exitCode: number } | null;
  aiFixBarStyle: Record<string, string> | undefined;
  isRdpConnection: boolean;
  localUiMode: boolean;
  dragActive: boolean;
  sftpViewVisible: boolean;
  commandMarkerDetails: string;
  commandMarkerElapsed: number | null;
  agentRunning: { command: string } | null | undefined;
  formatCommandDuration: (ms: number | null | undefined) => string;
}>();

const emit = defineEmits<{
  (e: "middle-click", event: MouseEvent): void;
  (e: "dismiss-diagnostics"): void;
  (e: "fix-with-ai"): void;
  (e: "dismiss-fix"): void;
  (e: "focus-terminal"): void;
  (e: "interrupt-agent"): void;
}>();

const { t } = props;

const terminalHost = ref<HTMLElement>();
defineExpose({ terminalHost });

const paneClass = computed(() => ({
  "drag-active": props.terminalDragActive,
  "marker-visible": props.commandMarker.installed,
  "gutter-visible": props.gutterPaneVisible,
  "wallpaper-active": props.wallpaperActive,
  // AI 修复条贴在终端顶部时，搜索面板/指标-录制浮卡也钉在同一处；用状态类
  // 把浮层整体下移一条，避免两个浮层互相盖住（输入框/标题被截断）。
  "ai-fix-visible": Boolean(props.aiFixBar),
  ...transferBusyPaneClass(props.terminalTransferBusy),
}));
</script>

<template>
  <section class="terminal-pane" :class="paneClass" :style="gutterPaneStyle">
    <!-- P2-9 背景图层：pointer-events:none 垫底（DOM 序先于 terminal-host），
         透明度 0.1-0.9 由设置页滑杆控制；开启期间强制 DOM 渲染器透出本层。 -->
    <div v-if="wallpaperActive" class="terminal-wallpaper" :style="{ backgroundImage: `url(${wallpaperDataUrl})`, opacity: wallpaperOpacity / 100 }" aria-hidden="true" />
    <!-- P1-3 行号/时间戳 gutter：绝对定位覆盖左缘 padding 环带（z-index 1，
         低于浮层 z-index 2），xterm 左 padding 随 --dbx-gutter-width 加宽，
         不遮文本；drop-overlay/搜索面板/诊断浮层定位不受影响。 -->
    <TerminalGutter v-if="gutterVisible" :rows="gutterRows" :width="gutterWidth" />
    <div ref="terminalHost" class="terminal-host" :class="{ 'bell-flash': terminalBellFlash }" @mousedown.middle="emit('middle-click', $event)" />
    <!-- VNC/RDP 画布（App 仍持有 surface ref 与输入接线）。 -->
    <slot name="surfaces" />
    <!-- P1-2 动作链接命令预览浮签：悬停 / Alt+点击时显示建议命令文本。 -->
    <div v-if="actionLinkHint" class="action-link-hint mono" :style="{ left: `${actionLinkHint.x}px`, top: `${actionLinkHint.y}px` }">{{ actionLinkHint.text }}</div>
    <!-- #33/#71 快速输入丢失诊断浮层：Ctrl/Cmd+Shift+D 切换。keys=onData
         路由到 PTY 的按键、sends=提交宿主桥的帧、acks=sidecar 确认的帧、
         errors=桥拒绝、swallowed=传输路由吞键。三者对不上即锁定丢失层。 -->
    <div v-if="terminalDiagVisible" class="terminal-diag-overlay" @dblclick="emit('dismiss-diagnostics')">
      keys {{ terminalDiag.keys }} · sends {{ terminalDiag.sends }} · acks {{ terminalDiag.acks }} · errors {{ terminalDiag.errors }} · swallowed {{ terminalDiag.swallowed }}
    </div>
    <!-- 光标锚点浮层（提示引导条/补全/ghost/AI 搜索）与搜索面板等仍由 App 持有。 -->
    <slot name="cursor" />
    <!-- 失败命令修复条（Warp Fix-with-AI 同位）：非弹窗不抢焦点，贴触发光标
         行顶；新命令开始/Esc 关闭/会话切换即消失。「用 AI 修复」首次发送过
         快照预览确认，输出已截断+脱敏（prepareAiOutputSnapshot）。 -->
    <div v-if="aiFixBar" class="terminal-ai-fix mono" :style="aiFixBarStyle" role="status">
      <span class="terminal-ai-fix-exit">✗ {{ aiFixBar.exitCode }}</span>
      <span class="terminal-ai-fix-command">{{ aiFixBar.command }}</span>
      <button type="button" class="terminal-ai-fix-btn" @click="emit('fix-with-ai')">{{ t("aiFix.barAction") }}</button>
      <button type="button" class="terminal-ai-fix-close" :title="t('aiFix.barClose')" :aria-label="t('aiFix.barClose')" @click="emit('dismiss-fix')">✕</button>
    </div>
    <div v-if="!isRdpConnection && !localUiMode && (terminalDragActive || (dragActive && !sftpViewVisible))" class="drop-overlay"><FileUp /><strong>{{ t("terminalDrop.hint") }}</strong></div>
    <slot name="panels" />
    <!-- 连接/本地/协议状态层与传输/回放层：组件由 App 装配，插槽仅保序。 -->
    <slot name="status" />
    <slot name="connection" />
    <div v-if="commandMarker.installed" class="terminal-command-marker" :class="{ active: commandMarker.active, failed: !commandMarker.active && commandMarker.exitCode !== null && commandMarker.exitCode !== 0 }" :title="commandMarkerDetails" @click="emit('focus-terminal')">
      <Loader2 v-if="commandMarker.active" class="spinning" />
      <TriangleAlert v-else-if="commandMarker.exitCode" />
      <Info v-else />
      <span v-if="commandMarker.active" class="marker-text">{{ t("terminalCommand.running", { command: commandMarker.command || "…" }) }}</span>
      <span v-if="commandMarker.active && commandMarkerElapsed !== null" class="marker-elapsed mono">{{ formatCommandDuration(commandMarkerElapsed) }}</span>
      <span v-else-if="commandMarker.exitCode !== null" class="marker-text">{{ t("terminalCommand.finished", { code: commandMarker.exitCode, duration: formatCommandDuration(commandMarker.durationMs) }) }}</span>
      <span v-else class="marker-text">{{ t("terminalCommand.hint") }}</span>
      <span v-if="commandMarker.cwd" class="marker-cwd mono">{{ commandMarker.cwd }}</span>
    </div>
    <div v-if="agentRunning" class="agent-run-banner" role="status">
      <Loader2 class="spinning" />
      <span class="agent-run-text">{{ t("agentRunningBanner") }}</span>
      <code class="agent-run-command mono" :title="agentRunning.command">{{ agentRunning.command }}</code>
      <button class="agent-interrupt" @click="emit('interrupt-agent')">{{ t("agentInterrupt") }}</button>
    </div>
    <slot name="transfer" />
    <slot name="recordings" />
  </section>
</template>

<style scoped>
/* —— P1-3 行号/时间戳 gutter 的布局联动 ——
   gutter 组件自身样式在 TerminalGutter.vue；这里只做两件事：
   1) gutter 可见时把 xterm 左 padding 加宽 --dbx-gutter-width（FitAddon 读
      element padding 算列数，列宽随之自动收窄，文本不会滑进 gutter 环带）；
   2) 命令预览浮签（悬停 / Alt+点击动作链接时出现），z-index 高于终端宿主
      （z 0）与 gutter（z 1），低于浮层梯队（z 2+）。 */
.terminal-pane.gutter-visible .terminal-host :deep(.xterm) {
  padding-left: calc(var(--dbx-gutter-width, 0px) + var(--ssh-terminal-padding-left, 10px));
}
/* —— P2-9 背景图（对标 NyaTerm，MVP 简化）——
   图层垫底（DOM 序先于 terminal-host，pointer-events 关）；开启期间 pane/
   宿主/xterm 表面底色透明化（color-mix 保留一层底色防纯黑/纯白刺眼），
   !important 压过 xterm 6.x 内联在 .xterm-scrollable-element 的主题背景。
   开启期间渲染器强制回退 DOM（rendererWebglEffective），否则 WebGL 画布
   不透明会盖死本层。 */
.terminal-wallpaper {
  position: absolute;
  z-index: 0;
  inset: 0;
  pointer-events: none;
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;
}
.terminal-pane.wallpaper-active { background: color-mix(in srgb, var(--ssh-terminal-background) 55%, transparent); }
.terminal-pane.wallpaper-active .terminal-host { background: transparent; }
.terminal-pane.wallpaper-active .terminal-host :deep(.xterm),
.terminal-pane.wallpaper-active .terminal-host :deep(.xterm .xterm-viewport),
.terminal-pane.wallpaper-active .terminal-host :deep(.xterm .xterm-scrollable-element),
.terminal-pane.wallpaper-active .terminal-host :deep(.xterm .xterm-screen),
.terminal-pane.wallpaper-active .terminal-host :deep(.xterm .xterm-rows) { background: transparent !important; }
.action-link-hint {
  position: absolute;
  z-index: 3;
  max-width: 300px;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px 8px;
  background: var(--popover);
  color: var(--foreground);
  font-size: 11px;
  white-space: pre;
  text-overflow: ellipsis;
  box-shadow: var(--shadow-sm, 0 1px 3px rgb(0 0 0 / 0.25));
  pointer-events: none;
}
/* 失败命令修复条（Warp Fix-with-AI 同位）：单行非弹窗，贴触发光标行顶。 */
.terminal-ai-fix {
  position: absolute;
  right: 8px;
  left: 8px;
  z-index: 8;
  display: flex;
  align-items: center;
  gap: 8px;
  overflow: hidden;
  border: 1px solid var(--border, rgb(128 128 132 / 28%));
  border-radius: var(--radius);
  background: var(--popover);
  box-shadow: var(--shadow-popover);
  padding: 4px 8px;
  font-size: 11.5px;
}
.terminal-ai-fix-exit {
  flex: 0 0 auto;
  border: 1px solid color-mix(in srgb, var(--destructive) 38%, transparent);
  border-radius: 999px;
  background: color-mix(in srgb, var(--destructive) 14%, transparent);
  color: var(--destructive);
  padding: 0 7px;
  font-size: var(--text-2xs);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  line-height: 16px;
}
.terminal-ai-fix-command {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  color: var(--popover-foreground);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.terminal-ai-fix-btn {
  flex: 0 0 auto;
  border: 0;
  border-radius: 4px;
  background: var(--accent, rgb(255 255 255 / 10%));
  color: var(--accent-foreground, inherit);
  cursor: pointer;
  padding: 2px 8px;
  font-size: 11px;
}
.terminal-ai-fix-btn:hover {
  filter: brightness(1.1);
}
.terminal-ai-fix-close {
  flex: 0 0 auto;
  border: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  padding: 0 2px;
}
.terminal-ai-fix-close:hover {
  color: var(--popover-foreground);
}
/* AI 终端同步执行：执行横幅（终端底部，避开命令标记条） */
.agent-run-banner { position: absolute; z-index: 3; right: 8px; bottom: 36px; left: 8px; display: flex; align-items: center; gap: 8px; border: 1px solid color-mix(in srgb, var(--primary) 40%, var(--border)); border-radius: var(--radius); padding: 6px 8px; background: color-mix(in srgb, var(--background) 92%, transparent); box-shadow: 0 4px 14px color-mix(in srgb, #000 18%, transparent); font-size: 11px; }
.agent-run-banner svg { width: 14px; height: 14px; flex: 0 0 14px; }
.agent-run-text { flex: 0 0 auto; color: var(--foreground); }
.agent-run-command { min-width: 0; flex: 1; overflow: hidden; color: var(--muted-foreground); text-overflow: ellipsis; white-space: nowrap; }
.agent-interrupt { flex: 0 0 auto; height: 24px; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--destructive); font-size: 11px; cursor: pointer; }
.agent-interrupt:hover { background: var(--accent); }
</style>
