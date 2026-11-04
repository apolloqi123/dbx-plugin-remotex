<script setup lang="ts">
// 容器 PTY 终端页：xterm 实例与视图归本组件，协议（open/close/resize、
// 二进制输出、state 事件）归 useContainerTerminal。
//
// 生命周期：只有「页签可见 + 容器 running + 可写」时才建立 PTY；隐藏页签时
// PTY 与 xterm 都保持存活（切回只 fit/聚焦）；容器切换复位 xterm 并重开；
// 不可写/未运行时只显示提示、不发 open；卸载关闭 PTY 并销毁 xterm。
// 主题/字体由工作台下传（theme/fontFamily/fontSize），缺省用内置回退值。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Terminal as XTerm, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import {
  useContainerTerminal,
  type ContainerTerminalStateEvent,
} from "../composables/useContainerTerminal";
import { isDockerContainerRunning, type DockerContainer } from "../lib/dockerPanelModel";
import { createTerminalOutputColorizer } from "../../terminal/lib/emulator/terminalOutputColor";

const DEFAULT_FONT_FAMILY = "Menlo, Monaco, 'Courier New', monospace";
const DEFAULT_FONT_SIZE = 13;

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
  /** 页签可见性：不可见时不建立 PTY，但已建立的 PTY 保持存活。 */
  visible: boolean;
  theme?: Record<string, unknown>;
  fontFamily?: string;
  fontSize?: number;
  /** 工作台事件总线上的 docker/terminal/state 订阅（可选）。 */
  terminalStateEvents?: (handler: (event: ContainerTerminalStateEvent) => void) => () => void;
}>();

const hostRef = ref<HTMLElement>();
const exitCode = ref<number | undefined>();

const running = computed(() => isDockerContainerRunning(props.container));

let terminal: XTerm | undefined;
let fitAddon: FitAddon | undefined;
let dataDisposable: { dispose(): void } | undefined;
let resizeDisposable: { dispose(): void } | undefined;
let resizeObserver: ResizeObserver | undefined;
let unsubscribeState: (() => void) | undefined;
let fitFrame = 0;
let starting = false;
/** 容器在 open 在途时又切走：open 因代际过期返回后必须补开最新目标。 */
let pendingStart = false;
let disposed = false;

function canConnect(): boolean {
  return running.value && props.canWrite !== false;
}

// 输出着色：与主终端同一转换器（备用屏/ESC/大输出透传），只改显示。
const outputColorizer = createTerminalOutputColorizer({
  isAlternateScreen: () => terminal?.buffer?.active?.type === "alternate",
});

const { status, errorMessage, open, close, retry, sendInput, resize, handleStateEvent } =
  useContainerTerminal({
    containerId: () => props.container.id,
    sessionId: () => props.sessionId,
    connectionId: () => props.connectionId,
    canWrite: () => props.canWrite !== false,
    running: () => running.value,
    t: (key, values) => props.t(key, values),
    onData: (bytes) => terminal?.write(outputColorizer.transform(bytes)),
    onState: (state, detail) => {
      if (state === "exited") exitCode.value = detail?.exitCode;
      if (state === "connecting" || state === "connected") exitCode.value = undefined;
    },
  });

/** 仅在宿主有实际尺寸时 fit：隐藏页签/未布局时 fit 会抛错或算出 0 列。 */
function fitNow(): void {
  const host = hostRef.value;
  if (!fitAddon || !terminal || !host || host.clientWidth <= 0 || host.clientHeight <= 0) return;
  try {
    fitAddon.fit();
  } catch {
    // iframe/页签切换过程中宿主可能已脱离文档；保留 xterm 现有尺寸。
  }
}

/** ResizeObserver 回调按帧合并：拖拽分隔条时每帧最多 fit 一次。 */
function scheduleFit(): void {
  if (disposed) return;
  if (typeof window.requestAnimationFrame !== "function") {
    fitNow();
    return;
  }
  if (fitFrame) return;
  fitFrame = window.requestAnimationFrame(() => {
    fitFrame = 0;
    fitNow();
  });
}

/**
 * 建立 PTY：只在本页可见时调用。read-only/未运行的 open 会在 composable 内
 * 直接返回 disabled（不发 RPC），因此这两类状态也走这里拿到提示文案。
 * exited/error 需要用户点「重新连接」，不自动重试。
 */
async function startIfNeeded(): Promise<void> {
  if (disposed || !terminal || !props.visible) {
    pendingStart = false;
    return;
  }
  // 快速切容器时上一次 open 可能仍在途（含等待 close RPC）；记下待补开，
  // 等它结束后对最新容器重开，否则终态会停在 idle/connecting 而没有 PTY。
  if (starting) {
    pendingStart = true;
    return;
  }
  // 不可连接时仍走 open：只拿 disabled 提示文案，不发 RPC。
  if (status.value === "disabled") {
    if (!canConnect()) return;
    // 页签隐藏期间容器被启动/连接变为可写：disabled 已过期，复位后重开。
    close();
  }
  if (status.value !== "idle" && status.value !== "disabled") return;
  starting = true;
  try {
    await nextTick();
    if (disposed || !props.visible) return;
    fitNow();
    await open(terminal.cols, terminal.rows);
  } finally {
    starting = false;
  }
  if (!disposed && props.visible) terminal?.focus();
  if (pendingStart) {
    pendingStart = false;
    // 只补开「被新目标取代」的场景：error/disabled 要等用户点重试。
    if (!disposed && props.visible && status.value === "idle") void startIfNeeded();
  }
}

async function retryTerminal(): Promise<void> {
  if (!terminal || disposed) return;
  exitCode.value = undefined;
  await nextTick();
  fitNow();
  await retry(terminal.cols, terminal.rows);
  if (!disposed && status.value === "connected") terminal.focus();
}

