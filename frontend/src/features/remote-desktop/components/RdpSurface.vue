<script setup lang="ts">
// RDP 画布表面（nyaterm-parity P3-4）：接收 App.vue 解码好的帧补丁做
// putImageData 增量绘制（rAF 合帧），fit/stretch/actual 三种前端缩放，
// 键鼠事件经扫描码映射后交给父组件发送（rdp/input），服务端光标形状
// （rdp/pointer）落到画布 CSS cursor。与 VncSurface 同构：同一 44 字节
// patch 头解码复用 vncFrame，状态覆盖层（connecting/reconnecting/closed）
// 由 App.vue 的 terminal-overlay 分支承担，本组件只负责活的桌面画面与
// 输入采集。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  buildRdpKeyDownEvent,
  buildRdpKeyUpEvent,
  coalesceRdpPatches,
  decodeRdpFramePatch,
  mapDomButtonToRdpButton,
  RDP_DRAW_MIN_INTERVAL_MS,
  type RdpFramePatch,
  type RdpInputEvent,
  type RdpPointerEvent,
} from "../lib/rdpFrame";
import { isLocalOnlyShortcut, isMacPlatform, isMetaCode, isPasteShortcut, metaMapsToControl } from "../lib/remoteKeyboard";
import { cursorFromPointer } from "../lib/remoteCursor";

interface Props {
  scaleMode: "fit" | "stretch" | "actual";
  /** Connection-form clipboard toggle: when off, Cmd/Ctrl+V is forwarded as a
   *  normal remote key instead of being intercepted for host sync. */
  clipboardEnabled?: boolean;
}
const props = withDefaults(defineProps<Props>(), { clipboardEnabled: true });
const emit = defineEmits<{
  input: [RdpInputEvent];
  "clipboard-out": [string];
  /** Ctrl/Cmd+V：父组件读取宿主剪贴板并同步到远端。 */
  "paste-request": [];
  /** 桌面尺寸随帧头变化时上报（P0-2 分辨率入口据此标记当前档位）。 */
  "desktop-size": [size: { width: number; height: number }];
  /** 画布 CSS 尺寸变化（侧栏开合/窗口缩放）：自动匹配窗口模式据此重协商分辨率。 */
  "container-resize": [size: { width: number; height: number }];
}>();

const isMac = typeof navigator !== "undefined" && isMacPlatform(navigator.userAgent, navigator.platform ?? "");
const CONTROL_KEY = { key: "Control", code: "ControlLeft" } as const;

const canvasRef = ref<HTMLCanvasElement | null>(null);
const holderRef = ref<HTMLDivElement | null>(null);
const desktopSize = ref<{ width: number; height: number } | null>(null);
const holderSize = ref<{ width: number; height: number }>({ width: 0, height: 0 });
const focused = ref(false);
const cursorStyle = ref("");
let resizeObserver: ResizeObserver | null = null;

onMounted(() => {
  const holder = holderRef.value;
  if (!holder || typeof ResizeObserver === "undefined") return;
  let lastWidth = 0;
  let lastHeight = 0;
  resizeObserver = new ResizeObserver((entries) => {
    const rect = entries[0]?.contentRect;
    if (!rect || rect.width <= 0 || rect.height <= 0) return;
    if (Math.abs(rect.width - lastWidth) < 1 && Math.abs(rect.height - lastHeight) < 1) return;
    lastWidth = rect.width;
    lastHeight = rect.height;
    holderSize.value = { width: rect.width, height: rect.height };
    emit("container-resize", { width: rect.width, height: rect.height });
  });
  resizeObserver.observe(holder);
});

const scaleClass = computed(() => `vnc-scale-${props.scaleMode}`);
/** Fit mode fills the pane in both directions (up- and downscaling); explicit
 * sizing keeps pointer mapping aligned with the visible bitmap. */
const fitCanvasStyle = computed(() => {
  if (props.scaleMode !== "fit" || !desktopSize.value) return undefined;
  const { width, height } = holderSize.value;
  if (width <= 0 || height <= 0) return undefined;
  const scale = Math.min(width / desktopSize.value.width, height / desktopSize.value.height);
  return {
    width: `${Math.max(1, Math.round(desktopSize.value.width * scale))}px`,
    height: `${Math.max(1, Math.round(desktopSize.value.height * scale))}px`,
  };
});

