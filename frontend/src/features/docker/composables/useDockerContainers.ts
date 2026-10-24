// Docker 面板的状态与行为：会话/模式解析、引擎连接设置、10s 轮询（3 连败
// 自动停）、生命周期动作与 kill/rm 确认。面板组件只做模板编排。
//
// resolveMode / dockerParams 同时被 logs 调用方使用：这里作为返回成员导出，
// 不在别处重复推导（模式不同 → target/sessionId 不同，重复实现必然漂移）。
import { onMounted, onScopeDispose, onUnmounted, ref, watch } from "vue";
import { normalizeBatchTargets } from "../../terminal/lib/batchSend";
import { rpc } from "../../../shared/lib/rpc";
import {
  dockerEngineParams,
  loadDockerEngineSettings,
  saveDockerEngineSettings,
  validateDockerEngineSettings,
  type DockerEngineSettings,
} from "../lib/dockerEngine";
import {
  confirmDockerAction,
  requestDockerAction,
  type DockerActionDispatch,
  type DockerActionName,
} from "../lib/dockerActions";
import {
  ACTION_TIMEOUT_MS,
  LIST_TIMEOUT_MS,
  POLL_MAX_FAILURES,
  POLL_MS,
  dockerListPayload,
  type DockerConfirmTarget,
  type DockerContainer,
} from "../lib/dockerPanelModel";

// —— 会话解析（SideNavPanel 不透传 session，面板自取最近存活会话）———————
// 无存活 SSH 会话时回落 target:"local"——查询 sidecar 所在机器的 docker
// daemon（Docker Desktop/OrbStack），面板不再是「没连服务器就空转」。
export type DockerPanelMode = "ssh" | "local";

export interface UseDockerContainersOptions {
  /** 工作台当前连接的 SSH 会话（App 传入的 prop 读取器）：调用时取值，
   *  多台服务器同时有存活会话时绑定仍以工作台为准（#168）。 */
  sessionId?: () => string | undefined;
  connectionId?: () => string | undefined;
}

