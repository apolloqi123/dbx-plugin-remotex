<script setup lang="ts">
// 容器日志页：真流式 + 默认跟随。`useDockerLogStream` 由本组件内部持有（props
// 契约固定为 t/container/sessionId/connectionId/canWrite）；窗口化渲染引擎在
// `useDockerLogView`（1000 行缓冲 / 200 行 DOM 窗口 / 100ms 合帧）。页签不可见
// 时停止跟随（不再占用一条 SSH session channel，服务器 MaxSessions 较小时尤其
// 重要），切回日志页再按当前 tail 重新跟随；容器切换/页面关闭/卸载同样收流。
//
// 跟随规则（spec §3.2）：默认跟随；向上滚动距底 >24px 暂停自动滚动但继续接收；
// 滚回距底 ≤4px 自动恢复；按钮在跟随时点一下只关自动滚动（不断流）。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { MAX_LOG_CHARS, MAX_LOG_LINES } from "../lib/dockerLogBuffer";
import { TAIL_OPTIONS, type DockerContainer } from "../lib/dockerPanelModel";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import { useDockerLogStream, type DockerLogStreamStatus } from "../composables/useDockerLogStream";
import { useDockerLogView, type DockerLogView } from "../composables/useDockerLogView";
import { documentHidden, onDocumentVisibilityChange } from "../../../shared/lib/documentVisibility";

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

const scroller = ref<HTMLElement>();
let view: DockerLogView | undefined;

const stream = useDockerLogStream({
  sessionId: () => props.sessionId,
  // 引擎设置与 list/files/exec 同口径按连接附带；缺省走默认 docker/本机 daemon。
  engineParams: () => dockerEngineParams(loadDockerEngineSettings(props.connectionId || "")),
  // 增量渲染：只把新完成的整行交给窗口引擎（不重建整段日志）。
  onLines: (lines) => view?.appendLines(lines),
  onReset: () => view?.reset(),
});