// rAF 合帧队列：同一帧内的多个补丁一次绘制，避免高频 update 撕裂。
// P0-1 卡顿优化（真机 22MB/8s 采样定位：burst 时多个 MB 级条带在同一 tick
// 逐个 createImageData+putImageData 拖死主线程）：
// 1. 绘制前 coalesceRdpPatches 合并——被更晚补丁完全覆盖的旧矩形与旧桌面
//    尺寸补丁直接丢弃（后画覆盖先画，最终像素不变）；
// 2. 批间隔 ≥ RDP_DRAW_MIN_INTERVAL_MS（≈30fps 上限）：间隔不足时推迟到下
//    一个 rAF，让 burst 继续在队列里合并（多付 ≤33ms 延迟换主线程喘息）；
// 3. ImageData 尺寸匹配时复用同一缓冲，不再每补丁分配（大补丁 4MB/次）。
let pendingPatches: RdpFramePatch[] = [];
let drawScheduled = false;
let lastSequence = -1;
let lastDrawAt = 0;
// 复用的 ImageData（与最近一次绘制补丁同尺寸才有效）。
let reusableImage: { width: number; height: number; image: ImageData } | null = null;

/** 帧管道计数器（P0-1 测量面）：绘制路径零日志开销，经 perfStats() 暴露。 */
const perf = {
  received: 0,
  drawn: 0,
  coalesced: 0,
  ticks: 0,
  decodeUs: 0,
  drawUs: 0,
  maxTickDrawUs: 0,
  maxTickPatches: 0,
};
let tickDrawUs = 0;

function perfStats() {
  return { ...perf };
}

function scheduleDraw() {
  if (drawScheduled) return;
  drawScheduled = true;
  requestAnimationFrame(() => {
    drawScheduled = false;
    // 30fps 节流：距上次绘制不足一个间隔时推迟本轮（补丁留队继续合并）。
    const now = performance.now();
    if (lastDrawAt && now - lastDrawAt < RDP_DRAW_MIN_INTERVAL_MS) {
      scheduleDraw();
      return;
    }
    lastDrawAt = now;
    const queued = coalesceRdpPatches(pendingPatches);
    perf.coalesced += pendingPatches.length - queued.length;
    pendingPatches = [];
    if (queued.length === 0) return;
    perf.ticks += 1;
    perf.maxTickPatches = Math.max(perf.maxTickPatches, queued.length);
    tickDrawUs = 0;
    for (const patch of queued) drawPatch(patch);
    perf.maxTickDrawUs = Math.max(perf.maxTickDrawUs, tickDrawUs);
  });
}

function drawPatch(patch: RdpFramePatch) {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const drawStart = performance.now();
  if (canvas.width !== patch.desktopWidth || canvas.height !== patch.desktopHeight) {
    canvas.width = patch.desktopWidth;
    canvas.height = patch.desktopHeight;
    desktopSize.value = { width: patch.desktopWidth, height: patch.desktopHeight };
    // 画布重设尺寸即清空 ImageData 缓存（尺寸绑定已失效）。
    reusableImage = null;
    emit("desktop-size", { ...desktopSize.value });
  }
  const context = canvas.getContext("2d");
  if (!context) return;
  if (!reusableImage || reusableImage.width !== patch.width || reusableImage.height !== patch.height) {
    reusableImage = { width: patch.width, height: patch.height, image: context.createImageData(patch.width, patch.height) };
  }
  const image = reusableImage.image;
  const rowBytes = patch.width * 4;
  if (patch.stride === rowBytes) {
    image.data.set(patch.payload.subarray(0, rowBytes * patch.height));
  } else {
    for (let row = 0; row < patch.height; row += 1) {
      const sourceStart = row * patch.stride;
      image.data.set(patch.payload.subarray(sourceStart, sourceStart + rowBytes), row * rowBytes);
    }
  }
  context.putImageData(image, patch.x, patch.y);
  perf.drawn += 1;
  const drawUs = performance.now() - drawStart;
  perf.drawUs += drawUs;
  tickDrawUs += drawUs;
}

/** App.vue 帧通道入口：原始二进制帧在这里解码、乱序丢弃后入绘制队列。 */
function acceptFrame(frame: Uint8Array): boolean {
  const decodeStart = performance.now();
  let patch: RdpFramePatch;
  try {
    patch = decodeRdpFramePatch(frame);
  } catch {
    // 非法帧直接丢弃（sidecar 侧同样的校验不该让坏帧走到这里）。
    return false;
  }
  perf.decodeUs += performance.now() - decodeStart;
  if (patch.sequence <= lastSequence) return false;
  lastSequence = patch.sequence;
  perf.received += 1;
  pendingPatches.push(patch);
  scheduleDraw();
  return true;
}

/** 重连整幅重绘/新会话：清空本地绘制状态，等下一帧。 */
function reset() {
  pendingPatches = [];
  lastSequence = -1;
  lastDrawAt = 0;
  reusableImage = null;
  desktopSize.value = null;
  cursorStyle.value = "";
  const canvas = canvasRef.value;
  if (canvas) {
    canvas.width = 0;
    canvas.height = 0;
  }
}

