<script setup lang="ts">
// VNC 画布表面（nyaterm-parity P2 2d）：接收 App.vue 解码好的帧补丁做
// putImageData 增量绘制（rAF 合帧），fit/stretch/actual 三种前端缩放，
// 键鼠事件经 keysym 映射后交给父组件发送（vnc/input）。状态覆盖层
// （connecting/closed）由 App.vue 的 terminal-overlay 分支承担，本组件
// 只负责活的桌面画面与输入采集。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { decodeVncFramePatch, mapKeyboardEventToVncKeysym, pointerButtonMask, type VncFramePatch, type VncInputEvent } from "../lib/vncFrame";
import { isLocalOnlyShortcut, isMacPlatform, isMetaCode, isPasteShortcut, metaMapsToControl } from "../lib/remoteKeyboard";
import { cursorFromPointer } from "../lib/remoteCursor";
import type { RdpPointerEvent } from "../lib/rdpFrame";

interface Props {
  scaleMode: "fit" | "stretch" | "actual";
}
const props = defineProps<Props>();
const emit = defineEmits<{
  input: [VncInputEvent];
  "clipboard-out": [string];
  /** Ctrl/Cmd+V：父组件读取宿主剪贴板并同步到远端。 */
  "paste-request": [];
  /** Ctrl/Cmd+C：父组件探测远端 X11 文件剪贴板（RFB 本身只传文本）。 */
  "copy-request": [];
  /** 画布 CSS 尺寸变化（侧栏开合/窗口缩放）：匹配窗口模式据此重协商远端分辨率。 */
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

// rAF 合帧队列：同一帧内的多个补丁一次绘制，避免高频 update 撕裂。
let pendingPatches: VncFramePatch[] = [];
let drawScheduled = false;
let lastSequence = -1;
// Reused ImageData (RdpSurface parity): a steady 30fps desktop otherwise
// allocates a fresh pixel buffer per patch.
let reusableImage: { width: number; height: number; image: ImageData } | null = null;

const scaleClass = computed(() => `vnc-scale-${props.scaleMode}`);
/** Fit mode fills the pane in both directions (up- and downscaling); the
 * explicit size keeps pointer mapping in sync because the canvas element
 * itself is the displayed bitmap, not a letterboxed box. */
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

function scheduleDraw() {
  if (drawScheduled) return;
  drawScheduled = true;
  requestAnimationFrame(() => {
    drawScheduled = false;
    const queued = pendingPatches;
    pendingPatches = [];
    for (const patch of queued) drawPatch(patch);
  });
}

function drawPatch(patch: VncFramePatch) {
  const canvas = canvasRef.value;
  if (!canvas) return;
  if (canvas.width !== patch.desktopWidth || canvas.height !== patch.desktopHeight) {
    canvas.width = patch.desktopWidth;
    canvas.height = patch.desktopHeight;
    desktopSize.value = { width: patch.desktopWidth, height: patch.desktopHeight };
    reusableImage = null;
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
}

/** App.vue 帧通道入口：原始二进制帧在这里解码、乱序丢弃后入绘制队列。 */
function acceptFrame(frame: Uint8Array): boolean {
  let patch: VncFramePatch;
  try {
    patch = decodeVncFramePatch(frame, { zeroCopy: true });
  } catch {
    // 非法帧直接丢弃（sidecar 侧同样的校验不该让坏帧走到这里）。
    return false;
  }
  if (patch.sequence <= lastSequence) return false;
  lastSequence = patch.sequence;
  pendingPatches.push(patch);
  scheduleDraw();
  return true;
}

/** 重连整幅重绘/新会话：清空本地绘制状态，等下一帧。 */
function reset() {
  pendingPatches = [];
  lastSequence = -1;
  desktopSize.value = null;
  reusableImage = null;
  cursorStyle.value = "";
  const canvas = canvasRef.value;
  if (canvas) {
    canvas.width = 0;
    canvas.height = 0;
  }
}

/** vnc/pointer（CursorPseudo）：与 RDP 共用 CSS cursor 转换。 */
function applyPointer(pointer: RdpPointerEvent) {
  const next = cursorFromPointer(pointer);
  if (next.changed) cursorStyle.value = next.cursor;
}

defineExpose({ acceptFrame, reset, applyPointer, sendPasteShortcut });

// —— 指针事件 → vnc/input ——

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

function sendPointer(event: PointerEvent | MouseEvent, extraMask = 0) {
  const point = desktopPoint(event);
  if (!point) return;
  emit("input", { kind: "pointer", x: point.x, y: point.y, buttonMask: pointerButtonMask(event.buttons) | extraMask });
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

function onPointerDown(event: PointerEvent) {
  event.preventDefault();
  canvasRef.value?.focus();
  capturePointer(event);
  sendPointer(event);
}

function onPointerMove(event: PointerEvent) {
  sendPointer(event);
}

function onPointerUp(event: PointerEvent) {
  sendPointer(event);
  releasePointer(event);
}

function onPointerCancel(event: PointerEvent) {
  // 系统取消指针（触控/窗口切换）：补 release-all，避免远端粘键。
  releasePointer(event);
  releaseAllKeys();
  emit("input", { kind: "release-all" });
}

function onPointerLeave(event: PointerEvent) {
  // 未捕获时拖拽移出画布补发释放；捕获期间 leave 不触发，由 pointerup/cancel 收口。
  if (canvasRef.value?.hasPointerCapture?.(event.pointerId)) return;
  sendPointer(event);
}

function onContextMenu(event: MouseEvent) {
  event.preventDefault();
}

function onWheel(event: WheelEvent) {
  event.preventDefault();
  const wheelMask = event.deltaY < 0 ? 8 : event.deltaY > 0 ? 16 : 0;
  if (!wheelMask) return;
  const point = desktopPoint(event);
  if (!point) return;
  // 滚轮是瞬时脉冲：按下 + 释放两个指针事件；保留当前按住的鼠标键，
  // 否则拖拽中滚动会把远端拖拽状态打断。
  const held = pointerButtonMask(event.buttons);
  emit("input", { kind: "pointer", x: point.x, y: point.y, buttonMask: held | wheelMask });
  emit("input", { kind: "pointer", x: point.x, y: point.y, buttonMask: held });
}

// —— 键盘事件 → vnc/input ——

const pressedKeysyms = new Set<number>();
const pressedCodes = new Set<string>();

function releaseAllKeys() {
  for (const keysym of pressedKeysyms) {
    emit("input", { kind: "key", keysym, pressed: false });
  }
  pressedKeysyms.clear();
  pressedCodes.clear();
}

function emitMappedKey(key: string, code: string, pressed: boolean): boolean {
  const keysym = mapKeyboardEventToVncKeysym({ key, code });
  if (keysym === null) return false;
  if (pressed) {
    pressedKeysyms.add(keysym);
    pressedCodes.add(code);
  } else {
    pressedKeysyms.delete(keysym);
    pressedCodes.delete(code);
  }
  emit("input", { kind: "key", keysym, pressed });
  return true;
}

/** 宿主 → 远端剪贴板写入后补发 Ctrl+V：用户只按一次粘贴键。
 *  macOS 也发送 Control（远端是 Windows/Linux 语义）。 */
function sendPasteShortcut() {
  // If the user is already holding Control (e.g. Ctrl+V), reuse it instead of
  // releasing it early.
  const syntheticControl = !pressedKeysyms.has(0xffe3);
  if (syntheticControl) emitMappedKey(CONTROL_KEY.key, CONTROL_KEY.code, true);
  emitMappedKey("v", "KeyV", true);
  emitMappedKey("v", "KeyV", false);
  if (syntheticControl) emitMappedKey(CONTROL_KEY.key, CONTROL_KEY.code, false);
}

function onKeyDown(event: KeyboardEvent) {
  // Ctrl/Cmd+V：交给父组件同步（读取失败也会补发 Ctrl+V，不会吞键）。
  if (isPasteShortcut(event)) {
    event.preventDefault();
    emit("paste-request");
    return;
  }
  // A remote file copy is invisible over RFB; probe the SSH-side X selection
  // while still forwarding the key (macOS maps Cmd → Control below).
  if ((isMac ? event.metaKey : event.ctrlKey) && event.code === "KeyC") {
    emit("copy-request");
  }
  // 只保留能帮用户离开远端的宿主快捷键（Cmd+W/Q、F5、DevTools 等）。
  if (isLocalOnlyShortcut(event, isMac)) return;
  // macOS：Cmd 映射成 Control（含 Meta 键本身），Cmd+C/A/Z 才能在远端生效。
  if (metaMapsToControl(isMac, event.code) && isMetaCode(event.code)) {
    event.preventDefault();
    emitMappedKey(CONTROL_KEY.key, CONTROL_KEY.code, true);
    // Track the physical Meta code too: keyup checks MetaLeft/MetaRight, so
    // without this entry the synthetic Control would stay down until blur.
    pressedCodes.add(event.code);
    return;
  }
  if (emitMappedKey(event.key, event.code, true)) event.preventDefault();
}

function onKeyUp(event: KeyboardEvent) {
  if (isMac && isMetaCode(event.code) && pressedCodes.has(event.code)) {
    event.preventDefault();
    emitMappedKey(CONTROL_KEY.key, CONTROL_KEY.code, false);
    pressedCodes.delete(event.code);
    return;
  }
  // 只对真正发给远端的键发 release（粘贴/宿主快捷键的 keyup 不能漏成孤儿按键）。
  if (!pressedCodes.has(event.code)) return;
  if (emitMappedKey(event.key, event.code, false)) event.preventDefault();
}

function onBlur() {
  focused.value = false;
  // 焦点丢失（切窗/切面板）释放全部按下的键，避免远端粘键。
  releaseAllKeys();
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
  resizeObserver?.disconnect();
  resizeObserver = null;
  releaseAllKeys();
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