const {
  hasOutput,
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

view = useDockerLogView({ scroller, follow });

const tail = ref<string>("200");
/** 用户是否刚点过“清空”（只用于空态提示，不参与流状态）。 */
const clearedNotice = ref(false);
/** 流是否因页签隐藏而停止：只有这种停止才在重新可见时自动重开。 */
let pausedByHidden = false;
/** 流是否因 DBX 窗口隐藏而停止（后台不再养远端 docker logs 客户端）。 */
let pausedByDocument = false;
/** 窗口隐藏后的收流宽限：短暂切走不重启，久挂后台才停掉远端进程。 */
const DOCUMENT_PAUSE_GRACE_MS = 10_000;
let documentPauseTimer = 0;

/** 截断提示文案；数值直接取自缓冲上限，避免两处漂移。 */
const truncatedHint = computed(() =>
  props.t("docker.logsTruncated", { lines: MAX_LOG_LINES, chars: MAX_LOG_CHARS / 1_000_000 }),
);

function scrollToBottom(): void {
  void nextTick(() => view?.scrollToBottom());
}

function startStream(): void {
  void start(props.container.id, { tail: tail.value });
}

function clearDocumentPauseTimer(): void {
  if (documentPauseTimer) {
    window.clearTimeout(documentPauseTimer);
    documentPauseTimer = 0;
  }
}

// DBX 窗口隐藏超过宽限期就停流：远端 `docker logs -f` 是服务器上的真实进程，
// 后台一直跟随会持续占用 CPU（尤其容器日志量大时）。复现时按当前 tail 重开。
const stopVisibilityWatch = onDocumentVisibilityChange((hidden) => {
  if (hidden) {
    clearDocumentPauseTimer();
    if (status.value !== "streaming" && status.value !== "starting") return;
    documentPauseTimer = window.setTimeout(() => {
      documentPauseTimer = 0;
      if (!documentHidden() || pausedByDocument) return;
      pausedByDocument = true;
      void stop();
    }, DOCUMENT_PAUSE_GRACE_MS);
    return;
  }
  clearDocumentPauseTimer();
  if (!pausedByDocument) return;
  pausedByDocument = false;
  // 页签本身仍不可见时不动：等页签显隐 watcher 恢复，避免后台重开。
  if (props.visible === false) return;
  clearedNotice.value = false;
  clear();
  startStream();
});

onBeforeUnmount(() => {
  clearDocumentPauseTimer();
  stopVisibilityWatch();
});

/** 清空按钮：只清视图（不断流），并在空态提示“已清空”。 */
function handleClear(): void {
  clear();
  clearedNotice.value = true;
}

function resumeFollow(): void {
  view?.resume();
}

function pauseFollow(): void {
  view?.pause();
}

/** 按钮：跟随时点一下只停自动滚动；暂停时恢复并跳到最新。 */
function toggleFollow(): void {
  if (follow.value) pauseFollow();
  else resumeFollow();
}

function onScroll(): void {
  view?.handleScroll();
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
/** 结束横幅的语义分档：正常退出（exit 0）只是“流结束”，不是错误。
    红色底只留给真实失败（流错误 / 非零退出 / 错误文案），否则整块红色会
    把一条普通的容器重启读成告警。 */
const streamEndedWithError = computed(
  () => status.value === "error" || Boolean(error.value) || (exitCode.value != null && exitCode.value !== 0),
);
/** 启动中还没有任何输出是正常的：不要显示“无输出”空态。 */
const showEmptyState = computed(() => !hasOutput.value && status.value !== "starting");

onMounted(() => {
  if (props.visible === false) {
    pausedByHidden = true;
    return;
  }
  if (documentHidden()) {
    pausedByDocument = true;
    return;
  }
  startStream();
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
    if (documentHidden()) {
      pausedByDocument = true;
      return;
    }
    pausedByHidden = false;
    void start(containerId, { tail: tail.value });
  },
);

// 有输出即清掉「已清空」提示；滚动跟随在窗口引擎追加后同步完成。
watch(hasOutput, (value) => {
  if (value) clearedNotice.value = false;
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
    if (pausedByHidden || pausedByDocument) {
      pausedByHidden = false;
      pausedByDocument = false;
      clearedNotice.value = false;
      clear();
      startStream();
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
    <div v-if="endedBanner(status) || needsSudo" class="docker-log-banner" :class="{ 'is-error': streamEndedWithError, 'is-warning': !streamEndedWithError && needsSudo }" data-role="stream-end">
      <span class="docker-log-ended">{{ t("docker.streamEnded") }}</span>
      <span v-if="exitCode !== null" class="docker-log-exit" :class="{ 'is-failed': exitCode !== 0 }">exit {{ exitCode }}</span>
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
      <pre ref="scroller" class="docker-log-scroll" @scroll="onScroll"></pre>
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
  min-height: 30px;
  align-items: center;
  gap: 7px;
  padding: 3px 8px;
  font-size: var(--text-xs);
}

.docker-log-tail {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  white-space: nowrap;
  color: var(--muted-foreground);
}

.docker-log-tail select {
  height: 22px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  padding: 0 5px;
  background: var(--card-surface-inset);
  color: var(--foreground);
  font-size: var(--text-xs);
}

.docker-log-follow,
.docker-log-clear,
.docker-log-older,
.docker-log-sudo,
.docker-log-restart {
  height: 22px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  padding: 0 9px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: var(--text-xs);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--transition-fast), border-color var(--transition-fast), color var(--transition-fast);
}

.docker-log-follow:hover,
.docker-log-clear:hover,
.docker-log-older:hover,
.docker-log-restart:hover {
  border-color: color-mix(in srgb, var(--foreground) 24%, transparent);
  background: color-mix(in srgb, var(--foreground) 6%, transparent);
  color: var(--foreground);
}

.docker-log-follow.is-active {
  border-color: color-mix(in srgb, var(--primary) 48%, transparent);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  color: var(--primary);
  font-weight: 600;
}

.docker-log-follow:disabled,
.docker-log-sudo:disabled {
  cursor: default;
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
  gap: 7px;
  border-top: 1px solid var(--card-border);
  border-bottom: 1px solid var(--card-border);
  padding: 4px 8px;
  background: color-mix(in srgb, var(--muted) 42%, transparent);
  color: var(--muted-foreground);
  font-size: var(--text-xs);
}
/* 状态点：正常结束（exit 0）= 中性灰点；失败/非零退出 = 红点；需要 sudo =
   琥珀点。颜色只在点与退出码上，整条背景保持安静。 */
.docker-log-banner::before {
  content: "";
  width: 6px;
  height: 6px;
  flex: 0 0 6px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--muted-foreground) 60%, transparent);
}
.docker-log-banner.is-warning { background: color-mix(in srgb, var(--warning) 10%, transparent); color: color-mix(in srgb, var(--foreground) 82%, transparent); }
.docker-log-banner.is-warning::before { background: var(--warning); }
.docker-log-banner.is-error { background: color-mix(in srgb, var(--destructive) 11%, transparent); color: color-mix(in srgb, var(--foreground) 88%, transparent); }
.docker-log-banner.is-error::before { background: var(--destructive); }
.docker-log-ended { font-weight: 600; color: var(--foreground); }
.docker-log-exit {
  border: 1px solid var(--card-border);
  border-radius: var(--radius-xs);
  padding: 0 5px;
  font-family: var(--terminal-font-family);
  font-size: 9.5px;
  font-variant-numeric: tabular-nums;
}
.docker-log-exit.is-failed {
  border-color: color-mix(in srgb, var(--destructive) 42%, transparent);
  color: var(--destructive);
}

.docker-log-error {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--destructive);
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
