<script setup lang="ts">
// 容器日志页：真流式 + 默认跟随。`useDockerLogStream` 由本组件内部持有（props
// 契约固定为 t/container/sessionId/connectionId/canWrite）。页签不可见时停止
// 跟随（不再占用一条 SSH session channel，服务器 MaxSessions 较小时尤其重要），
// 切回日志页再按当前 tail 重新跟随；容器切换/页面关闭/卸载同样收流。
//
// 跟随规则（spec §3.2）：默认跟随；向上滚动距底 >24px 暂停自动滚动但继续接收；
// 滚回距底 ≤4px 自动恢复；按钮在跟随时点一下只关自动滚动（不断流）。
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { FOLLOW_PAUSE_PX, MAX_LOG_CHARS, MAX_LOG_LINES, shouldFollow } from "../lib/dockerLogBuffer";
import { TAIL_OPTIONS, type DockerContainer } from "../lib/dockerPanelModel";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import { useDockerLogStream, type DockerLogStreamStatus } from "../composables/useDockerLogStream";
import { escapeLogHtml, tokenizeOutputLine } from "../../../shared/lib/logTokens";

// Vue 对未传的 Boolean prop 会转成 false；这里显式默认 true，避免「未声明
// visible」的调用方被误判为隐藏页签而不开流。
const props = withDefaults(defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
  /** 页签是否可见：隐藏时停流，重新显示时恢复跟随并补一次置底。 */
  visible?: boolean;
}>(), { visible: true });

const stream = useDockerLogStream({
  sessionId: () => props.sessionId,
  // 引擎设置与 list/files/exec 同口径按连接附带；缺省走默认 docker/本机 daemon。
  engineParams: () => dockerEngineParams(loadDockerEngineSettings(props.connectionId || "")),
});

const {
  text,
  truncated,
  follow,
  status,
  error,
  needsSudo,
  exitCode,
  start,
  stop,
  restart,
  retrySudo,
  clear,
} = stream;

const tail = ref<string>("200");
const scroller = ref<HTMLElement>();
/** 用户是否刚点过“清空”（只用于空态提示，不参与流状态）。 */
const clearedNotice = ref(false);
/** 流是否因页签隐藏而停止：只有这种停止才在重新可见时自动重开。 */
let pausedByHidden = false;

// —— 日志着色 ——————————————————————————————————————————————
// 逐行按 logTokens 语义着色（级别/时间戳/键值/数字/IP/URL + 应用自带的 ANSI
// 颜色）。行 HTML 按内容缓存：流式追加时只有新行付出解析成本，重复行直接复用。
// 高亮上限之外的头部按纯文本整段渲染，避免万行缓冲把 DOM 撑爆。
const LOG_HIGHLIGHT_MAX_LINES = 2000;
const LOG_ROW_CACHE_LIMIT = 8192;
const rowHtmlCache = new Map<string, string>();

function logRowHtml(line: string): string {
  const cached = rowHtmlCache.get(line);
  if (cached !== undefined) return cached;
  let html = "";
  for (const token of tokenizeOutputLine(line)) {
    html += token.kind === "plain"
      ? escapeLogHtml(token.text)
      : `<span class="log-${token.kind}">${escapeLogHtml(token.text)}</span>`;
  }
  if (rowHtmlCache.size >= LOG_ROW_CACHE_LIMIT) rowHtmlCache.clear();
  rowHtmlCache.set(line, html);
  return html;
}

const renderedLog = computed(() => {
  const value = text.value;
  if (!value) return { prefix: "", rows: [] as string[] };
  const lines = value.split("\n");
  const highlightFrom = Math.max(0, lines.length - LOG_HIGHLIGHT_MAX_LINES);
  return {
    prefix: highlightFrom > 0 ? `${lines.slice(0, highlightFrom).join("\n")}\n` : "",
    rows: lines.slice(highlightFrom).map(logRowHtml),
  };
});

/** 截断提示文案；数值直接取自缓冲上限，避免两处漂移。 */
const truncatedHint = computed(() =>
  props.t("docker.logsTruncated", { lines: MAX_LOG_LINES, chars: MAX_LOG_CHARS / 1_000_000 }),
);

function scrollToBottom(): void {
  void nextTick(() => {
    const element = scroller.value;
    if (element) element.scrollTop = element.scrollHeight;
  });
}

/** 清空按钮：只清视图（不断流），并在空态提示“已清空”。 */
function handleClear(): void {
  clear();
  clearedNotice.value = true;
}

function resumeFollow(): void {
  if (!follow.value) follow.value = true;
  scrollToBottom();
}

function pauseFollow(): void {
  if (follow.value) follow.value = false;
}

/** 按钮：跟随时点一下只停自动滚动；暂停时恢复并跳到最新。 */
function toggleFollow(): void {
  if (follow.value) pauseFollow();
  else resumeFollow();
}

function onScroll(): void {
  const element = scroller.value;
  if (!element) return;
  const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
  if (distance > FOLLOW_PAUSE_PX) pauseFollow();
  else if (shouldFollow(element.scrollTop, element.clientHeight, element.scrollHeight)) resumeFollow();
}

/** tail 变更：停旧流、清空缓冲、按新 tail 重启（spec §3.2）。 */
async function onTailChange(): Promise<void> {
  clearedNotice.value = false;
  clear();
  await restart({ tail: tail.value });
}

/** 结束横幅的“重新开始”：清掉上一轮的输出后按原 tail 重开，避免 tail 行重复。 */
function restartFromEnd(): void {
  clearedNotice.value = false;
  clear();
  void restart();
}

