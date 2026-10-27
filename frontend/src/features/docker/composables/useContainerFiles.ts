// 容器文件浏览状态（Task 10）：当前路径、条目、选中项与懒加载 stat。
//
// 列表/stat 的载荷归一在 `lib/dockerFilePaths`；这里只做请求编排、错误留驻
// 与「容器/会话变化即回到 `/` 重载」。选中条目的 stat 是尽力而为：容器缺
// `stat` 时后端返回 `{unavailable:true}`，缺工具/查询失败都不弹错误，UI 显示
// “—”，下载仍可继续（size 由 export 再确认）。
import { computed, onMounted, ref, watch } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import { dockerChannelKey, runDockerChannel } from "../lib/dockerChannelQueue";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import {
  breadcrumbs as buildBreadcrumbs,
  containerFileListPayload,
  isMissingLsError,
  normalizeContainerPath,
  parentPath,
  type ContainerFileEntry,
} from "../lib/dockerFilePaths";

export interface ContainerFileStat {
  unavailable: boolean;
  kind: string;
  size: number | null;
  mtime: number | null;
  mode: string;
}

export interface UseContainerFilesOptions {
  t: (key: string, params?: Record<string, string | number>) => string;
  /** 选中容器；切换即重置到根目录。 */
  containerId: () => string;
  /** 工作台绑定的 SSH 会话；缺省 = 本机 daemon。 */
  sessionId?: () => string | undefined;
  /** 引擎设置（cli/socket/host）按连接持久化的键。 */
  connectionId?: () => string | undefined;
  /** 文件页是否可见；隐藏时只复位状态、不发 docker/files/list。 */
  visible?: () => boolean;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/** `docker/files/stat` 响应归一；未知形状视为 unavailable（UI 显示 “—”）。 */
export function containerStatPayload(raw: unknown): ContainerFileStat {
  const record = (raw ?? {}) as Record<string, unknown>;
  if (record.unavailable === true) {
    return { unavailable: true, kind: "", size: null, mtime: null, mode: "" };
  }
  const size = typeof record.size === "number" && Number.isFinite(record.size) ? record.size : null;
  return {
    unavailable: false,
    kind: typeof record.kind === "string" ? record.kind : "",
    size,
    mtime: typeof record.mtime === "number" ? record.mtime : null,
    mode: typeof record.mode === "string" ? record.mode : "",
  };
}

export function useContainerFiles(options: UseContainerFilesOptions) {
  const readSessionId = options.sessionId ?? (() => undefined);
  const readConnectionId = options.connectionId ?? (() => undefined);
  const isVisible = options.visible ?? (() => true);

  const path = ref("/");
  const entries = ref<ContainerFileEntry[]>([]);
  const selected = ref<ContainerFileEntry>();
  const selectedMeta = ref<ContainerFileStat>();
  const selectedMetaLoading = ref(false);
  const loading = ref(false);
  const error = ref("");
  const toolMissing = ref(false);

  const crumbs = computed(() => buildBreadcrumbs(path.value));

  /** 单调请求序号：慢链路下过期响应整体丢弃，不把 UI 回跳到旧目录。 */
  let listSeq = 0;
  let statSeq = 0;
  /** 列表请求串行合并：同一时刻最多一个 docker/files/list channel，
   *  快速切容器时只保留最新目标（SSH MaxSessions 压力来源之一）。 */
  let queuedTarget: string | undefined;
  let listRunner: Promise<void> | undefined;
  let loadedKey = "";

  function targetKey(): string {
    return `${options.containerId()}\u0000${readSessionId() ?? ""}\u0000${readConnectionId() ?? ""}`;
  }

  function requestParams(target: string): Record<string, unknown> {
    const sessionId = readSessionId();
    return {
      ...(sessionId ? { sessionId } : { target: "local" as const }),
      containerId: options.containerId(),
      path: target,
      ...dockerEngineParams(loadDockerEngineSettings(readConnectionId() || "")),
    };
  }

  function clearSelected(): void {
    selected.value = undefined;
    selectedMeta.value = undefined;
    selectedMetaLoading.value = false;
    statSeq += 1;
  }

  function scheduleLoad(target: string): Promise<void> {
    queuedTarget = target;
    if (listRunner) return listRunner;
    listRunner = (async () => {
      while (queuedTarget !== undefined) {
        const next = queuedTarget;
        queuedTarget = undefined;
        await loadNow(next);
      }
    })().finally(() => {
      listRunner = undefined;
    });
    return listRunner;
  }

  async function loadNow(target: string): Promise<void> {
    const seq = ++listSeq;
    loading.value = true;
    error.value = "";
    try {
      const payload = containerFileListPayload(
        await runDockerChannel(
          dockerChannelKey(readSessionId(), readConnectionId()),
          () => rpc<unknown>("docker/files/list", requestParams(target)),
        ),
        target,
      );
      if (seq !== listSeq) return;
      path.value = payload.path;
      entries.value = payload.entries;
      toolMissing.value = false;
    } catch (cause) {
      if (seq !== listSeq) return;
      const message = errorMessage(cause);
      toolMissing.value = isMissingLsError(message);
      error.value = toolMissing.value ? options.t("docker.filesToolMissing") : message;
      entries.value = [];
    } finally {
      if (seq === listSeq) loading.value = false;
    }
  }

  async function enter(target: string | ContainerFileEntry): Promise<void> {
    const next = normalizeContainerPath(
      typeof target === "string" ? target : target.kind === "dir" ? target.path : path.value,
    );
    if (typeof target !== "string" && target.kind !== "dir") return;
    clearSelected();
    await scheduleLoad(next);
  }

  async function parent(): Promise<void> {
    clearSelected();
    await scheduleLoad(parentPath(path.value));
  }

  async function refresh(): Promise<void> {
    clearSelected();
    await scheduleLoad(path.value);
  }

  /** 选中条目：文件懒加载 stat；`unavailable`/失败都不改变选中与下载能力。 */
  async function select(entry: ContainerFileEntry): Promise<void> {
    selected.value = entry;
    selectedMeta.value = undefined;
    const seq = ++statSeq;
    if (entry.kind !== "file") return;
    selectedMetaLoading.value = true;
    try {
      const payload = containerStatPayload(
        await runDockerChannel(
          dockerChannelKey(readSessionId(), readConnectionId()),
          () => rpc<unknown>("docker/files/stat", requestParams(entry.path)),
        ),
      );
      if (seq === statSeq) selectedMeta.value = payload;
    } catch {
      // stat 是可选信息：缺 stat/权限不足都只让详情显示 “—”。
      if (seq === statSeq) selectedMeta.value = undefined;
    } finally {
      if (seq === statSeq) selectedMetaLoading.value = false;
    }
  }

  /** 容器或目标会话变化：回到根目录；只有可见时才重载。 */
  function reset(): void {
    listSeq += 1;
    queuedTarget = undefined;
    path.value = "/";
    entries.value = [];
    loading.value = false;
    error.value = "";
    toolMissing.value = false;
    clearSelected();
    loadedKey = targetKey();
    if (isVisible()) void scheduleLoad("/");
  }

  watch(
    () => [options.containerId(), readSessionId(), readConnectionId()] as const,
    () => reset(),
  );

  watch(isVisible, (visible) => {
    if (!visible) return;
    if (loadedKey !== targetKey()) {
      loadedKey = targetKey();
      reset();
      return;
    }
    if (!entries.value.length) void scheduleLoad(path.value);
  });

  onMounted(() => {
    loadedKey = targetKey();
    if (isVisible()) void scheduleLoad(path.value);
  });

  return {
    path,
    entries,
    selected,
    selectedMeta,
    selectedMetaLoading,
    loading,
    error,
    toolMissing,
    breadcrumbs: crumbs,
    enter,
    parent,
    refresh,
    select,
    clearSelected,
  };
}
