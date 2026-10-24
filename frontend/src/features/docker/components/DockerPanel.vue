<script setup lang="ts">
// Docker 管理面板（IMPL_PLAN v2 Task P2-4 前端半）：容器列表表 + 10s 轮询
// （3 连败自动停轮询）+ 生命周期动作（start/stop/restart 直发；kill/rm 强制
// 确认）+ Logs 抽屉（docker/logs，tail 可选）+ 「在终端打开」。
// 只读采集走 docker/list；动作走 docker/action（后端负责白名单、容器 id 门、
// 只读连接拒绝、执行前审计与 Quick Sudo 回落）。会话 id 由面板自行从
// ssh/sessions/list 解析（最近的存活会话）——SideNavPanel 容器不透传 session。
// 「在终端打开」（M3 遗留 6）：面板不直接写 PTY，改为 emit 语义的 window 自定义
// 事件（SideNavPanel 不透传事件且不在本次改动范围），由 App.vue 走「填入输入行
// 不回车」通道——命令落到 shell 输入行原地，用户确认后再回车执行。
//
// 本文件只做模板编排：状态/轮询/动作在 composables/useDockerContainers，纯逻辑
// 在 lib/dockerPanelModel，四块 UI 在 DockerEnginePopover / DockerRowMenu /
// DockerConfirmDialog / DockerLogsDialog（各自携带 scoped CSS）。
import { onScopeDispose, onUnmounted, ref } from "vue";
import { Loader2, RefreshCw } from "@lucide/vue";
import { rpc } from "../../../shared/lib/rpc";
import type { DockerEngineSettings } from "../lib/dockerEngine";
import type { DockerActionName } from "../lib/dockerActions";
import {
  LOGS_TIMEOUT_MS,
  dockerExecCommand,
  dockerStateClass,
  isDockerContainerRunning,
  type DockerContainer,
  type DockerInspectSummary,
} from "../lib/dockerPanelModel";
import { createRowMenuController } from "../lib/dockerRowMenu";
import { useDockerContainers } from "../composables/useDockerContainers";
import DockerEnginePopover from "./DockerEnginePopover.vue";
import DockerRowMenu from "./DockerRowMenu.vue";
import DockerConfirmDialog from "./DockerConfirmDialog.vue";
import DockerLogsDialog from "./DockerLogsDialog.vue";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  /** 工作台当前连接的 SSH 会话（App 传入）：多台服务器同时有存活会话时，
   * “全局最新会话”猜测会把面板绑到别的机器（#168 错乱主因）；绑定一律以
   * 工作台会话为准，缺省时才回落旧启发式。 */
  sessionId?: string;
  connectionId?: string;
}>();

const {
  mode,
  engineSettings,
  engineOpen,
  engineDraft,
  toggleEngineSettings,
  applyEngineSettings,
  resolveMode,
  dockerParams,
  containers,
  available,
  needsSudo,
  loading,
  listError,
  pollPaused,
  refresh,
  retryPolling,
  actionError,
  rowBusy,
  confirmTarget,
  confirmBusy,
  requestAction,
  confirmAction,
} = useDockerContainers({
  sessionId: () => props.sessionId,
  connectionId: () => props.connectionId,
});

/** 齿轮弹层开合：只在关闭时落状态（与 reka 的 update:open 语义一致）。 */
function setEngineOpen(open: boolean): void {
  if (!open) engineOpen.value = false;
}

function setEngineDraft(draft: DockerEngineSettings): void {
  engineDraft.value = draft;
}

// —— 行操作悬浮下拉（hover 开合，状态机在 lib/dockerRowMenu）—————————————
// 触发器 hover 延迟开、移出延迟关（跨 side-offset 间隙不闪断）、click 兜底
// 切换；7 个内联按钮收纳为单项下拉，操作列不再挤占容器表宽度。
const rowMenu = createRowMenuController();
onScopeDispose(() => rowMenu.close());

/** 菜单选项统一入口：先收菜单再派发（kill/rm 仍走确认弹层）。 */
function runMenuAction(container: DockerContainer, action: DockerActionName): void {
  rowMenu.close();
  requestAction(container, action);
}

function menuOpenLogs(container: DockerContainer): void {
  rowMenu.close();
  openLogs(container);
}

function menuOpenInTerminal(container: DockerContainer): void {
  rowMenu.close();
  openInTerminal(container);
}

// —— Logs 抽屉 ———————————————————————————————————————
const logsOpen = ref(false);
const logsTarget = ref<DockerContainer | null>(null);
const logsText = ref("");
const logsMeta = ref<DockerInspectSummary | null>(null);
const logsBusy = ref(false);
const logsError = ref("");
const logsTail = ref<string>("200");