const endedBanner = (value: DockerLogStreamStatus): boolean => value === "ended" || value === "error";
/** 启动中还没有任何输出是正常的：不要显示“无输出”空态。 */
const showEmptyState = computed(() => !text.value && status.value !== "starting");

onMounted(() => {
  if (props.visible === false) {
    pausedByHidden = true;
    return;
  }
  void start(props.container.id, { tail: tail.value });
});

// 容器切换：先清视图再开新流（start 内部会停掉旧流）。页签隐藏时只清视图，
// 不建立 channel；重新可见时再按新容器开流。
watch(
  () => props.container.id,
  (containerId) => {
    clearedNotice.value = false;
    clear();
    if (props.visible === false) {
      pausedByHidden = true;
      return;
    }
    pausedByHidden = false;
    void start(containerId, { tail: tail.value });
  },
);

// 新日志到达：跟随态滚到底；暂停态不动滚动位置（内容继续进缓冲）。
watch(text, (value) => {
  if (value) clearedNotice.value = false;
  if (follow.value) scrollToBottom();
});

// 页签显隐：隐藏即停流（日志页不可见时不必占用 SSH channel）；重新可见时
// 若是本组件主动暂停的流则按当前 tail 重开，并补一次置底（v-show 隐藏期间
// 节点没有布局，scrollToBottom 无效）。
watch(
  () => props.visible,
  (visible) => {
    if (visible === false) {
      if (status.value === "streaming" || status.value === "starting") {
        pausedByHidden = true;
        void stop();
      }
      return;
    }
    if (pausedByHidden) {
      pausedByHidden = false;
      clearedNotice.value = false;
      clear();
      void start(props.container.id, { tail: tail.value });
    }
    if (follow.value) scrollToBottom();
  },
);
</script>

<template>
  <div class="container-logs-tab" data-tab-panel="logs">
    <div class="docker-log-toolbar">
      <label class="docker-log-tail">
        <span>{{ t("docker.logsTail") }}</span>
        <select v-model="tail" data-action="tail" @change="onTailChange">
          <option v-for="option in TAIL_OPTIONS" :key="option" :value="option">{{ option }}</option>
        </select>
      </label>
      <button
        type="button"
        class="docker-log-follow"
        data-action="follow"
        :class="{ 'is-active': follow }"
        :aria-pressed="follow ? 'true' : 'false'"
        @click="toggleFollow"
      >
        {{ follow ? t("docker.followLive") : t("docker.followResume") }}
      </button>
      <span v-if="!follow" class="docker-log-paused">{{ t("docker.followPaused") }}</span>
      <span v-if="truncated" class="docker-log-truncated">{{ truncatedHint }}</span>
      <span class="docker-log-spacer" />
      <button type="button" class="docker-log-clear" data-action="clear" @click="handleClear">
        {{ t("docker.logsClear") }}
      </button>
    </div>
    <div v-if="endedBanner(status) || needsSudo" class="docker-log-banner" data-role="stream-end">
      <span class="docker-log-ended">{{ t("docker.streamEnded") }}</span>
      <span v-if="exitCode !== null" class="docker-log-exit">exit {{ exitCode }}</span>
      <span v-if="error" class="docker-log-error">{{ error }}</span>
      <span class="docker-log-spacer" />
      <button
        v-if="needsSudo"
        type="button"
        class="docker-log-sudo"
        data-action="sudo-retry"
        :disabled="canWrite === false"
        @click="retrySudo()"
      >
        {{ t("docker.sudoRetry") }}
      </button>
      <button type="button" class="docker-log-restart" data-action="restart" @click="restartFromEnd">
        {{ t("docker.streamRetry") }}
      </button>
    </div>
    <div class="docker-log-body">
      <pre ref="scroller" class="docker-log-scroll" @scroll="onScroll"><template v-if="renderedLog.prefix">{{ renderedLog.prefix }}</template><template v-for="(row, index) in renderedLog.rows" :key="index"><span class="docker-log-line" v-html="row"></span>{{ index + 1 < renderedLog.rows.length ? "\n" : "" }}</template></pre>
      <p v-if="showEmptyState" class="docker-log-empty">
        {{ clearedNotice ? t("docker.logsCleared") : t("docker.logsEmpty") }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.container-logs-tab {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
}

.docker-log-toolbar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 2px 6px;
  font-size: 11px;
}

.docker-log-tail {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
}

.docker-log-tail select {
  font-size: 11px;
  padding: 0 2px;
}

.docker-log-follow,
.docker-log-clear,
.docker-log-sudo,
.docker-log-restart {
  border: 1px solid var(--border);
  border-radius: 4px;
  background: none;
  color: inherit;
  padding: 1px 8px;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}

.docker-log-follow.is-active {
  border-color: var(--primary);
  color: var(--primary);
}

.docker-log-follow:disabled,
.docker-log-sudo:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.docker-log-paused,
.docker-log-truncated {
  opacity: 0.7;
  white-space: nowrap;
}

.docker-log-spacer {
  flex: 1;
}

.docker-log-banner {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 2px 6px;
  font-size: 11px;
  background: rgba(214, 69, 69, 0.14);
}

.docker-log-ended {
  font-weight: 600;
}

.docker-log-error {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 固定高度滚动区：只有日志自身滚动，整页不滚（spec §3.2）。 */
.docker-log-body {
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
}

.docker-log-scroll {
  flex: 1;
  min-height: 0;
  margin: 0;
  overflow: auto;
  padding: 4px 8px;
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-all;
}

.docker-log-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  font-size: 12px;
  opacity: 0.6;
  pointer-events: none;
}
</style>
