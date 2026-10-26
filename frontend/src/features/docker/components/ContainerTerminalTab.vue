<script setup lang="ts">
// 容器命令终端页：每条命令都是一次独立的 `docker exec <id> sh -lc <command>`
// （无常驻 shell，cwd/env 不跨命令保留），前端 60s 超时。输出是只读文本缓冲，
// 超过上限按头部裁剪；命令历史只在本页签内记忆（↑/↓，最多 50 条）。
// 只读连接（canWrite === false）直接禁用输入与执行；undefined 表示未限制。
import { computed, nextTick, ref, watch } from "vue";
import { Loader2 } from "@lucide/vue";
import { rpc } from "../../../shared/lib/rpc";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import type { DockerContainer } from "../lib/dockerPanelModel";

const MAX_HISTORY = 50;
const MAX_OUTPUT_CHARS = 200_000;
const EXEC_TIMEOUT_MS = 60_000;

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
}>();

const input = ref("");
const output = ref("");
const busy = ref(false);
const error = ref("");
const history = ref<string[]>([]);
const outputRef = ref<HTMLElement>();
const inputRef = ref<HTMLInputElement>();

/** -1 = 正在编辑新命令；否则指向 history 下标（0 最旧）。 */
let historyIndex = -1;
/** 离开新命令开始浏览历史时暂存输入，ArrowDown 到底后还原。 */
let draft = "";

const readOnly = computed(() => props.canWrite === false);

/** 只保留末尾 MAX_OUTPUT_CHARS 个字符（头部裁剪，最新输出始终可见）。 */
function trimHead(text: string): string {
  return text.length > MAX_OUTPUT_CHARS ? text.slice(text.length - MAX_OUTPUT_CHARS) : text;
}

function scrollToBottom(): void {
  void nextTick(() => {
    const element = outputRef.value;
    if (element) element.scrollTop = element.scrollHeight;
  });
}

function append(text: string): void {
  if (!text) return;
  output.value = trimHead(output.value + text);
  scrollToBottom();
}

function focusInput(): void {
  void nextTick(() => inputRef.value?.focus());
}

/** 响应归一：缺失/异常形状不阻断渲染，输出按空串、退出码显示 “?”。 */
function execResult(raw: unknown): { output: string; exitCode: number | null } {
  const record = (raw ?? {}) as Record<string, unknown>;
  return {
    output: typeof record.output === "string" ? record.output : "",
    exitCode:
      typeof record.exitCode === "number" && Number.isFinite(record.exitCode)
        ? Math.trunc(record.exitCode)
        : null,
  };
}

/** 与 FilesTab 同口径：SSH 会话优先，否则本机 daemon；引擎设置按连接附带。 */
function requestParams(command: string): Record<string, unknown> {
  return {
    ...(props.sessionId ? { sessionId: props.sessionId } : { target: "local" as const }),
    containerId: props.container.id,
    command,
    ...dockerEngineParams(loadDockerEngineSettings(props.connectionId || "")),
  };
}

/** 相邻重复命令不重复入历史；超出上限从最旧开始淘汰。 */
function remember(command: string): void {
  if (history.value[history.value.length - 1] !== command) {
    history.value.push(command);
    if (history.value.length > MAX_HISTORY) {
      history.value.splice(0, history.value.length - MAX_HISTORY);
    }
  }
  historyIndex = -1;
  draft = "";
}

async function run(): Promise<void> {
  if (readOnly.value || busy.value) return;
  const command = input.value.trim();
  if (!command) return;
  const containerId = props.container.id;
  busy.value = true;
  error.value = "";
  append(`$ ${command}\n`);
  input.value = "";
  remember(command);
  try {
    const result = execResult(
      await rpc<unknown>("docker/exec", requestParams(command), { timeoutMs: EXEC_TIMEOUT_MS }),
    );
    // 切容器后旧响应丢弃，不写进新容器的输出。
    if (props.container.id !== containerId) return;
    if (result.output) {
      append(result.output.endsWith("\n") ? result.output : `${result.output}\n`);
    }
    append(`${props.t("docker.execExitCode", { code: result.exitCode ?? "?" })}\n`);
  } catch (cause) {
    if (props.container.id !== containerId) return;
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    busy.value = false;
    focusInput();
  }
}

function clear(): void {
  output.value = "";
  error.value = "";
}

function setInput(value: string): void {
  input.value = value;
  void nextTick(() => {
    const element = inputRef.value;
    if (!element) return;
    element.focus();
    const end = element.value.length;
    element.setSelectionRange?.(end, end);
  });
}