function openLogs(container: DockerContainer): void {
  logsTarget.value = container;
  logsText.value = "";
  logsMeta.value = null;
  logsError.value = "";
  logsOpen.value = true;
  void loadLogs();
}

async function loadLogs(): Promise<void> {
  const target = logsTarget.value;
  if (!target) return;
  await resolveMode();
  logsBusy.value = true;
  logsError.value = "";
  try {
    const payload = await rpc<{ logs?: string; container?: DockerInspectSummary | null }>(
      "docker/logs",
      dockerParams({ containerId: target.id, tail: Number(logsTail.value) || 200 }),
      { timeoutMs: LOGS_TIMEOUT_MS },
    );
    logsText.value = typeof payload.logs === "string" ? payload.logs : "";
    logsMeta.value = payload.container && typeof payload.container === "object" ? payload.container : null;
  } catch (cause) {
    logsError.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    logsBusy.value = false;
  }
}

function setLogsOpen(open: boolean): void {
  logsOpen.value = open;
}

/** tail 变更即重取（原 Select 的 update:model-value 语义）。 */
function setLogsTail(tail: string): void {
  logsTail.value = tail;
  void loadLogs();
}

// —— 「在终端打开」（window 事件 → App.vue 填入输入行，不回车）——————————
// 事件名与 App.vue 的监听保持一致；detail 只带命令字符串。无终端会话时由
// App.vue 侧 toast 提示先连接（面板无法感知终端状态）。
const DOCKER_OPEN_IN_TERMINAL_EVENT = "dbx:docker-open-in-terminal";

const fillRequestedId = ref("");
let fillReset: ReturnType<typeof setTimeout> | undefined;

function openInTerminal(container: DockerContainer): void {
  window.dispatchEvent(
    new CustomEvent(DOCKER_OPEN_IN_TERMINAL_EVENT, { detail: { command: dockerExecCommand(engineSettings.value.cli, container.id) } }),
  );
  fillRequestedId.value = container.id;
  if (fillReset) clearTimeout(fillReset);
  fillReset = setTimeout(() => {
    fillRequestedId.value = "";
  }, 2000);
}

onUnmounted(() => {
  if (fillReset) clearTimeout(fillReset);
});

const running = (container: DockerContainer): boolean => isDockerContainerRunning(container);
</script>

