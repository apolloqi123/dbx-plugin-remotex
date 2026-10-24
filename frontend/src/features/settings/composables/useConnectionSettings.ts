// 连接级设置（启动命令 / SFTP 文件名编码覆盖 / agent 模式镜像）与设置弹窗的
// 会话级加载路径。从 SettingsDialog.vue 抽出，行为与状态语义不变：
// - 启动命令与文件名编码同 X11 走组件内自治 RPC 读写 sidecar 偏好；镜像
//   （pluginStore）兼作云同步载荷，条目缺失播种、存在但漂移则 store 赢并推回。
// - agent 模式 + 免审批命令在 reloadSettings 拿到 sidecar 视图后逐域收敛。
// - settingsMeta/settingsDraft 仍由弹窗主保存链（saveSettings）直接读取。
import { reactive } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import {
  sanitizeAgentTerminalMode,
  sanitizeRememberedCommands,
  SSH_AGENT_APPROVED_STORE_KEY,
  SSH_AGENT_MODES_STORE_KEY,
} from "../../agent/lib/agentTerminal";
import { connectionEntryEquals, loadConnectionEntry, saveConnectionEntry } from "../../connections/lib/connectionSettingMirror";
import {
  choiceFromOverride,
  mergeNameEncodingStore,
  NAME_ENCODING_OVERRIDES_STORE_KEY,
  normalizeNameEncodingOverride,
  overrideFromChoice,
  type ConnectionNameEncodingChoice,
  type NameEncodingOverride,
} from "../../connections/lib/connectionNameEncoding";
import {
  createStartupEntry,
  mergeStartupStore,
  normalizeStartupConfig,
  STARTUP_COMMAND_MAX,
  STARTUP_COMMANDS_STORE_KEY,
  STARTUP_DELAY_DEFAULT_MS,
  STARTUP_DELAY_MAX_MS,
  type StartupCommandEntry,
  type StartupCommandsConfig,
} from "../../connections/lib/startupCommands";
import type { SshSettings } from "../lib/settingsModel";

/** 设置弹窗左侧「会话级」设置草稿（ssh/settings/get 回显 + 主保存链读取）。 */
export interface ConnectionSettingsDraft {
  quickSudo: boolean;
  sudoUsePty: boolean;
  sudoPassword: string;
  totpSecret: string;
  authFlowMode: string;
  passwordPromptHint: string;
  totpPromptHint: string;
  quickSudoProfileId: string;
  agentTerminalMode: string;
  rememberedCommands: string[];
}