function historyStep(delta: -1 | 1): void {
  if (!history.value.length) return;
  if (delta < 0) {
    if (historyIndex === -1) {
      draft = input.value;
      historyIndex = history.value.length - 1;
    } else if (historyIndex > 0) {
      historyIndex -= 1;
    }
    setInput(history.value[historyIndex]);
    return;
  }
  if (historyIndex === -1) return;
  if (historyIndex >= history.value.length - 1) {
    historyIndex = -1;
    setInput(draft);
    return;
  }
  historyIndex += 1;
  setInput(history.value[historyIndex]);
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing || event.keyCode === 229) return;
  if (event.key === "ArrowUp") {
    event.preventDefault();
    historyStep(-1);
  } else if (event.key === "ArrowDown") {
    event.preventDefault();
    historyStep(1);
  } else if (event.key === "Enter") {
    event.preventDefault();
    void run();
  }
}

// 容器切换：上一容器的输出/输入/历史都不再适用（在途响应由 run 内 id 校验丢弃）。
watch(
  () => props.container.id,
  () => {
    output.value = "";
    error.value = "";
    input.value = "";
    history.value = [];
    historyIndex = -1;
    draft = "";
    busy.value = false;
  },
);
</script>

<template>
  <div class="container-terminal-tab" data-tab-panel="terminal">
    <p class="docker-exec-hint" data-exec-hint>{{ t("docker.execHint") }}</p>
    <div class="docker-exec-body">
      <pre ref="outputRef" class="docker-exec-output" data-exec-output>{{ output }}</pre>
      <p v-if="!output" class="docker-exec-empty" data-exec-empty>{{ t("docker.execEmpty") }}</p>
    </div>
    <p v-if="error" class="docker-exec-error" data-exec-error role="alert">
      {{ t("docker.execFailed", { error }) }}
    </p>
    <p v-if="busy" class="docker-exec-status" data-exec-busy>
      <Loader2 class="spin" />
      {{ t("docker.execRunning") }}
    </p>
    <p v-if="readOnly" class="docker-exec-readonly" data-exec-readonly>{{ t("docker.execReadOnly") }}</p>
    <form class="docker-exec-form" @submit.prevent="void run()">
      <span class="docker-exec-prompt" aria-hidden="true">$</span>
      <input
        ref="inputRef"
        v-model="input"
        type="text"
        class="docker-exec-input mono"
        data-exec-input
        autocomplete="off"
        spellcheck="false"
        :placeholder="t('docker.execPlaceholder')"
        :aria-label="t('docker.execPlaceholder')"
        :disabled="readOnly"
        @keydown="onKeydown"
      />
      <button
        type="submit"
        class="docker-exec-run"
        data-action="exec-run"
        :title="t('docker.execRun')"
        :aria-label="t('docker.execRun')"
        :disabled="readOnly || busy"
      >
        {{ t("docker.execRun") }}
      </button>
      <button
        type="button"
        class="docker-exec-clear"
        data-action="exec-clear"
        :title="t('docker.execClear')"
        :aria-label="t('docker.execClear')"
        @click="clear"
      >
        {{ t("docker.execClear") }}
      </button>
    </form>
  </div>
</template>

<style scoped>
.container-terminal-tab {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.docker-exec-hint {
  flex: 0 0 auto;
  margin: 0;
  padding: 3px 8px;
  border-bottom: 1px solid var(--border);
  font-size: 11px;
  opacity: 0.7;
}

/* 终端输出固定深色底：与主题明暗无关，始终像一块终端。 */
.docker-exec-body {
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
  background: #0b0e14;
  color: #d5dae2;
}

.docker-exec-output {
  flex: 1;
  min-height: 0;
  margin: 0;
  overflow: auto;
  padding: 6px 8px;
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-all;
}

.docker-exec-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  font-size: 11px;
  opacity: 0.5;
  pointer-events: none;
}

.docker-exec-error {
  flex: 0 0 auto;
  margin: 0;
  padding: 3px 8px;
  font-size: 11px;
  background: color-mix(in srgb, var(--destructive) 12%, transparent);
  color: var(--destructive);
}

.docker-exec-status,
.docker-exec-readonly {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  margin: 0;
  padding: 3px 8px;
  font-size: 11px;
  opacity: 0.75;
}

.docker-exec-readonly {
  color: var(--destructive);
  opacity: 1;
}

.docker-exec-form {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  padding: 4px 6px;
  border-top: 1px solid var(--border);
}

.docker-exec-prompt {
  flex: 0 0 auto;
  padding: 0 2px;
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 12px;
  opacity: 0.7;
}

.docker-exec-input {
  flex: 1;
  min-width: 0;
  padding: 2px 6px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--background);
  color: inherit;
  font-size: 11px;
}

.docker-exec-input:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.docker-exec-run,
.docker-exec-clear {
  flex: 0 0 auto;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: none;
  color: inherit;
  padding: 1px 8px;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}

.docker-exec-run:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.spin {
  animation: docker-exec-spin 1s linear infinite;
}

@keyframes docker-exec-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
