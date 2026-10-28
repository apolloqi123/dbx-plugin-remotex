/**
 * 端口映射面板状态（列表/预设/事件/启停/复制）：工具栏快捷弹层与完整管理器
 * 共享同一实例，避免两份列表各自漂移。纯逻辑仍在 lib/portForward.ts。
 */
import { computed, getCurrentScope, onScopeDispose, ref, type ComputedRef, type Ref } from "vue";
import { writeClipboardText, type ClipboardDeps } from "../../../shared/lib/clipboardBridge";
import {
  applyForwardState,
  findForwardConflict,
  formatForwardAddress,
  forwardStartParams,
  forwardSummary,
  mergeForwardCards,
  parseForwards,
  parseInterfaces,
  profileMatchesForward,
  validateForwardForm,
  type ForwardCard,
  type ForwardFormDraft,
  type HostInterface,
  type PortForward,
  type ForwardSummary,
} from "../lib/portForward";
import {
  deleteTunnelProfile,
  loadTunnelProfiles,
  saveTunnelProfile,
  type TunnelProfile,
} from "../lib/tunnelProfiles";

export interface PortForwardsOptions {
  connectionId: () => string;
  independent: () => boolean;
  sessionId: () => string | null;
  clipboard?: () => ClipboardDeps;
  onError?: (cause: unknown) => void;
}

export interface PortForwardsController {
  forwards: Ref<PortForward[]>;
  profiles: Ref<TunnelProfile[]>;
  interfaces: Ref<HostInterface[]>;
  loading: Ref<boolean>;
  busyId: Ref<string | null>;
  startingAll: Ref<boolean>;
  submitting: Ref<boolean>;
  error: Ref<string>;
  message: Ref<string>;
  startedCount: Ref<number | null>;
  summary: ComputedRef<ForwardSummary>;
  cards: ComputedRef<ForwardCard<TunnelProfile>[]>;
  inactiveProfiles: ComputedRef<TunnelProfile[]>;
  open: () => Promise<void>;
  refresh: () => Promise<void>;
  start: (draft: ForwardFormDraft) => Promise<boolean>;
  startSaved: (targets?: TunnelProfile[]) => Promise<void>;
  toggle: (profile: TunnelProfile) => Promise<void>;
  stop: (id: string) => Promise<void>;
  stopAll: () => Promise<void>;
  saveProfile: (draft: ForwardFormDraft) => boolean;
  removeProfile: (id: string) => void;
  copyAddress: (target: Pick<PortForward, "listenHost" | "listenPort" | "boundPort">) => Promise<string>;
}