// —— rdp/pointer：服务端光标形状 → CSS cursor ————————————————

function applyPointer(pointer: RdpPointerEvent) {
  const next = cursorFromPointer(pointer);
  if (next.changed) cursorStyle.value = next.cursor;
}

defineExpose({ acceptFrame, reset, applyPointer, perfStats, sendPasteShortcut });

// —— 指针事件 → rdp/input ——

const pressedButtons = new Set<number>();
let lastPointerPoint: { x: number; y: number } | null = null;

function desktopPoint(event: PointerEvent | MouseEvent): { x: number; y: number } | null {
  const canvas = canvasRef.value;
  const size = desktopSize.value;
  if (!canvas || !size || size.width === 0 || size.height === 0) return null;
  const rect = canvas.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return null;
  const x = Math.min(size.width - 1, Math.max(0, Math.floor(((event.clientX - rect.left) * size.width) / rect.width)));
  const y = Math.min(size.height - 1, Math.max(0, Math.floor(((event.clientY - rect.top) * size.height) / rect.height)));
  return { x, y };
}

function capturePointer(event: PointerEvent) {
  try {
    canvasRef.value?.setPointerCapture(event.pointerId);
  } catch {
    // 鼠标捕获在部分 WebView 会抛 InvalidPointerId：不影响普通事件路径。
  }
}

function releasePointer(event: PointerEvent) {
  const canvas = canvasRef.value;
  if (!canvas) return;
  try {
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  } catch {
    // Ignore host-specific capture failures.
  }
}

function onPointerMove(event: PointerEvent) {
  const point = desktopPoint(event);
  if (!point) return;
  lastPointerPoint = point;
  emit("input", { kind: "mouse-move", x: point.x, y: point.y });
}

function onPointerDown(event: PointerEvent) {
  event.preventDefault();
  canvasRef.value?.focus();
  capturePointer(event);
  const point = desktopPoint(event);
  const button = mapDomButtonToRdpButton(event.button);
  if (!point || !button) return;
  lastPointerPoint = point;
  pressedButtons.add(event.button);
  emit("input", { kind: "mouse-button", button, pressed: true, x: point.x, y: point.y });
}

function onPointerUp(event: PointerEvent) {
  const point = desktopPoint(event);
  const button = mapDomButtonToRdpButton(event.button);
  releasePointer(event);
  pressedButtons.delete(event.button);
  if (!point || !button) return;
  lastPointerPoint = point;
  emit("input", { kind: "mouse-button", button, pressed: false, x: point.x, y: point.y });
}

function onPointerCancel(event: PointerEvent) {
  releasePointer(event);
  pressedButtons.clear();
  pressedCodes.clear();
  emit("input", { kind: "release-all" });
}

function onPointerLeave(event: PointerEvent) {
  // Capture normally keeps drags inside the canvas; if the WebView refused it,
  // release every held button at the last known point instead of leaking it.
  if (canvasRef.value?.hasPointerCapture?.(event.pointerId)) return;
  const point = desktopPoint(event) ?? lastPointerPoint;
  if (!point) return;
  for (const buttonNumber of pressedButtons) {
    const button = mapDomButtonToRdpButton(buttonNumber);
    if (button) emit("input", { kind: "mouse-button", button, pressed: false, x: point.x, y: point.y });
  }
  pressedButtons.clear();
}

function onContextMenu(event: MouseEvent) {
  event.preventDefault();
}

function onWheel(event: WheelEvent) {
  event.preventDefault();
  const point = desktopPoint(event);
  if (!point) return;
  // 浏览器增量原样下发（deltaX/deltaY），取反映射为 RDP 旋转单位由 sidecar 负责。
  if (!event.deltaX && !event.deltaY) return;
  emit("input", { kind: "mouse-wheel", deltaX: event.deltaX, deltaY: event.deltaY, x: point.x, y: point.y });
}

// —— 键盘事件 → rdp/input ——

/** 真正发给远端的键码：只对它们发 keyup，粘贴/宿主快捷键不能漏成孤儿按键。 */
const pressedCodes = new Set<string>();

function emitKeyDown(key: string, code: string): boolean {
  const input = buildRdpKeyDownEvent({ key, code });
  if (input === null) return false;
  pressedCodes.add(code);
  emit("input", input);
  return true;
}

function emitKeyUp(key: string, code: string): boolean {
  const input = buildRdpKeyUpEvent({ key, code });
  if (input === null) return false;
  pressedCodes.delete(code);
  emit("input", input);
  return true;
}

/** 宿主 → 远端剪贴板写入后补发 Ctrl+V（macOS 同样映射为 Control），
 *  让宿主的一次粘贴操作直接落到远端应用。若用户已经按住 Ctrl，则只补
 *  V，避免重复按下并提前释放远端的 Control。 */
