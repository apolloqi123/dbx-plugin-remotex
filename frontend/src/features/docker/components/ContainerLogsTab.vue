<script setup lang="ts">
// 容器日志页：真流式 + 默认跟随。`useDockerLogStream` 由本组件内部持有（props
// 契约固定为 t/container/sessionId/connectionId/canWrite），页签切到文件时
// 组件不卸载、流不断；容器切换/页面关闭/卸载才收流。
//
// 跟随规则（spec §3.2）：默认跟随；向上滚动距底 >24px 暂停自动滚动但继续接收；
// 滚回距底 ≤4px 自动恢复；按钮在跟随时点一下只关自动滚动（不断流）。
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { FOLLOW_PAUSE_PX, FOLLOW_RESUME_PX, MAX_LOG_CHARS, MAX_LOG_LINES } from "../lib/dockerLogBuffer";
import { TAIL_OPTIONS, type DockerContainer } from "../lib/dockerPanelModel";
import { useDockerLogStream, type DockerLogStreamStatus } from "../composables/useDockerLogStream";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
  /** 页签是否可见：v-show 隐藏时没有布局，重新显示后要补一次置底。 */
  visible?: boolean;
}>();

const stream = useDockerLogStream({
  sessionId: () => props.sessionId,
  connectionId: () => props.connectionId,
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
  restart,
  retrySudo,
  clear,
} = stream;

const tail = ref<string>("200");
const scroller = ref<HTMLElement>();
/** 用户是否刚点过“清空”（只用于空态提示，不参与流状态）。 */
const clearedNotice = ref(false);

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
  else if (distance <= FOLLOW_RESUME_PX) resumeFollow();
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
  void start(props.container.id, { tail: tail.value });
});

// 容器切换：先清视图再开新流（start 内部会停掉旧流）。
watch(
  () => props.container.id,
  (containerId) => {
    clearedNotice.value = false;
    clear();
    void start(containerId, { tail: tail.value });
  },
);

// 新日志到达：跟随态滚到底；暂停态不动滚动位置（内容继续进缓冲）。
watch(text, (value) => {
  if (value) clearedNotice.value = false;
  if (follow.value) scrollToBottom();
});

// v-show 隐藏期间节点没有布局，scrollToBottom 无效；重新显示时补一次置底。
watch(
  () => props.visible,
  (visible) => {
    if (visible !== false && follow.value) scrollToBottom();
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
      <pre ref="scroller" class="docker-log-scroll" @scroll="onScroll">{{ text }}</pre>
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