export function useDockerContainers(options: UseDockerContainersOptions = {}) {
  const readSessionId = options.sessionId ?? (() => undefined);
  const readConnectionId = options.connectionId ?? (() => undefined);

  const sessionId = ref("");
  const mode = ref<DockerPanelMode>("ssh");

  // —— 引擎连接设置（Podman / 自定义 socket / tcp host）———————————————————
  // 按连接持久化（connectionId 为键，local 模式 "local"）；随每次 docker 家族
  // invoke 以 cli/socket/host 附加参数下发，sidecar 负责归一与最终校验。
  const connectionKey = ref("");
  const engineSettings = ref<DockerEngineSettings>({ ...loadDockerEngineSettings("") });
  const engineOpen = ref(false);
  const engineDraft = ref<DockerEngineSettings>({ cli: "", socket: "", host: "" });

  function toggleEngineSettings(): void {
    engineOpen.value = !engineOpen.value;
    if (engineOpen.value) engineDraft.value = { ...engineSettings.value };
  }

  /** 保存即生效并重采列表；非法草稿（红线不满足）不落盘。 */
  function applyEngineSettings(): void {
    const { settings, errors } = validateDockerEngineSettings(engineDraft.value);
    if (errors.cli || errors.endpoints) return;
    engineSettings.value = settings;
    saveDockerEngineSettings(connectionKey.value, settings);
    void refresh();
  }

  async function resolveMode(): Promise<DockerPanelMode> {
    // 工作台绑定会话优先（#168）：有 sessionId prop 就严格绑定本工作台的
    // 连接，绝不按“全局最新会话”抢别人的机器；连接变化时下一次轮询重绑。
    const boundSessionId = readSessionId();
    if (boundSessionId) {
      if (boundSessionId !== sessionId.value) {
        sessionId.value = boundSessionId;
        mode.value = "ssh";
        adoptConnection(readConnectionId() || "");
      }
      return "ssh";
    }
    if (mode.value === "ssh" && sessionId.value) return "ssh";
    try {
      const response = await rpc<{ sessions: unknown }>("ssh/sessions/list");
      const targets = normalizeBatchTargets(response.sessions)
        .filter((target) => target.connected !== false)
        .sort((a, b) => (b.createdAt ?? 0) - (a.createdAt ?? 0));
      if (targets.length) {
        sessionId.value = targets[0].sessionId;
        mode.value = "ssh";
        adoptConnection(targets[0].connectionId);
        return "ssh";
      }
    } catch {
      // 会话枚举失败（旧 sidecar 等）同样回落本机探测。
    }
    mode.value = "local";
    adoptConnection("");
    return "local";
  }

  /** 切换持久化键并装载该连接的引擎设置（新连接不继承上一台主机配置）。 */
  function adoptConnection(connectionId: string): void {
    const nextKey = connectionId || "local";
    if (nextKey === connectionKey.value) return;
    connectionKey.value = nextKey;
    engineSettings.value = loadDockerEngineSettings(nextKey);
    if (engineOpen.value) engineDraft.value = { ...engineSettings.value };
  }

  /** 按当前模式组装 docker 家族参数：local 不需要 sessionId；引擎设置随行。 */
  function dockerParams(extra: Record<string, unknown> = {}): Record<string, unknown> {
    const base =
      mode.value === "local"
        ? { target: "local" as const, ...extra }
        : { sessionId: sessionId.value, ...extra };
    return { ...base, ...dockerEngineParams(engineSettings.value) };
  }

  // —— 列表 + 轮询（10s；3 连败停轮询，弱化空态）——————————————
  const containers = ref<DockerContainer[]>([]);
  const available = ref<boolean | null>(null);
  const needsSudo = ref(false);
  const loading = ref(false);
  const listError = ref("");
  const pollPaused = ref(false);

  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let consecutiveFailures = 0;

  async function refresh(): Promise<void> {
    if (loading.value) return;
    await resolveMode();
    loading.value = true;
    try {
      const payload = dockerListPayload(
        await rpc<unknown>(
          "docker/list",
          dockerParams(),
          { timeoutMs: LIST_TIMEOUT_MS },
        ),
      );
      containers.value = payload.containers;
      available.value = payload.available;
      needsSudo.value = payload.needsSudo;
      listError.value = "";
      consecutiveFailures = 0;
      pollPaused.value = false;
    } catch (cause) {
      consecutiveFailures += 1;
      listError.value = cause instanceof Error ? cause.message : String(cause);
      if (consecutiveFailures >= POLL_MAX_FAILURES) stopPolling();
    } finally {
      loading.value = false;
    }
  }

  function startPolling(): void {
    stopPolling();
    pollTimer = setInterval(() => void refresh(), POLL_MS);
  }

  function stopPolling(): void {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = undefined;
    if (consecutiveFailures >= POLL_MAX_FAILURES) pollPaused.value = true;
  }

  async function retryPolling(): Promise<void> {
    consecutiveFailures = 0;
    pollPaused.value = false;
    await refresh();
    if (pollPaused.value) return;
    startPolling();
  }

  onMounted(() => {
    void (async () => {
      await refresh();
      if (!pollPaused.value) startPolling();
    })();
  });

  onUnmounted(stopPolling);

  // —— 生命周期动作（kill/rm 先确认）———————————————————————
  const busyContainerId = ref("");
  const actionError = ref("");

  function rowBusy(container: DockerContainer): boolean {
    return busyContainerId.value === container.id;
  }

  const confirmTarget = ref<DockerConfirmTarget | null>(null);
  const confirmBusy = ref(false);

  function requestAction(container: DockerContainer, action: DockerActionName): void {
    actionError.value = "";
    const transition = requestDockerAction({ id: container.id, name: container.name || container.id }, action);
    confirmTarget.value = transition.confirmation;
    if (transition.dispatch) void runAction(transition.dispatch);
  }

  async function runAction(dispatch: DockerActionDispatch): Promise<void> {
    await resolveMode();
    busyContainerId.value = dispatch.id;
    try {
      await rpc<{ success: boolean; output: string }>(
        "docker/action",
        dockerParams({ containerId: dispatch.id, action: dispatch.action }),
        { timeoutMs: ACTION_TIMEOUT_MS },
      );
      await refresh();
    } catch (cause) {
      actionError.value = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busyContainerId.value = "";
    }
  }

  async function confirmAction(accepted: boolean): Promise<void> {
    const transition = confirmDockerAction(confirmTarget.value, accepted);
    confirmTarget.value = transition.confirmation;
    if (!transition.dispatch) return;
    confirmBusy.value = true;
    try {
      await runAction(transition.dispatch);
    } finally {
      confirmBusy.value = false;
    }
  }

  // kill/rm 确认弹窗的 Esc 取消：面板内状态不在 App 的分层 Esc 链里，而
  // DialogContent 的 @escape-key-down.prevent 又挡掉了 reka 自带关闭（Esc 此前
  // 完全失效）。document 捕获阶段监听先于 App 冒泡链与 reka 触发，打开时挂、
  // 关闭即卸；stopPropagation 防止同一次 Esc 再关掉底层弹层。
  function onConfirmEsc(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    void confirmAction(false);
  }
  watch(confirmTarget, (target) => {
    if (target) document.addEventListener("keydown", onConfirmEsc, true);
    else document.removeEventListener("keydown", onConfirmEsc, true);
  });
  onScopeDispose(() => document.removeEventListener("keydown", onConfirmEsc, true));

  return {
    sessionId,
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
  };
}