<template>
  <div class="docker-panel">
    <div class="docker-header">
      <strong>{{ props.t("docker.title") }}</strong>
      <span v-if="mode === 'local'" class="docker-source-badge">{{ props.t("docker.localSource") }}</span>
      <span v-if="available && containers.length" class="docker-count">{{ containers.length }}</span>
      <span class="docker-header-spacer" />
      <button v-if="engineSettings.cli" type="button" class="docker-engine-badge" :title="props.t('docker.engineBadge', { cli: engineSettings.cli })" :aria-label="props.t('docker.engineBadge', { cli: engineSettings.cli })" @click="toggleEngineSettings">
        {{ engineSettings.cli }}
      </button>
      <button
        v-if="pollPaused"
        type="button"
        class="docker-link-button"
        :title="props.t('docker.pollStopped')" :aria-label="props.t('docker.pollStopped')"
        @click="retryPolling"
      >
        {{ props.t("docker.refresh") }}
      </button>
      <button v-else type="button" class="icon-button" :title="props.t('docker.refresh')" :aria-label="props.t('docker.refresh')" @click="refresh">
        <Loader2 v-if="loading" class="spinning" />
        <RefreshCw v-else />
      </button>
      <DockerEnginePopover
        :t="props.t"
        :open="engineOpen"
        :draft="engineDraft"
        @update:open="setEngineOpen"
        @update:draft="setEngineDraft"
        @toggle="toggleEngineSettings"
        @apply="applyEngineSettings"
      />
    </div>

    <p v-if="pollPaused" class="docker-hint docker-hint-warn">{{ props.t("docker.pollStopped") }}</p>
    <p v-if="listError && !pollPaused" class="docker-hint docker-hint-warn" :title="listError">
      {{ props.t("docker.listFailed") }}
    </p>
    <p v-if="actionError" class="docker-hint docker-hint-error" :title="actionError">
      {{ props.t("docker.actionFailed", { error: actionError }) }}
    </p>

    <!-- 加载中 -->
    <p v-if="loading && available === null" class="docker-empty"><Loader2 class="spinning" />{{ props.t("docker.loading") }}</p>

    <!-- Docker 不可用：弱化空态，按探针结果解释原因（本机 daemon 拒绝走本地文案） -->
    <div v-else-if="available === false" class="docker-empty">
      <p class="docker-empty-title">{{ props.t("docker.unavailableTitle") }}</p>
      <p class="docker-empty-hint">
        <template v-if="!needsSudo">{{ props.t("docker.unavailableMissing") }}</template>
        <template v-else-if="mode === 'local'">{{ props.t("docker.unavailableSudoLocal") }}</template>
        <template v-else>{{ props.t("docker.unavailableSudo") }}</template>
      </p>
    </div>

    <!-- 空列表 -->
    <p v-else-if="!containers.length" class="docker-empty">{{ props.t("docker.empty") }}</p>

    <!-- 容器表 -->
    <div v-else class="docker-table-wrap">
      <table class="docker-table">
        <thead>
          <tr>
            <th>{{ props.t("docker.colName") }}</th>
            <th>{{ props.t("docker.colImage") }}</th>
            <th>{{ props.t("docker.colState") }}</th>
            <th>{{ props.t("docker.colPorts") }}</th>
            <th class="docker-col-actions">{{ props.t("docker.colActions") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="container in containers" :key="container.id">
            <td class="docker-cell-name" :title="container.id">
              <strong class="mono">{{ container.name || container.id.slice(0, 12) }}</strong>
            </td>
            <td class="docker-cell-image mono" :title="container.image">{{ container.image }}</td>
            <td class="docker-cell-state">
              <span class="docker-badge" :class="dockerStateClass(container.state)">{{ container.state }}</span>
              <small class="docker-status" :title="container.status">{{ container.status }}</small>
            </td>
            <td class="docker-cell-ports mono" :title="container.ports">{{ container.ports || "–" }}</td>
            <td class="docker-col-actions">
              <DockerRowMenu
                :t="props.t"
                :container="container"
                :menu="rowMenu"
                :fill-requested="fillRequestedId === container.id"
                :busy="rowBusy(container)"
                :running="running(container)"
                :can-open-terminal="mode === 'ssh'"
                @action="(action) => runMenuAction(container, action)"
                @logs="menuOpenLogs(container)"
                @terminal="menuOpenInTerminal(container)"
              />
            </td>
          </tr>
        </tbody>
      </table>
      <p v-if="mode === 'ssh'" class="docker-hint">{{ props.t("docker.terminalHint") }}</p>
    </div>

    <!-- kill/rm 强制确认 -->
    <DockerConfirmDialog
      :t="props.t"
      :target="confirmTarget"
      :busy="confirmBusy"
      :cli="engineSettings.cli"
      @resolve="confirmAction"
    />

    <!-- Logs 抽屉 -->
    <DockerLogsDialog
      :t="props.t"
      :open="logsOpen"
      :target="logsTarget"
      :text="logsText"
      :meta="logsMeta"
      :busy="logsBusy"
      :error="logsError"
      :tail="logsTail"
      @update:open="setLogsOpen"
      @update:tail="setLogsTail"
      @refresh="loadLogs"
    />
  </div>
</template>

<style scoped>
.docker-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
  overflow: auto;
  padding: 4px 2px;
}

.docker-header {
  display: flex;
  align-items: center;
  gap: 6px;
}

.docker-header-spacer {
  flex: 1;
}

.docker-count {
  font-size: 11px;
  opacity: 0.65;
}

.docker-source-badge {
  padding: 0 6px;
  border-radius: 8px;
  font-size: 10px;
  line-height: 16px;
  background: rgba(90, 140, 220, 0.22);
  white-space: nowrap;
}

/* 引擎设置（齿轮弹层）：紧凑表单，保存即生效。 */
.docker-engine-badge {
  border: none;
  background: none;
  padding: 0 6px;
  border-radius: 8px;
  font-size: 10px;
  line-height: 16px;
  font-family: inherit;
  color: inherit;
  cursor: pointer;
  background: rgba(90, 140, 220, 0.22);
  white-space: nowrap;
}

.docker-table-wrap {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  /* 列多超出面板宽时横向滚动兜底（面板本身可拖宽，见 App.vue 的 docker divider）。 */
  overflow-x: auto;
  /* 内容过宽时用户找不到滚动入口——全局 `*::-webkit-scrollbar{display:none}`
     把 wrap 的滚动条也藏了。这里作为例外显式恢复（同 .file-rows 模式）：
     横向溢出时滚动条可见可拖。 */
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--foreground) 25%, transparent) transparent;
}

