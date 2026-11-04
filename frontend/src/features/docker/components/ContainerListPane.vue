<script setup lang="ts">
// 容器列表列（Task 7，自 DockerPanel 拆出）：10s 轮询（3 连败自动停）+
// 生命周期动作（start/stop/restart 直发；kill/rm 强制确认）+ 引擎设置 +
// 「在终端打开」。列表还持有本列的选中规则：首次加载后选中第一个 running
// 容器（否则第一个），轮询后选中项消失则按同一规则回退；选中签名（id +
// state/status/image/ports）变化或列表清空（上抛 null）时向上 emit，行点击
// 额外带 explicit 标记（父组件据此在详情收起后以日志重开）。
//
// 状态/轮询/动作在 composables/useDockerContainers，纯逻辑在 lib/dockerPanelModel，
// 三个 UI 块在 DockerEnginePopover / DockerRowMenu / DockerConfirmDialog。
import { computed, onScopeDispose, onUnmounted, ref, watch } from "vue";
import { ChevronDown, Loader2, RefreshCw, X } from "@lucide/vue";
import type { DockerEngineSettings } from "../lib/dockerEngine";
import type { DockerActionName } from "../lib/dockerActions";
import {
  dockerExecCommand,
  dockerStateClass,
  filterContainers,
  isDockerContainerRunning,
  searchContainers,
  type ContainerFilter,
  type DockerContainer,
} from "../lib/dockerPanelModel";
import { createRowMenuController } from "../lib/dockerRowMenu";
import { useDockerContainers } from "../composables/useDockerContainers";
import DockerEnginePopover from "./DockerEnginePopover.vue";
import DockerRowMenu from "./DockerRowMenu.vue";
import DockerConfirmDialog from "./DockerConfirmDialog.vue";
import DockerSearchField from "./DockerSearchField.vue";
import DockerWhaleLogo from "./DockerWhaleLogo.vue";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  /** 工作台当前连接的 SSH 会话；缺省时回落“最近存活会话 → 本机 daemon”。 */
  sessionId?: string;
  connectionId?: string;
}>();

const emit = defineEmits<{
  /** 选中快照（id/state/status/image/ports 变化时重发）；`null` = 列表已清空，
   *  父组件据此收起详情。`explicit` 为 true 表示用户行点击/行菜单，父组件据此
   *  在详情收起后以日志重开；自动选中（首轮/轮询/刷新）不带该标记。 */
  (e: "select", container: DockerContainer | null, explicit?: boolean): void;
  /** 行菜单「Logs」：打开该容器的日志详情。 */
  (e: "logs", container: DockerContainer): void;
  /** 行菜单「Files」：打开该容器的文件详情。 */
  (e: "files", container: DockerContainer): void;
  /** 页头 X：关闭整个容器管理分屏（详情收起由详情页头的 X 负责）。 */
  (e: "close"): void;
}>();