export function useConnectionSettings(options: {
  connectionId: () => string | undefined;
  sessionId: () => string | undefined;
  isLocalSession: () => boolean;
  isOpen: () => boolean;
  emitError: (cause: unknown) => void;
}) {
  const state = reactive({
    meta: undefined as SshSettings | undefined,
    loading: false,
    loadFailed: false,
    draft: {
      quickSudo: true,
      sudoUsePty: false,
      sudoPassword: "",
      totpSecret: "",
      authFlowMode: "password_then_otp",
      passwordPromptHint: "",
      totpPromptHint: "",
      quickSudoProfileId: "",
      agentTerminalMode: "off",
      rememberedCommands: [] as string[],
    } as ConnectionSettingsDraft,
    /** 连接级启动命令（P0-4，Tabby「Login scripts」对标）：按 connectionId 分桶。 */
    startupEnabled: false,
    startupCommands: [] as StartupCommandEntry[],
    /** 连接级 SFTP 文件名编码覆盖（M16）：「follow」= 不落盘、跟随全局。 */
    nameEncoding: "follow" as ConnectionNameEncodingChoice,
  });

  function applyStartupConfig(config: StartupCommandsConfig) {
    state.startupEnabled = config.enabled;
    state.startupCommands = config.commands;
  }

  async function loadStartupCommands() {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    // 存储迁移批 2（IMPL_PLAN_STORAGE_SYNC）：镜像播种/收敛——条目缺失从
    // sidecar 播种；条目存在但与 sidecar 漂移（云同步恢复后）则 store 赢，
    // 推回 sidecar（复用读改写持久化）。
    const mirrored = loadConnectionEntry(STARTUP_COMMANDS_STORE_KEY, connectionId, normalizeStartupConfig);
    try {
      const prefs = await window.dbxPlugin?.invoke<{ startup_commands?: unknown }>("local/preferences/get", {});
      const store = prefs?.startup_commands as Record<string, unknown> | undefined;
      const sidecar = normalizeStartupConfig(store?.[connectionId]);
      if (mirrored === null) {
        applyStartupConfig(sidecar);
        saveConnectionEntry(STARTUP_COMMANDS_STORE_KEY, connectionId, sidecar);
        return;
      }
      if (!connectionEntryEquals(mirrored, sidecar)) {
        applyStartupConfig(mirrored);
        await persistStartupConfig(mirrored);
        return;
      }
      applyStartupConfig(sidecar);
    } catch {
      // sidecar 不可用：镜像兜底（未迁移则维持空表，下次打开重试播种）。
      if (mirrored !== null) applyStartupConfig(mirrored);
    }
  }

  /// 读改写合并：`local/preferences/set` 对 startup_commands 是整键替换，
  /// 其他连接的桶从最新偏好读回后原样保留。
  async function persistStartupConfig(config: StartupCommandsConfig) {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    try {
      const prefs = await window.dbxPlugin?.invoke<{ startup_commands?: unknown }>("local/preferences/get", {});
      await window.dbxPlugin?.invoke("local/preferences/set", {
        startup_commands: mergeStartupStore(
          prefs?.startup_commands,
          connectionId,
          config,
        ),
      });
    } catch (cause) {
      options.emitError(cause);
    }
  }

  async function persistStartupCommands() {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    const config: StartupCommandsConfig = { enabled: state.startupEnabled, commands: state.startupCommands };
    await persistStartupConfig(config);
    // 保存双写：RPC（sidecar 即时权威）+ 镜像（云同步载荷）。
    saveConnectionEntry(STARTUP_COMMANDS_STORE_KEY, connectionId, config);
  }

  async function setStartupEnabled(next: boolean) {
    state.startupEnabled = next;
    // 开启且无命令时给一行空行，省一次点击；清空命令立即持久化。
    if (next && state.startupCommands.length === 0) state.startupCommands.push(createStartupEntry());
    await persistStartupCommands();
  }

  function addStartupEntry() {
    if (state.startupCommands.length >= STARTUP_COMMAND_MAX) return;
    state.startupCommands.push(createStartupEntry());
    void persistStartupCommands();
  }

  function removeStartupEntry(index: number) {
    state.startupCommands.splice(index, 1);
    void persistStartupCommands();
  }

  function moveStartupEntry(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= state.startupCommands.length) return;
    const [row] = state.startupCommands.splice(index, 1);
    state.startupCommands.splice(target, 0, row);
    void persistStartupCommands();
  }

  /** 行内增量（命令文本 / 延迟 / 启用）：改完即持久化（@change 语义，非逐键）。 */
  function updateStartupEntry(index: number, patch: Partial<StartupCommandEntry>) {
    const row = state.startupCommands[index];
    if (!row) return;
    Object.assign(row, patch);
    void persistStartupCommands();
  }

  /** 延迟输入收敛：空/非法回缺省 300ms，越界截断到 30s。 */
  function clampStartupDelayInput(raw: string): number {
    const value = Number.parseInt(raw.trim(), 10);
    if (!Number.isFinite(value) || value < 0) return STARTUP_DELAY_DEFAULT_MS;
    return Math.min(value, STARTUP_DELAY_MAX_MS);
  }

  async function loadConnNameEncoding() {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    // 存储迁移批 2：镜像条目值与 sidecar 同形（"auto"/"latin-1"/null=跟随全局），
    // 播种/收敛语义同启动命令；垃圾条目归一为 null，不会误删 sidecar 覆盖。
    const mirrored = loadConnectionEntry(NAME_ENCODING_OVERRIDES_STORE_KEY, connectionId, normalizeNameEncodingOverride);
    try {
      const prefs = await window.dbxPlugin?.invoke<{ sftp_name_encoding_overrides?: unknown }>("local/preferences/get", {});
      const store = prefs?.sftp_name_encoding_overrides as Record<string, unknown> | undefined;
      const sidecar = normalizeNameEncodingOverride(store?.[connectionId]);
      if (mirrored === null) {
        state.nameEncoding = choiceFromOverride(sidecar);
        saveConnectionEntry(NAME_ENCODING_OVERRIDES_STORE_KEY, connectionId, sidecar);
        return;
      }
      if (mirrored !== sidecar) {
        state.nameEncoding = choiceFromOverride(mirrored);
        await persistConnNameEncodingOverride(mirrored);
        return;
      }
      state.nameEncoding = choiceFromOverride(sidecar);
    } catch {
      // sidecar 不可用：镜像兜底。
      if (mirrored !== null) state.nameEncoding = choiceFromOverride(mirrored);
    }
  }

  /// 读改写合并：`local/preferences/set` 对该键是整表替换（其他连接的桶从最新
  /// 偏好读回后原样保留）；value=null 删除本连接桶（「跟随全局」）。
  async function persistConnNameEncodingOverride(override: NameEncodingOverride | null) {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    try {
      const prefs = await window.dbxPlugin?.invoke<{ sftp_name_encoding_overrides?: unknown }>("local/preferences/get", {});
      await window.dbxPlugin?.invoke("local/preferences/set", {
        sftp_name_encoding_overrides: mergeNameEncodingStore(
          prefs?.sftp_name_encoding_overrides,
          connectionId,
          override,
        ),
      });
    } catch (cause) {
      options.emitError(cause);
    }
  }

  async function persistConnNameEncoding() {
    const connectionId = options.connectionId();
    if (!connectionId) return;
    const override = overrideFromChoice(state.nameEncoding);
    await persistConnNameEncodingOverride(override);
    // 保存双写：RPC + 镜像（null 同样落条目 = 「跟随全局」已迁移）。
    saveConnectionEntry(NAME_ENCODING_OVERRIDES_STORE_KEY, connectionId, override);
  }

  function setConnNameEncoding(choice: ConnectionNameEncodingChoice) {
    state.nameEncoding = choice;
    void persistConnNameEncoding();
  }

  // ---------------------------------------------------------------------------
  // agent 模式 + 免审批命令的连接级镜像（存储迁移批 2，IMPL_PLAN_STORAGE_SYNC）：
  // sidecar（agent-modes.json / agent-approved-commands.json）在审批门即时消费，
  // pluginStore 只存云同步载荷。reloadSettings 拿到 sidecar 视图后：
  // 条目缺失 → 播种；条目存在但漂移（云同步恢复后）→ store 赢，采纳镜像并推回
  // sidecar（ssh/settings/set 是部分更新，只带漂移域）。保存双写在 saveSettings。
  // ---------------------------------------------------------------------------

  async function syncAgentSettingsMirror() {
    const connectionId = options.connectionId();
    const sessionId = options.sessionId();
    const meta = state.meta;
    if (!connectionId || !sessionId || !meta) return;
    const sidecarMode = sanitizeAgentTerminalMode(meta.agentTerminalMode);
    const sidecarRemembered = sanitizeRememberedCommands(meta.rememberedCommands);
    const mirroredMode = loadConnectionEntry(SSH_AGENT_MODES_STORE_KEY, connectionId, sanitizeAgentTerminalMode);
    const mirroredRemembered = loadConnectionEntry(SSH_AGENT_APPROVED_STORE_KEY, connectionId, sanitizeRememberedCommands);
    if (mirroredMode === null) saveConnectionEntry(SSH_AGENT_MODES_STORE_KEY, connectionId, sidecarMode);
    if (mirroredRemembered === null) saveConnectionEntry(SSH_AGENT_APPROVED_STORE_KEY, connectionId, sidecarRemembered);
    if (mirroredMode === null || mirroredRemembered === null) return;
    const modeDrift = !connectionEntryEquals(mirroredMode, sidecarMode);
    const rememberedDrift = !connectionEntryEquals(mirroredRemembered, sidecarRemembered);
    if (!modeDrift && !rememberedDrift) return;
    if (modeDrift) state.draft.agentTerminalMode = mirroredMode;
    if (rememberedDrift) state.draft.rememberedCommands = mirroredRemembered;
    try {
      await rpc("ssh/settings/set", {
        sessionId,
        ...(modeDrift ? { agentTerminalMode: mirroredMode } : {}),
        ...(rememberedDrift ? { rememberedCommands: mirroredRemembered } : {}),
      });
    } catch {
      // 推回失败不阻断弹窗加载：镜像仍在，下次打开重试。
    }
  }

  /** 打开设置时的会话级加载（loading 门控 + ssh/settings/get + 镜像收敛）。 */
  async function load() {
    state.loading = true;
    state.loadFailed = false;
    state.meta = undefined;
    void loadStartupCommands();
    void loadConnNameEncoding();
    try {
      // 本地终端没有 SSH 会话：跳过会话级设置拉取，只保留全局偏好面板——
      // 否则 required_string 直接报错，整条弹窗被「SSH 设置加载失败」横幅替换。
      if (!options.isLocalSession()) {
        // revealSecrets: 预填已存原值（原始凭据串），避免只能看到"已配置"占位。
        const meta = await rpc<SshSettings>("ssh/settings/get", { sessionId: options.sessionId(), revealSecrets: true });
        state.meta = meta;
        state.draft.quickSudo = meta.quickSudo;
        state.draft.sudoUsePty = meta.sudoUsePty;
        state.draft.authFlowMode = meta.authFlowMode || "password_then_otp";
        state.draft.passwordPromptHint = meta.passwordPromptHint || "";
        state.draft.totpPromptHint = meta.totpPromptHint || "";
        state.draft.quickSudoProfileId = meta.quickSudoProfileId || "";
        state.draft.agentTerminalMode = sanitizeAgentTerminalMode(meta.agentTerminalMode);
        state.draft.rememberedCommands = sanitizeRememberedCommands(meta.rememberedCommands);
        state.draft.sudoPassword = meta.sudoPassword || "";
        state.draft.totpSecret = meta.totpSecret || "";
        // 存储迁移批 2（IMPL_PLAN_STORAGE_SYNC）：agent 模式与免审批命令的
        // 镜像播种/收敛（逐域独立，见函数注释）。
        await syncAgentSettingsMirror();
      }
    } catch {
      state.loadFailed = true;
    } finally {
      state.loading = false;
    }
  }

  /// 全局配置或其绑定变化后，刷新设置弹窗的只读摘要（会话内即时生效）。
  async function refreshSettingsMeta() {
    if (!options.isOpen() || options.isLocalSession()) return;
    try {
      state.meta = await rpc<SshSettings>("ssh/settings/get", { sessionId: options.sessionId() });
    } catch {
      // 摘要刷新失败不打断主流程；重新打开设置时会再次加载。
    }
  }

  return {
    state,
    load,
    loadStartupCommands,
    loadConnNameEncoding,
    setStartupEnabled,
    addStartupEntry,
    removeStartupEntry,
    moveStartupEntry,
    updateStartupEntry,
    clampStartupDelayInput,
    setConnNameEncoding,
    refreshSettingsMeta,
  };
}

export type ConnectionSettingsController = ReturnType<typeof useConnectionSettings>;
