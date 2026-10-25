import { computed, ref } from "vue";
import { WORKBENCH_ID } from "../../../../shared/lib/pluginIds";

type LocalShell = { program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean };
type DockConnection = { id: string; name: string; providerId: string; connectionType?: string; readOnly?: boolean };

/**
 * 本地终端偏好（sidecar preferences.json 持久化；iframe 沙箱无 localStorage）
 * 与 shell 发现/选择器：
 * - shell 空串 = 跟随自动探测；integration 缺省开；
 * - 启动目录/环境变量保留未填完的草稿行作为 UI 模型，持久化与 spawn 载荷
 *   都取净化后的副本；
 * - 发现清单两个入口（设置菜单 / dock「+」面板菜单）共用，失败静默降级为
 *   仅自动探测入口，loading 守卫防重入。
 *
 * 本地终端进程生命周期（start/close）不在本 composable：它牵动终端输出管线
 * 的序号/补发状态，仍由 App 承载；这里只经回调触发「换 shell 重启 / 请求打开」。
 */
export function useLocalShellPreferences(options: {
  isLocalMode: () => boolean;
  isLocalShellRestored: () => boolean;
  restartLocalTerminal: (shellOverride?: string) => void;
  requestLocalTerminal: (shellOverride?: string) => void;
  closeSessionMenu: () => void;
  setShellPickOpen: (open: boolean) => void;
}) {
  const {
    isLocalMode, isLocalShellRestored, restartLocalTerminal, requestLocalTerminal,
    closeSessionMenu, setShellPickOpen,
  } = options;

  // shell 空串 = 跟随自动探测；integration 缺省开。
  const localShellPref = ref("");
  const localShellIntegrationPref = ref(true);
  // 本地 Shell 启动目录/环境变量（localShellCwd/localShellEnv 偏好）。envPref
  // 保留未填完的草稿行作为 UI 模型；持久化与 spawn 载荷都取净化后的副本。
  const localShellCwdPref = ref("");
  const localShellEnvPref = ref<Array<{ name: string; value: string }>>([]);
  // 净化视图：名字非空的行（与 sidecar 的 sanitize_env_pair 边界对齐的前端版）。
  const localShellEnvClean = computed(() =>
    localShellEnvPref.value
      .map((pair) => ({ name: pair.name.trim(), value: pair.value }))
      .filter((pair) => pair.name),
  );

  const localShells = ref<LocalShell[]>([]);
  const localShellsLoading = ref(false);
  // 当前偏好 shell 是否支持注入（ksh/csh/cmd 等裸 shell 灰掉开关）。
  const selectedShellInjectable = computed<boolean | undefined>(() => {
    if (!localShellPref.value) return undefined;
    return localShells.value.find((entry) => entry.program === localShellPref.value)?.injectable;
  });

  // Dock panel "+": opens another dock entry with the selected shell type via the bridge openWorkbench
  // (the host owns the surface: inside a panel webview -> a new dock entry; inside a tab -> a new tab).
  const localShellSurfaceOpen = ref(false);
  // host.listConnections (PR-A4 generic extension point): a read-only, secret-free list of the plugin's own connections,
  // for in-panel connection switching; hosts without it degrade to a hidden connection section.
  const dockConnections = ref<DockConnection[]>([]);

  // 会话菜单「选择 Shell 打开」：一次性 override（dock per-shell 语义），不动
  // localShell 默认偏好。已在本地模式（运行中或恢复态）= 换 shell 重启；SSH 在
  // 连时走既有确认流（override 经 pendingLocalShellOverride 透传）。
  function pickLocalShell(program?: string) {
    closeSessionMenu();
    setShellPickOpen(false);
    if (isLocalShellRestored() || isLocalMode()) {
      restartLocalTerminal(program);
      return;
    }
    requestLocalTerminal(program);
  }

  // 下拉内切到 shell 选择视图：同弹层视图切换，菜单不关，顺带懒加载 shell 列表。
  function openSessionShellPick() {
    setShellPickOpen(true);
    void openLocalShellPrefs();
  }

  // —— shell 选择器：多平台 shell 发现 + 偏好（VS Code terminal profiles 简化版）——
  // 发现清单两个入口（设置菜单 / dock「+」面板菜单）共用；失败静默降级为仅
  // 自动探测入口，loading 守卫防重入。
  async function loadLocalShells() {
    if (localShellsLoading.value || localShells.value.length) return;
    localShellsLoading.value = true;
    try {
      const result = await window.dbxPlugin.invoke<{ shells: LocalShell[] }>("local/shells/list", {}, { timeoutMs: 10_000 });
      localShells.value = result.shells || [];
    } catch {
      // 旧 sidecar 无发现方法：菜单退化为仅注入开关（start 仍走自动探测）。
    } finally {
      localShellsLoading.value = false;
    }
  }

  async function openLocalShellPrefs() {
    await loadLocalShells();
  }

  // 嵌套在会话弹层内：只切换内层 shell 菜单，不关闭父弹层（reka 嵌套层自行
  // 处理外点/焦点）；其它工具条 toggle 的互斥统一由 closeToolbarPopovers 负责。
  async function openLocalShellSurfaceMenu() {
    if (localShellSurfaceOpen.value) {
      localShellSurfaceOpen.value = false;
      return;
    }
    localShellSurfaceOpen.value = true;
    await loadLocalShells();
    try {
      const listed = await window.dbxPlugin.request<{ connections?: DockConnection[] }>("host.listConnections");
      dockConnections.value = listed?.connections ?? [];
    } catch {
      // Legacy hosts without the listConnections extension point: the connection section stays hidden.
      dockConnections.value = [];
    }
  }

  function openConnectionSurface(connection: DockConnection) {
    localShellSurfaceOpen.value = false;
    void window.dbxPlugin.openWorkbench?.(
      WORKBENCH_ID,
      {
        connectionId: connection.id,
        providerId: connection.providerId,
        connectionType: connection.connectionType,
        connection: { id: connection.id, name: connection.name, readOnly: connection.readOnly === true },
      },
      { forceNew: true },
    );
  }

  function openLocalShellSurface(program?: string) {
    localShellSurfaceOpen.value = false;
    void window.dbxPlugin.openWorkbench?.(
      WORKBENCH_ID,
      { plugin: { mode: "local-terminal", ...(program ? { shell: program } : {}) } },
      { forceNew: true },
    );
  }

  async function setLocalShellPref(program: string) {
    localShellPref.value = program;
    try {
      await window.dbxPlugin.invoke("local/preferences/set", { localShell: program });
    } catch {
      // 旧 sidecar：会话内存态兜底。
    }
  }

  async function setLocalShellIntegrationPref(enabled: boolean) {
    localShellIntegrationPref.value = enabled;
    try {
      await window.dbxPlugin.invoke("local/preferences/set", { localShellIntegration: enabled });
    } catch {
      // 旧 sidecar：会话内存态兜底。
    }
  }

  // 启动目录：空串 = 恢复默认（家目录）；sidecar 在 start 时校验现存目录。
  async function setLocalShellCwd(dir: string) {
    localShellCwdPref.value = dir.trim();
    try {
      await window.dbxPlugin.invoke("local/preferences/set", { localShellCwd: localShellCwdPref.value });
    } catch {
      // 旧 sidecar：会话内存态兜底。
    }
  }

  // 环境变量：持久化取净化副本（未填名的草稿行不落库，保留在 UI 里继续编辑）；
  // 名字重复时后者覆盖前者（与 start 载荷的 map 语义一致）。
  async function setLocalShellEnv() {
    try {
      await window.dbxPlugin.invoke("local/preferences/set", { localShellEnv: localShellEnvClean.value });
    } catch {
      // 旧 sidecar：会话内存态兜底。
    }
  }

  function addLocalShellEnvRow() {
    localShellEnvPref.value = [...localShellEnvPref.value, { name: "", value: "" }];
  }

  function removeLocalShellEnvRow(index: number) {
    localShellEnvPref.value = localShellEnvPref.value.filter((_, i) => i !== index);
    void setLocalShellEnv();
  }

  // P0.2 self-open: opens a separate connectionless local-terminal tab through the host openWorkbench bridge.
  // A4 target contract (spec §4/§11): the plugin payload lives only in context.plugin and the instance identity
  // (workbenchId) is generated by the host — when a legacy host omits it, the workbenchId fallback covers it.
  // The menu item is hidden on hosts without this bridge.
  const canOpenLocalTab = computed(() => Boolean(window.dbxPlugin?.openWorkbench));

  async function openLocalTerminalTab() {
    const api = window.dbxPlugin;
    if (!api.openWorkbench) return;
    await api.openWorkbench(
      WORKBENCH_ID,
      { plugin: { mode: "local-terminal" } },
      { forceNew: true },
    );
  }

  return {
    localShellPref,
    localShellIntegrationPref,
    localShellCwdPref,
    localShellEnvPref,
    localShellEnvClean,
    localShells,
    localShellsLoading,
    selectedShellInjectable,
    localShellSurfaceOpen,
    dockConnections,
    pickLocalShell,
    openSessionShellPick,
    loadLocalShells,
    openLocalShellPrefs,
    openLocalShellSurfaceMenu,
    openConnectionSurface,
    openLocalShellSurface,
    setLocalShellPref,
    setLocalShellIntegrationPref,
    setLocalShellCwd,
    setLocalShellEnv,
    addLocalShellEnvRow,
    removeLocalShellEnvRow,
    canOpenLocalTab,
    openLocalTerminalTab,
  };
}