const {
  mode,
  engineSettings,
  engineOpen,
  engineDraft,
  toggleEngineSettings,
  applyEngineSettings,
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

// —— 状态筛选：默认只显示 running + 5 分钟内停止的容器 —————————————
const containerFilter = ref<ContainerFilter>("recent");

/** 搜索词（页头输入框）：与状态筛选叠加，空白分词 AND，标识字段支持子序列模糊。 */
const searchQuery = ref("");

const filteredContainers = computed(() =>
  searchContainers(filterContainers(containers.value, containerFilter.value), searchQuery.value),
);

const searchActive = computed(() => searchQuery.value.trim().length > 0);

/** 齿轮弹层开合：只在关闭时落状态（与 reka 的 update:open 语义一致）。 */
function setEngineOpen(open: boolean): void {
  if (!open) engineOpen.value = false;
}

function setEngineDraft(draft: DockerEngineSettings): void {
  engineDraft.value = draft;
}

// —— 选中规则：首个 running，否则首个；消失则回退同一规则 —————————————
const selectedId = ref("");
/** 最近一次上抛的选中签名（id + 身份字段）。轮询重复命中同一容器时不重复
 *  emit；但 state/status/image/ports 变化必须重发，否则父组件手里一直是首次
 *  选中的旧快照（启停后详情页头/页签看到过期状态）。 */
let lastSignature = "";

function identitySignature(container: DockerContainer | null): string {
  return container
    ? [container.id, container.state, container.status, container.image, container.ports].join("\u0000")
    : "";
}

/** `force`：行点击总是上抛（即使签名未变）并带上显式标记，父组件可据此在详情
 *  收起后重开；自动选中（首轮/轮询）只上抛签名变化且不带该标记。 */
function emitSelection(container: DockerContainer | null, force = false): void {
  const signature = identitySignature(container);
  if (!force && signature === lastSignature) return;
  lastSignature = signature;
  // 自动选中沿用旧的两参数形态（explicit 缺省 undefined），行点击才带 true。
  if (force) emit("select", container, true);
  else emit("select", container);
}

function applySelection(rows: DockerContainer[]): void {
  if (!rows.length) {
    selectedId.value = "";
    // 空列表也要上抛：删掉最后一个容器后父组件必须收起详情。
    emitSelection(null);
    return;
  }
  const current = rows.find((container) => container.id === selectedId.value)
    ?? rows.find((container) => isDockerContainerRunning(container))
    ?? rows[0];
  selectedId.value = current.id;
  emitSelection(current);
}

watch(filteredContainers, (rows) => applySelection(rows), { flush: "post" });
// 连接切换：旧连接的选中项/去重标记立即失效；等新列表落地（watch containers）
// 再按规则选中，避免把上一台机器的容器重新抛给父组件。
watch([() => props.sessionId, () => props.connectionId], () => {
  selectedId.value = "";
  lastSignature = "";
  // 立即按新目标重采一遍；否则首轮选中要等下一次 10s 轮询，
  // 面板会长时间停在空列表/旧容器上。
  void refreshList();
});

function selectRow(container: DockerContainer): void {
  selectedId.value = container.id;
  emitSelection(container, true);
}

// —— 行操作悬浮下拉（hover 开合，状态机在 lib/dockerRowMenu）———————————
const rowMenu = createRowMenuController();
onScopeDispose(() => rowMenu.close());

/** 菜单选项统一入口：先收菜单再派发（kill/rm 仍走确认弹层）。 */
function runMenuAction(container: DockerContainer, action: DockerActionName): void {
  rowMenu.close();
  requestAction(container, action);
}

function menuOpenLogs(container: DockerContainer): void {
  rowMenu.close();
  selectedId.value = container.id;
  emitSelection(container, true);
  emit("logs", container);
}

function menuOpenFiles(container: DockerContainer): void {
  rowMenu.close();
  selectedId.value = container.id;
  emitSelection(container, true);
  emit("files", container);
}

function menuOpenInTerminal(container: DockerContainer): void {
  rowMenu.close();
  openInTerminal(container);
}

/** 显式刷新：拉列表，选中项变化（含清空/状态变化）经 watch → select 上抛。 */
function refreshList(): Promise<void> {
  return refresh();
}

function retryList(): Promise<void> {
  return retryPolling();
}

// —— 「在终端打开」（window 事件 → App.vue 填入输入行，不回车）——————————
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
  <div class="container-list-pane">
    <header class="container-list-head">
      <span class="ui-icon-tile ui-tone-blue container-list-logo"><DockerWhaleLogo /></span>
      <strong class="container-list-title">{{ props.t("docker.title") }}</strong>
      <span v-if="mode === 'local'" class="docker-source-badge">{{ props.t("docker.localSource") }}</span>
      <span v-if="available && filteredContainers.length" class="docker-count">{{ filteredContainers.length }}</span>
      <span class="container-list-spacer" />
      <DockerSearchField
        :model-value="searchQuery"
        :placeholder="props.t('docker.searchPlaceholder')"
        :clear-label="props.t('docker.searchClear')"
        @update:model-value="searchQuery = $event"
      />
      <div class="container-filter-wrap">
        <select
          v-model="containerFilter"
          class="container-filter-select"
          :aria-label="props.t('docker.filterLabel')"
        >
          <option value="recent">{{ props.t("docker.filterRecent") }}</option>
          <option value="running">{{ props.t("docker.filterRunning") }}</option>
          <option value="stopped">{{ props.t("docker.filterStopped") }}</option>
          <option value="all">{{ props.t("docker.filterAll") }}</option>
        </select>
        <ChevronDown class="container-filter-chevron" />
      </div>
      <button
        v-if="engineSettings.cli"
        type="button"
        class="docker-engine-badge"
        :title="props.t('docker.engineBadge', { cli: engineSettings.cli })"
        :aria-label="props.t('docker.engineBadge', { cli: engineSettings.cli })"
        @click="toggleEngineSettings"
      >
        {{ engineSettings.cli }}
      </button>
      <button
        v-if="pollPaused"
        type="button"
        class="docker-link-button"
        :title="props.t('docker.pollStopped')"
        :aria-label="props.t('docker.pollStopped')"
        @click="retryList"
      >
        {{ props.t("docker.refresh") }}
      </button>
      <button
        v-else
        type="button"
        class="icon-button compact container-refresh"
        :title="props.t('docker.refresh')"
        :aria-label="props.t('docker.refresh')"
        @click="refreshList"
      >
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
      <button
        type="button"
        class="icon-button compact container-manager-close"
        :title="props.t('docker.close')"
        :aria-label="props.t('docker.close')"
        @click="emit('close')"
      >
        <X />
      </button>
    </header>

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

    <!-- 空列表（有搜索词时提示“无匹配”，并保留清除入口） -->
    <p v-else-if="!filteredContainers.length" class="docker-empty">
      <template v-if="searchActive">{{ props.t("docker.searchEmpty", { query: searchQuery.trim() }) }}</template>
      <template v-else>{{ props.t("docker.empty") }}</template>
    </p>

    <!-- 容器行：名称 + 状态徽标在上，镜像/端口/状态文案在下（窄列可读） -->
    <ul v-else class="container-rows" role="listbox" :aria-label="props.t('docker.title')">
      <li v-for="container in filteredContainers" :key="container.id" class="container-row-item">
        <button
          type="button"
          role="option"
          class="container-row"
          :class="{ 'is-selected': selectedId === container.id }"
          :aria-selected="selectedId === container.id"
          :data-container-id="container.id"
          @click="selectRow(container)"
        >
          <span class="container-row-line">
            <strong class="mono container-row-name" :title="container.id">{{ container.name || container.id.slice(0, 12) }}</strong>
            <span class="docker-badge" :class="dockerStateClass(container.state)">{{ container.state }}</span>
          </span>
          <span class="mono container-row-meta" :title="container.image">{{ container.image }}</span>
          <span v-if="container.ports" class="mono container-row-meta" :title="container.ports">{{ container.ports }}</span>
          <small v-if="container.status" class="container-row-status" :title="container.status">{{ container.status }}</small>
        </button>
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
          @files="menuOpenFiles(container)"
          @terminal="menuOpenInTerminal(container)"
        />
      </li>
    </ul>

    <!-- kill/rm 强制确认 -->
    <DockerConfirmDialog
      :t="props.t"
      :target="confirmTarget"
      :busy="confirmBusy"
      :cli="engineSettings.cli"
      @resolve="confirmAction"
    />
  </div>
</template>

<style scoped>
.container-list-pane {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  /* 容器查询基准：超窄列（拖窄或分屏）里优先保住刷新/引擎/关闭按钮。 */
  container-type: inline-size;
}

.container-list-head {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 2px 4px;
}

.container-list-title {
  font-size: 12px;
  white-space: nowrap;
}

.container-list-spacer {
  flex: 1;
}

.docker-count {
  font-size: 11px;
  opacity: 0.65;
}

.container-filter-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.container-filter-select {
  appearance: none;
  height: 20px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 0 18px 0 6px;
  background: var(--background);
  color: var(--foreground);
  font-size: 10px;
  cursor: pointer;
  outline: none;
}

.container-filter-select:focus {
  border-color: color-mix(in srgb, var(--primary) 55%, var(--border));
}

.container-filter-chevron {
  position: absolute;
  right: 4px;
  width: 10px;
  height: 10px;
  pointer-events: none;
  color: var(--muted-foreground);
}

.docker-source-badge,
.docker-engine-badge {
  padding: 0 6px;
  border-radius: 8px;
  font-size: 10px;
  line-height: 16px;
  background: rgba(90, 140, 220, 0.22);
  white-space: nowrap;
}

.docker-engine-badge {
  border: none;
  font-family: inherit;
  color: inherit;
  cursor: pointer;
}

.container-rows {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  min-height: 0;
  margin: 0;
  padding: 0;
  overflow: auto;
  list-style: none;
  border-top: 1px solid var(--border, rgba(128, 128, 128, 0.25));
}

.container-row-item {
  display: flex;
  align-items: flex-start;
  gap: 2px;
  border-bottom: 1px solid var(--border, rgba(128, 128, 128, 0.15));
}

.container-row-item:hover {
  background: color-mix(in srgb, var(--foreground) 5%, transparent);
}

.container-row {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  border: none;
  background: none;
  color: inherit;
  text-align: left;
  padding: 2px 4px;
  cursor: pointer;
  font: inherit;
}

.container-row.is-selected {
  background: color-mix(in srgb, var(--primary) 14%, transparent);
}

.container-row-line {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
}

.container-row-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10px;
}

.container-row-meta,
.container-row-status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 9px;
  opacity: 0.65;
}

.container-row-status {
  font-size: 9px;
  opacity: 0.55;
}

.docker-badge {
  display: inline-block;
  flex: 0 0 auto;
  padding: 0 4px;
  border-radius: 6px;
  font-size: 9px;
  line-height: 12px;
  background: rgba(128, 128, 128, 0.18);
}

.docker-state-running {
  background: rgba(34, 160, 94, 0.22);
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
  flex: 0 0 auto;
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

.spinning {
  animation: docker-spin 1s linear infinite;
}

@keyframes docker-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