export function usePortForwards(options: PortForwardsOptions): PortForwardsController {
  const forwards = ref<PortForward[]>([]);
  const profiles = ref<TunnelProfile[]>([]);
  const interfaces = ref<HostInterface[]>([]);
  const loading = ref(false);
  const busyId = ref<string | null>(null);
  const startingAll = ref(false);
  const submitting = ref(false);
  const error = ref("");
  const message = ref("");
  const startedCount = ref<number | null>(null);

  const summary = computed(() => forwardSummary(forwards.value));
  const cards = computed(() => mergeForwardCards(forwards.value, profiles.value));
  const inactiveProfiles = computed(() =>
    profiles.value.filter(
      (profile) => !forwards.value.some((row) => row.state !== "stopped" && row.state !== "error" && profileMatchesForward(profile, row)),
    ),
  );

  let listRevision = 0;

  function reportError(cause: unknown) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    options.onError?.(cause);
  }

  async function refresh() {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    const revision = ++listRevision;
    loading.value = true;
    try {
      const payload = await window.dbxPlugin.invoke("ssh/forward/list", { connectionId });
      if (revision === listRevision) forwards.value = parseForwards(payload);
    } catch (cause) {
      if (revision !== listRevision) return;
      console.warn("[port-forward] list failed", cause);
      reportError(cause);
    } finally {
      if (revision === listRevision) loading.value = false;
    }
  }

  function refreshProfiles() {
    profiles.value = loadTunnelProfiles(options.connectionId());
  }

  async function refreshInterfaces() {
    try {
      interfaces.value = parseInterfaces(await window.dbxPlugin.invoke("ssh/forward/interfaces"));
    } catch (cause) {
      console.warn("[port-forward] interface probe failed", cause);
      interfaces.value = [];
    }
  }

  async function open() {
    error.value = "";
    await refresh();
    refreshProfiles();
    void refreshInterfaces();
  }

  async function start(draft: ForwardFormDraft): Promise<boolean> {
    startedCount.value = null;
    message.value = "";
    const connectionId = options.connectionId();
    const ownerId = options.independent() ? connectionId : options.sessionId();
    if (!ownerId) {
      error.value = "ssh/forward/start needs an active SSH session";
      return false;
    }
    const params = forwardStartParams(draft, ownerId, options.independent());
    try {
      let payload: unknown;
      try {
        payload = await window.dbxPlugin.invoke("ssh/forward/start", params);
      } catch (cause) {
        if (!options.independent() || !String(cause).includes("Connection is not active")) throw cause;
        if (window.dbxPlugin.reopenConnection) await window.dbxPlugin.reopenConnection(connectionId);
        else await window.dbxPlugin.request("host.reopenConnection", { connectionId });
        payload = await window.dbxPlugin.invoke("ssh/forward/start", params);
      }
      const started = parseForwards(payload);
      if (started.length) {
        forwards.value = [...forwards.value.filter((row) => row.id !== started[0].id), ...started];
      }
      error.value = "";
      return true;
    } catch (cause) {
      reportError(cause);
      return false;
    }
  }

  function saveProfile(draft: ForwardFormDraft): boolean {
    const invalid = validateForwardForm(draft);
    if (invalid) {
      error.value = "Invalid tunnel preset";
      return false;
    }
    try {
      saveTunnelProfile(options.connectionId(), draft);
      refreshProfiles();
      error.value = "";
      return true;
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause);
      return false;
    }
  }

  function removeProfile(id: string) {
    deleteTunnelProfile(id);
    refreshProfiles();
  }

  async function startSaved(targets: TunnelProfile[] = profiles.value) {
    if (startingAll.value) return;
    startingAll.value = true;
    message.value = "";
    startedCount.value = null;
    const failures: string[] = [];
    let started = 0;
    for (const profile of targets) {
      if (!inactiveProfiles.value.some((candidate) => candidate.id === profile.id)) continue;
      if (findForwardConflict(forwards.value, profile)) continue;
      if (await start(profile)) started += 1;
      else failures.push(`${formatForwardAddress({ ...profile, listenPort: Number(profile.listenPort), boundPort: 0 })}: ${error.value}`);
    }
    startedCount.value = started;
    message.value = failures.join("\n");
    startingAll.value = false;
  }

  function runningForward(profile: TunnelProfile): PortForward | undefined {
    return forwards.value.find(
      (row) => row.state !== "stopped" && row.state !== "error" && profileMatchesForward(profile, row),
    );
  }

  async function toggle(profile: TunnelProfile) {
    const running = runningForward(profile);
    if (running) await stop(running.id);
    else await startSaved([profile]);
  }

  async function stop(id: string) {
    if (busyId.value) return;
    busyId.value = id;
    try {
      await window.dbxPlugin.invoke("ssh/forward/stop", { id });
      forwards.value = forwards.value.filter((row) => row.id !== id);
    } catch (cause) {
      console.warn("[port-forward] stop failed", cause);
      reportError(cause);
    } finally {
      busyId.value = null;
    }
  }

  async function stopAll() {
    if (busyId.value) return;
    busyId.value = "all";
    error.value = "";
    const failures: string[] = [];
    for (const row of forwards.value.filter((candidate) => candidate.state !== "error")) {
      try {
        await window.dbxPlugin.invoke("ssh/forward/stop", { id: row.id });
        forwards.value = forwards.value.filter((candidate) => candidate.id !== row.id);
      } catch (cause) {
        failures.push(`${formatForwardAddress(row)}: ${String(cause)}`);
      }
    }
    if (failures.length) error.value = failures.join("\n");
    busyId.value = null;
  }

  async function copyAddress(target: Pick<PortForward, "listenHost" | "listenPort" | "boundPort">): Promise<string> {
    const address = formatForwardAddress(target);
    await writeClipboardText(address, options.clipboard?.() ?? {});
    return address;
  }

  function handleEvent(event: DbxPluginEvent) {
    if (event.type === "env") return;
    if (event.method !== "ssh/forward/state") return;
    if (event.params.connectionId !== options.connectionId()) return;
    // 打开面板时的 list 快照可能比事件旧：让在途请求作废，事件状态优先。
    ++listRevision;
    loading.value = false;
    const known = forwards.value.some((row) => row.id === event.params.id);
    forwards.value = applyForwardState(forwards.value, event.params);
    if (event.params.state === "stopped") {
      forwards.value = forwards.value.filter((row) => row.id !== event.params.id);
    } else if (!known) {
      // 右键菜单/其他工作台发起的隧道：事件不带完整行，按 id 重新拉取。
      void refresh();
    }
  }

  const unsubscribe = window.dbxPlugin.onEvent(handleEvent);
  if (getCurrentScope()) onScopeDispose(unsubscribe);

  return {
    forwards,
    profiles,
    interfaces,
    loading,
    busyId,
    startingAll,
    submitting,
    error,
    message,
    startedCount,
    summary,
    cards,
    inactiveProfiles,
    open,
    refresh,
    start,
    startSaved,
    toggle,
    stop,
    stopAll,
    saveProfile,
    removeProfile,
    copyAddress,
  };
}