function createTerminal(): void {
  const host = hostRef.value;
  if (!host || terminal) return;
  terminal = new XTerm({
    convertEol: false,
    cursorBlink: true,
    fontFamily: props.fontFamily || DEFAULT_FONT_FAMILY,
    fontSize: props.fontSize || DEFAULT_FONT_SIZE,
    scrollback: 5_000,
    allowProposedApi: true,
    theme: (props.theme ?? {}) as ITheme,
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(host);
  // Unicode11 与主终端一致：在 open() 之后加载再切换 activeVersion。
  terminal.loadAddon(new Unicode11Addon());
  terminal.unicode.activeVersion = "11";
  terminal.loadAddon(new WebLinksAddon());
  dataDisposable = terminal.onData((data) => sendInput(data));
  resizeDisposable = terminal.onResize(({ cols, rows }) => resize(cols, rows));
}

onMounted(() => {
  // 懒创建：没打开过终端页签就不占一个 xterm + WebGL 上下文（每个详情页常驻
  // 四个页签，v-show 只隐藏不卸载）。首次可见/首次输入时再建实例。
  if (props.visible) createTerminal();
  const host = hostRef.value;
  if (host && typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => scheduleFit());
    resizeObserver.observe(host);
  }
  unsubscribeState = props.terminalStateEvents?.((event) => handleStateEvent(event));
  void startIfNeeded();
});

onBeforeUnmount(() => {
  disposed = true;
  outputColorizer.reset();
  unsubscribeState?.();
  unsubscribeState = undefined;
  if (fitFrame && typeof window.cancelAnimationFrame === "function") {
    window.cancelAnimationFrame(fitFrame);
  }
  fitFrame = 0;
  resizeObserver?.disconnect();
  resizeObserver = undefined;
  dataDisposable?.dispose();
  dataDisposable = undefined;
  resizeDisposable?.dispose();
  resizeDisposable = undefined;
  // 先关闭远端 PTY（若已打开），再销毁 xterm：销毁后不再接收输出。
  close();
  terminal?.dispose();
  terminal = undefined;
  fitAddon = undefined;
});

// 页签隐藏：PTY 保持存活，仅停止 fit/聚焦；重新可见时补一次 fit 并聚焦。
watch(
  () => props.visible,
  async (visible) => {
    if (!visible || disposed) return;
    if (!terminal) {
      createTerminal();
      scheduleFit();
    }
    await nextTick();
    fitNow();
    terminal?.focus();
    void startIfNeeded();
  },
);

// 容器切换：旧 shell 不可复用，复位显示并（可见时）为新容器重开。
watch(
  () => props.container.id,
  () => {
    if (disposed) return;
    exitCode.value = undefined;
    terminal?.reset();
    close();
    void startIfNeeded();
  },
);

// 运行状态/只读变化：变为不可连接时关闭并展示提示；恢复后重新建立。
watch([running, () => props.canWrite], () => {
  if (disposed || !props.visible) return;
  if (!canConnect()) {
    close();
    void startIfNeeded();
    return;
  }
  if (status.value === "disabled") close();
  void startIfNeeded();
});

watch(
  () => props.theme,
  (theme) => {
    if (terminal) terminal.options.theme = (theme ?? {}) as ITheme;
  },
  { deep: true },
);

watch(
  () => [props.fontFamily, props.fontSize] as const,
  ([fontFamily, fontSize]) => {
    if (!terminal) return;
    terminal.options.fontFamily = fontFamily || DEFAULT_FONT_FAMILY;
    terminal.options.fontSize = fontSize || DEFAULT_FONT_SIZE;
    scheduleFit();
  },
);
</script>

<template>
  <div class="container-terminal-tab" data-tab-panel="terminal">
    <div ref="hostRef" class="container-terminal-host" data-xterm-host />
    <div v-if="status !== 'connected'" class="container-terminal-overlay" data-terminal-overlay>
      <p v-if="status === 'disabled'" class="container-terminal-message">{{ errorMessage }}</p>
      <p v-else-if="status === 'exited'" class="container-terminal-message">
        {{ t("docker.terminalExited", { code: exitCode ?? "?" }) }}
      </p>
      <p v-else-if="status === 'error'" class="container-terminal-message" :title="errorMessage">
        {{ errorMessage }}
      </p>
      <p v-else class="container-terminal-message">{{ t("docker.terminalConnecting") }}</p>
      <button
        v-if="status === 'exited' || status === 'error'"
        type="button"
        class="container-terminal-retry"
        data-terminal-retry
        @click="retryTerminal"
      >
        {{ t("docker.terminalRetry") }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.container-terminal-tab {
  position: relative;
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--background);
}

.container-terminal-host {
  position: absolute;
  inset: 0;
  padding: 4px;
  overflow: hidden;
}

/* xterm 自行创建 .xterm 节点：让它填满宿主内容盒（padding 之外）。 */
.container-terminal-host :deep(.xterm) {
  height: 100%;
}

.container-terminal-host :deep(.xterm-viewport) {
  scrollbar-width: thin;
}

.container-terminal-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px;
  text-align: center;
  background: color-mix(in srgb, var(--background) 84%, transparent);
}

.container-terminal-message {
  margin: 0;
  max-width: 90%;
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.85;
  overflow-wrap: anywhere;
}

.container-terminal-retry {
  border: 1px solid var(--border);
  border-radius: 4px;
  background: none;
  color: inherit;
  padding: 2px 12px;
  font-size: 12px;
  cursor: pointer;
}

.container-terminal-retry:hover {
  border-color: var(--primary);
}
</style>