function sendPasteShortcut() {
  const syntheticControl = !pressedCodes.has(CONTROL_KEY.code);
  if (syntheticControl) emitKeyDown(CONTROL_KEY.key, CONTROL_KEY.code);
  emitKeyDown("v", "KeyV");
  emitKeyUp("v", "KeyV");
  if (syntheticControl) emitKeyUp(CONTROL_KEY.key, CONTROL_KEY.code);
}

function onKeyDown(event: KeyboardEvent) {
  // Ctrl/Cmd+V：剪贴板共享开启时交给父组件同步（读取/写入失败也会补发 Ctrl+V，
  // 不会吞键）；关闭时按普通按键转发，让远端粘贴自己的剪贴板。
  if (isPasteShortcut(event) && props.clipboardEnabled !== false) {
    event.preventDefault();
    emit("paste-request");
    return;
  }
  // 只保留能帮用户离开远端的宿主快捷键（Cmd+W/Q、F5、DevTools 等）。
  if (isLocalOnlyShortcut(event, isMac)) return;
  // macOS：Cmd 映射成 Control（含 Meta 键本身），Cmd+C/A/Z 才能在远端生效。
  if (metaMapsToControl(isMac, event.code) && isMetaCode(event.code)) {
    event.preventDefault();
    emitKeyDown(CONTROL_KEY.key, CONTROL_KEY.code);
    // Track the physical Meta code too so keyup releases the synthetic Control.
    pressedCodes.add(event.code);
    return;
  }
  if (emitKeyDown(event.key, event.code)) event.preventDefault();
}

function onKeyUp(event: KeyboardEvent) {
  if (isMac && isMetaCode(event.code) && pressedCodes.has(event.code)) {
    event.preventDefault();
    emitKeyUp(CONTROL_KEY.key, CONTROL_KEY.code);
    pressedCodes.delete(event.code);
    return;
  }
  if (!pressedCodes.has(event.code)) return;
  pressedCodes.delete(event.code);
  if (emitKeyUp(event.key, event.code)) event.preventDefault();
}

function onBlur() {
  focused.value = false;
  // 焦点丢失（切窗/切面板）发 release-all，避免远端粘键。
  pressedCodes.clear();
  emit("input", { kind: "release-all" });
}

function onPaste(event: ClipboardEvent) {
  const text = event.clipboardData?.getData("text");
  if (text) {
    event.preventDefault();
    emit("clipboard-out", text);
  }
}

onBeforeUnmount(() => {
  pendingPatches = [];
  reusableImage = null;
  pressedCodes.clear();
  resizeObserver?.disconnect();
  resizeObserver = null;
});
</script>

<template>
  <div ref="holderRef" class="vnc-surface-holder" :class="[scaleClass, { 'is-focused': focused }]">
    <canvas
      ref="canvasRef"
      class="vnc-canvas"
      tabindex="0"
      :style="[fitCanvasStyle, cursorStyle ? { cursor: cursorStyle } : undefined]"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerCancel"
      @pointerleave="onPointerLeave"
      @contextmenu="onContextMenu"
      @wheel="onWheel"
      @keydown="onKeyDown"
      @keyup="onKeyUp"
      @focus="focused = true"
      @blur="onBlur"
      @paste="onPaste"
    ></canvas>
  </div>
</template>

<style scoped>
/* 与 VncSurface 同构的画布几何/缩放样式（scoped 各自持有，类名复用同一套
   语义：fit 等比、stretch 铺满、actual 1:1 滚动）。 */
.vnc-surface-holder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  background: var(--background);
  z-index: 2;
}
.vnc-canvas {
  image-rendering: auto;
  outline: none;
  cursor: default;
  touch-action: none;
  user-select: none;
  /* 帧到来前 canvas 尺寸为 0，给画布一个可点击聚焦的最小区域。 */
  min-width: 320px;
  min-height: 200px;
}
.vnc-surface-holder.is-focused .vnc-canvas {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--primary, #2563eb) 35%, transparent);
}
/* fit：等比缩放到视图内（默认）。 */
.vnc-scale-fit .vnc-canvas {
  max-width: 100%;
  max-height: 100%;
}
/* stretch：铺满视图（不保比例）。 */
.vnc-scale-stretch .vnc-canvas {
  width: 100%;
  height: 100%;
}
/* actual：1:1 原始像素，超出滚动。 */
.vnc-scale-actual {
  justify-content: flex-start;
  align-items: flex-start;
  overflow: auto;
}
.vnc-scale-actual .vnc-canvas {
  max-width: none;
  max-height: none;
  /* 1:1 显示在 Retina/HiDPI 上由设备像素放大，pixelated 保持像素锐利。 */
  image-rendering: pixelated;
}
</style>