.docker-table-wrap::-webkit-scrollbar { display: block; height: 10px; width: 10px; }
.docker-table-wrap::-webkit-scrollbar-thumb { background: color-mix(in srgb, var(--foreground) 25%, transparent); border-radius: 5px; }
.docker-table-wrap::-webkit-scrollbar-thumb:hover { background: color-mix(in srgb, var(--foreground) 40%, transparent); }
.docker-table-wrap::-webkit-scrollbar-track { background: transparent; }

.docker-table {
  width: 100%;
  /* 粘性列需要 separate：collapse 下 Firefox 的粘性单元格边框不随滚动绘制。
     行分隔线本就只画在各 cell 的 border-bottom 上，spacing 0 视觉等价。 */
  border-collapse: separate;
  border-spacing: 0;
  font-size: 12px;
}

.docker-table th {
  text-align: left;
  font-weight: 600;
  opacity: 0.6;
  padding: 2px 6px 4px;
  border-bottom: 1px solid var(--border, rgba(128, 128, 128, 0.25));
  white-space: nowrap;
}

.docker-table td {
  padding: 4px 6px;
  border-bottom: 1px solid var(--border, rgba(128, 128, 128, 0.15));
  vertical-align: top;
}

/* 列设最小宽度：窄面板下表格总宽超过容器 → wrap 横向滚动展示完整内容，
   而不是各列被省略号挤没；宽面板下仍按 max-width 截长内容。 */
.docker-cell-name {
  min-width: 110px;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 粘性首列（容器名）：横向滚动时固定靠左，滚动出的内容从其右缘滑过。
   实底取浮层同款 --popover（下层内容透出会误读成两列重叠）；右缘 1px
   分隔让滚动时的层叠关系可见。表头同规则，滚动中表头保持一致。 */
.docker-table th:first-child,
.docker-table td.docker-cell-name {
  position: sticky;
  left: 0;
  z-index: 2;
  background: var(--popover);
  box-shadow: 1px 0 0 var(--border, rgba(128, 128, 128, 0.25));
}

.docker-cell-image {
  min-width: 130px;
  max-width: 280px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  opacity: 0.8;
}

.docker-cell-state {
  min-width: 96px;
  white-space: nowrap;
}

.docker-status {
  display: block;
  max-width: 110px;
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.6;
}

/* 端口映射串很长且逗号分隔：允许换行展示全部映射（省略号会吞信息），
   列宽在 min/max 间自适应，默认面板宽度下五列 + 操作列即可全部见。 */
.docker-cell-ports {
  min-width: 130px;
  max-width: 220px;
}

.docker-col-actions {
  min-width: 44px;
  white-space: nowrap;
  text-align: right;
}

/* 粘性末列（操作按钮）：横向滚动时固定靠右，下拉触发器永不被内容推出
   视口（issue：列内容过大时下拉按钮被遮挡）。实底同浮层底色，左缘 1px
   分隔；z-index 与首列同级。 */
.docker-table th.docker-col-actions,
.docker-table td.docker-col-actions {
  position: sticky;
  right: 0;
  z-index: 2;
  background: var(--popover);
  box-shadow: -1px 0 0 var(--border, rgba(128, 128, 128, 0.25));
}

.docker-badge {
  display: inline-block;
  padding: 0 6px;
  border-radius: 8px;
  font-size: 11px;
  line-height: 16px;
  background: rgba(128, 128, 128, 0.18);
}

.docker-state-running {
  background: rgba(34, 160, 94, 0.22);
}

.docker-state-exited,
.docker-state-created,
.docker-state-removing,
.docker-state-unknown {
  background: rgba(128, 128, 128, 0.18);
}

.docker-state-paused,
.docker-state-restarting {
  background: rgba(217, 154, 35, 0.25);
}

.docker-state-dead {
  background: rgba(214, 69, 69, 0.25);
}

.docker-empty {
  opacity: 0.65;
  font-size: 12px;
  padding: 8px 4px;
}

.docker-empty-title {
  font-weight: 600;
  opacity: 0.85;
}

.docker-empty-hint {
  margin-top: 2px;
}

.docker-hint {
  font-size: 11px;
  opacity: 0.7;
}

.docker-hint-warn {
  opacity: 0.85;
}

.docker-hint-error {
  color: var(--danger, #d64545);
  opacity: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.docker-link-button {
  background: none;
  border: none;
  color: inherit;
  font-size: 12px;
  cursor: pointer;
  text-decoration: underline;
  padding: 2px;
}

.icon-button.is-copied {
  color: var(--success, #22a05e);
}

.spinning {
  animation: docker-spin 1s linear infinite;
}

@keyframes docker-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
