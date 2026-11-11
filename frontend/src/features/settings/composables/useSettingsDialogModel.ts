// 设置弹窗的数据/加载/保存模型：从 SettingsDialog.vue 原样抽出（纯机械搬运，
// 行为、保存顺序、校验与 i18n key 不变）。持有连接级设置（ssh/settings/get|set）、
// sudo 配置档（sudo/profiles/*）、已知主机（ssh/knownHosts/*）、本机密钥发现
// （keys/discover）、MCP 限速（mcp/settings/*）、X11 / 自动录制 / RDP 实验开关
// 的组件内自治偏好、下载/传输/编辑器/命令建议草稿、本地 Shell 选项，以及主
// 保存链与 Esc 分层退出。
//
// 组件只保留 props/emits、分类导航与模板；模板绑定名通过解构本模型返回的扁平
// 对象保持不变。终端偏好（WebGL/选中复制/终端字体）的权威态仍在宿主 App——
// 下载偏好经 downloadPrefs 适配器读写，字体经 emitApplyFont 上抛。
import { computed, reactive, ref, watch } from "vue";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { rpc } from "../../../shared/lib/rpc";
import { settingsPreferences } from "../lib/settingsPreferences";
import {
  MIB,
  mibField,
  persistMcpSettingsMirror,
  settingsErrorOf,
  type DiscoveredKey,
  type KnownHostEntry,
  type McpSizeSettings,
  type SshSettings,
} from "../lib/settingsModel";
import type { SettingsDialogProps } from "../lib/settingsDialogProps";
import { SSH_AGENT_APPROVED_STORE_KEY, SSH_AGENT_MODES_STORE_KEY, sanitizeAgentTerminalMode, sanitizeRememberedCommands } from "../../agent/lib/agentTerminal";
import { saveConnectionEntry } from "../../connections/lib/connectionSettingMirror";
import { rdpExperimentalEnabled as parseRdpExperimentalEnabled } from "../../remote-desktop/lib/rdpExperimental";
import type { EditorConfig, KnownEditor } from "../../sftp/lib/editor/editorRules";
import type { DownloadConflictPolicy } from "../../sftp/lib/transfer/downloadPrefs";
import type { SftpNameEncoding } from "../../sftp/lib/entries/sftpName";
import type { TransferDuplicatePolicy } from "../../transfers/lib/transferQueue";
import type { TerminalHotkeyBindings } from "../../terminal/lib/behavior/terminalHotkeys";
import { useConnectionSettings } from "./useConnectionSettings";
import { useSudoProfiles } from "./useSudoProfiles";
import { useTerminalFontControls } from "./useTerminalFontControls";

export function useSettingsDialogModel(options: {
  props: SettingsDialogProps;
  emitNotice: (message: string) => void;
  emitError: (cause: unknown) => void;
  emitApplyFont: (payload: { family: string | null; size: number }) => void;
  emitHotkeys: (bindings: TerminalHotkeyBindings) => void;
}) {
  const props = options.props;
  const t = props.t;

  // 会话级分类（sudo / 智能体）只对 SSH 会话有意义：本地终端没有 sessionId，
  // 加载/保存/清密钥/摘要刷新四处门控都会跳过，面板沦为可编辑却永不可保存的
  // 死 UI——直接从导航隐藏。本地判定收敛成一处，新增会话级门控一律引用
  // isLocalSession，别再手写 `!props.sessionId`（漏写会重新制造「横幅替换
  // 整条弹窗」的原始 bug）。
  const isLocalSession = computed(() => !props.sessionId);

  // 连接级设置（启动命令 / 文件名编码 / agent 镜像）与会话级加载路径：
  // settingsMeta/settingsDraft 仍由本模型的主保存链直接读取。
  const connection = useConnectionSettings({
    connectionId: () => props.connectionId,
    sessionId: () => props.sessionId,
    isLocalSession: () => isLocalSession.value,
    isOpen: () => props.open,
    emitError: options.emitError,
  });

  // 全局 quick sudo 配置档：列表与编辑表单状态（密钥只在提交时发送）。设置弹窗
  // 内联 section 与独立 profiles 弹窗共存复用同一份状态。
  const sudo = useSudoProfiles({
    t,
    settingsDraft: connection.state.draft,
    isProfilesOpen: () => props.profilesOpen,
    refreshSettingsMeta: connection.refreshSettingsMeta,
    emitNotice: options.emitNotice,
  });

  // 终端字体控件（issue #31）：预设等宽字体 + 跟随宿主 + 自定义；初始化一次，
  // 应用/恢复默认后刷新，外观分栏只负责渲染。
  const font = useTerminalFontControls({
    terminalFontSize: () => props.terminalFontSize,
    hostFontSize: () => props.hostFontSize,
    emitApplyFont: options.emitApplyFont,
  });

  const settingsSaving = ref(false);
  // 下载 / 传输 / 编辑器 / 命令建议草稿：随主「保存」落库，重开弹窗（reloadSettings）
  // 从适配器回填。
  const transferDrafts = reactive({
    downloadDir: "",
    downloadUseDefault: props.downloadPrefs.loadUseDefault(),
    downloadConflict: "rename" as DownloadConflictPolicy,
    editorConfig: { associations: [], customEditors: [], uploadPolicy: "auto" } as EditorConfig,
    editorAssociationPattern: "",
    editorAssociationTarget: "",
    editorDefault: "",
    editorKnownEditors: [] as KnownEditor[],
    concurrency: String(props.transferPrefs.loadConcurrency()),
    duplicatePolicy: props.transferPrefs.loadDuplicatePolicy() as TransferDuplicatePolicy,
    maxActive: String(props.transferPrefs.loadMaxActive()),
    downloadLimit: String(props.transferPrefs.loadDownloadLimit()),
    compatMode: props.transferPrefs.loadCompatMode(),
    nameEncoding: props.transferPrefs.loadNameEncoding() as SftpNameEncoding,
    compressMode: props.transferPrefs.loadCompressMode(),
    compressThreshold: String(props.transferPrefs.loadCompressThreshold()),
  });
  const suggestionDrafts = reactive({
    enabled: props.suggestionPrefs.loadEnabled(),
    minChars: String(props.suggestionPrefs.loadMinChars()),
    maxChars: String(props.suggestionPrefs.loadMaxChars()),
  });
  // 历史条数输入框草稿由设置弹窗持有：关闭再打开时保留用户输入的非提交态
  // （拆分前语义），提交才归一化写穿 store。
  const commandHistoryLimitDraft = ref(String(settingsPreferences.commandHistoryLimit.value));

  function editorDefaultValue(config: EditorConfig): string {
    return config.defaultCustomId
      ? `custom:${config.defaultCustomId}`
      : config.defaultEditorId
        ? `known:${config.defaultEditorId}`
        : /* reka Select 不接受空串 value，用 "system" 哨兵表示系统默认 */ "system";
  }

  const knownHosts = ref<KnownHostEntry[]>([]);
  const knownHostsLoading = ref(false);
  const knownHostsError = ref("");
  const localKeys = ref<DiscoveredKey[]>([]);
  const localKeysLoading = ref(false);
  const localKeysError = ref("");
  const mcpDraft = reactive({ readMiB: "", uploadMiB: "", downloadMiB: "", permissionMode: "autonomous", connectionScope: "" });
  const mcpLoading = ref(false);
  const mcpError = ref("");
  const mcpSaving = ref(false);
  const mcpInputsValid = computed(() => [mcpDraft.readMiB, mcpDraft.uploadMiB, mcpDraft.downloadMiB]
    .every((value) => /^\d+$/.test(value.trim()) && Number.parseInt(value.trim(), 10) > 0));

  /** X11 转发偏好（P3-3）：组件内自治读写 sidecar 偏好——即时生效语义
   * （新会话才启用），不走 props/emit（App 无需感知）。 */
  const x11Enabled = ref(false);
  const x11ReadOnlyNote = ref(false);
  /** 偏好快照读取失败：三项开关此时显示的是默认值，不是已保存值。 */
  const localTogglePrefsFailed = ref(false);

  async function setX11Enabled(next: boolean) {
    x11Enabled.value = next;
    try {
      await window.dbxPlugin?.invoke("local/preferences/set", { x11_forwarding: next });
    } catch (cause) {
      x11Enabled.value = !next;
      // 回滚必须解释原因：开关自己弹回去是最难理解的失败形态（用户会反复点）。
      // 与 saveSettings 同一条 error 通道（SettingsDialog 上抛 App 的 showError）。
      options.emitError(cause);
    }
  }

  /** 会话自动录制偏好（M14，对标 iShell）：与 X11 同款「组件内自治读写
   * sidecar 偏好」模式，即时生效语义（之后 open 的会话才自动录制）。 */
  const autoRecordEnabled = ref(false);

  /** RDP 仍处实验阶段：默认关闭，必须由用户在设置中显式启用。 */
  const rdpExperimentalEnabled = ref(false);

  async function setRdpExperimentalEnabled(next: boolean) {
    rdpExperimentalEnabled.value = next;
    try {
      // 开关持久化到 sidecar 偏好即可生效（后端 rdp/start 前置检查）；
      // App 不再持有内存门副本（M32-A 后工具条无 RDP 入口）。
      await window.dbxPlugin?.invoke("local/preferences/set", { rdp_experimental_enabled: next });
    } catch (cause) {
      rdpExperimentalEnabled.value = !next;
      options.emitError(cause);
    }
  }

  /** 三个开关同一份 sidecar 偏好快照：此前各发一次完整 get，弹窗一挂载
   *  就连打三个相同的 RPC；合并成一次读取，任一失败整体回落默认。 */
  async function loadLocalTogglePreferences() {
    try {
      const prefs = await window.dbxPlugin?.invoke<{ x11_forwarding?: unknown; rdp_experimental_enabled?: unknown; auto_record?: unknown }>("local/preferences/get", {});
      x11Enabled.value = prefs?.x11_forwarding === true;
      rdpExperimentalEnabled.value = parseRdpExperimentalEnabled(prefs?.rdp_experimental_enabled);
      autoRecordEnabled.value = prefs?.auto_record === true;
    } catch {
      // 回落默认值是正确降级（偏好读不到就按未启用渲染），但要让用户知道
      // 这三项现在显示的是「默认值」而不是「已保存的值」。
      x11Enabled.value = false;
      rdpExperimentalEnabled.value = false;
      autoRecordEnabled.value = false;
      localTogglePrefsFailed.value = true;
    }
  }

  async function setAutoRecordEnabled(next: boolean) {
    autoRecordEnabled.value = next;
    try {
      await window.dbxPlugin?.invoke("local/preferences/set", { auto_record: next });
    } catch (cause) {
      autoRecordEnabled.value = !next;
      options.emitError(cause);
    }
  }

  void loadLocalTogglePreferences();

  async function reloadSettings() {
    transferDrafts.downloadDir = props.downloadPrefs.loadDir();
    transferDrafts.downloadUseDefault = props.downloadPrefs.loadUseDefault();
    transferDrafts.downloadConflict = props.downloadPrefs.loadConflict();
    transferDrafts.concurrency = String(props.transferPrefs.loadConcurrency());
    transferDrafts.duplicatePolicy = props.transferPrefs.loadDuplicatePolicy();
    transferDrafts.maxActive = String(props.transferPrefs.loadMaxActive());
    transferDrafts.downloadLimit = String(props.transferPrefs.loadDownloadLimit());
    transferDrafts.compatMode = props.transferPrefs.loadCompatMode();
    transferDrafts.nameEncoding = props.transferPrefs.loadNameEncoding();
    transferDrafts.compressMode = props.transferPrefs.loadCompressMode();
    transferDrafts.compressThreshold = String(props.transferPrefs.loadCompressThreshold());
    suggestionDrafts.enabled = props.suggestionPrefs.loadEnabled();
    suggestionDrafts.minChars = String(props.suggestionPrefs.loadMinChars());
    suggestionDrafts.maxChars = String(props.suggestionPrefs.loadMaxChars());
    transferDrafts.editorConfig = props.editorPrefs.loadConfig();
    transferDrafts.editorDefault = editorDefaultValue(transferDrafts.editorConfig);
    void props.editorPrefs.reloadEditors().then((list) => { transferDrafts.editorKnownEditors = list; });
    if (connection.state.loading || settingsSaving.value) return;
    // 每次打开都回到收起态，并丢弃上次遗留的内联编辑草稿：
    // 主「保存」会串行提交未保存的 profile 编辑，不能把陈旧草稿静默入库。
    sudo.state.inlineOpen = false;
    sudo.cancelProfileEdit();
    void loadKnownHosts();
    void loadLocalKeys();
    void loadMcpSettings();
    void sudo.loadSudoProfiles();
    await connection.load();
  }

  // 本地终端默认 Shell（设置·终端）：下拉项来自 local/shells/list（弹窗打开时
  // 拉一次）；localShell 偏好的权威态在 App，这里经 set-local-shell 上抛。
  const localShellOptions = ref<Array<{ program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean }>>([]);
  const localShellOptionsLoaded = ref(false);
  async function ensureLocalShellOptions() {
    if (localShellOptionsLoaded.value) return;
    localShellOptionsLoaded.value = true;
    try {
      const result = await window.dbxPlugin?.invoke<{ shells: typeof localShellOptions.value }>("local/shells/list", {}, { timeoutMs: 10_000 });
      localShellOptions.value = result?.shells ?? [];
    } catch {
      // 旧 sidecar 无 shell 发现能力：下拉只剩自动检测项（与工具条选择器同降级）。
      localShellOptions.value = [];
    }
  }

  watch(() => props.open, (open) => {
    if (open) {
      void reloadSettings();
      void ensureLocalShellOptions();
    }
  });

  // 弹窗开着时活跃会话切到另一连接：启动命令区按新 connectionId 重新回显。
  watch(() => props.connectionId, () => {
    if (props.open) {
      void connection.loadStartupCommands();
      void connection.loadConnNameEncoding();
    }
  });

  /** 快捷键整表替换：编辑器内部管草稿，只把最终结果上抛给 App 持久化。 */
  function updateHotkeys(bindings: TerminalHotkeyBindings) {
    options.emitHotkeys(bindings);
  }

  async function loadKnownHosts() {
    knownHostsLoading.value = true;
    knownHostsError.value = "";
    try {
      const result = await rpc<{ entries: KnownHostEntry[] }>("ssh/knownHosts/list", {});
      knownHosts.value = result.entries;
    } catch (cause) {
      knownHosts.value = [];
      knownHostsError.value = settingsErrorOf(cause);
    } finally {
      knownHostsLoading.value = false;
    }
  }

  async function removeKnownHost(entry: KnownHostEntry) {
    if (!(await confirmDialog(t("knownHosts.removeConfirm", { host: `${entry.host}:${entry.port}` })))) return;

    try {
      await rpc("ssh/knownHosts/remove", { host: entry.host, port: entry.port });
      options.emitNotice(t("knownHosts.removed", { host: `${entry.host}:${entry.port}` }));
    } catch (cause) {
      knownHostsError.value = settingsErrorOf(cause);
    } finally {
      await loadKnownHosts();
    }
  }

  async function loadLocalKeys() {
    localKeysLoading.value = true;
    localKeysError.value = "";
    try {
      const result = await rpc<{ keys: DiscoveredKey[] }>("keys/discover", {});
      localKeys.value = result.keys.map((key) => ({ ...key, hasPassphrase: key.hasPassphrase ?? key.has_passphrase === true }));
    } catch (cause) {
      localKeys.value = [];
      localKeysError.value = settingsErrorOf(cause);
    } finally {
      localKeysLoading.value = false;
    }
  }

  async function loadMcpSettings() {
    mcpLoading.value = true;
    mcpError.value = "";
    try {
      const result = await rpc<McpSizeSettings>("mcp/settings/get", {});
      mcpDraft.readMiB = mibField(result.maxReadBytes);
      mcpDraft.uploadMiB = mibField(result.maxUploadBytes);
      mcpDraft.downloadMiB = mibField(result.maxDownloadBytes);
      // §1.3 新字段：旧 sidecar 不回时用默认（autonomous / 空=不限）。
      mcpDraft.permissionMode = result.execPermissionMode === "confirm" ? "confirm" : "autonomous";
      mcpDraft.connectionScope = Array.isArray(result.connectionScope) ? result.connectionScope.join("\n") : "";
    } catch (cause) {
      mcpError.value = settingsErrorOf(cause);
    } finally {
      mcpLoading.value = false;
    }
  }

  async function saveMcpSettings() {
    if (!mcpInputsValid.value || mcpSaving.value) return;
    mcpSaving.value = true;
    mcpError.value = "";
    try {
      // 可同步子集一次成形：RPC（sidecar 即时权威）与 pluginStore 镜像（云同
      // 步载荷）双写同一份；localTransferRoot 不在保存参数里，由 sidecar 部分
      // 更新语义保留（存储迁移批 1，IMPL_PLAN_STORAGE_SYNC）。
      const updates = {
        maxReadBytes: Number.parseInt(mcpDraft.readMiB.trim(), 10) * MIB,
        maxUploadBytes: Number.parseInt(mcpDraft.uploadMiB.trim(), 10) * MIB,
        maxDownloadBytes: Number.parseInt(mcpDraft.downloadMiB.trim(), 10) * MIB,
        // §1.3 MCP 权限档 + 连接作用域（每行一条，trim 去空后提交；旧 sidecar
        // 不识别新字段时整体报错，经 mcpError 容错展示）。
        execPermissionMode: mcpDraft.permissionMode === "confirm" ? "confirm" : "autonomous",
        connectionScope: mcpDraft.connectionScope.split("\n").map((line) => line.trim()).filter(Boolean),
      };
      await rpc("mcp/settings/set", updates);
      persistMcpSettingsMirror({ ...updates });
      options.emitNotice(t("mcpLimits.saved"));
    } catch (cause) {
      mcpError.value = settingsErrorOf(cause);
    } finally {
      mcpSaving.value = false;
    }
  }

  /**
   * 一次保存链（设置弹窗主按钮）：① 未保存的 profile 编辑 → ② 连接设置 →
   * ③ MCP 限速。各步独立容错——saveProfileDraft/saveMcpSettings 内部已把失败
   * 写入 sudoProfilesError/mcpError 并展示，单步失败不阻断其余步骤；
   * MCP 表单非法时保持现有校验提示、静默跳过提交。
   */
  async function saveSettings() {
    // 本地 Shell 工作台没有 SSH 会话（settingsMeta 恒缺）：全局偏好面板必须
    // 可保存，只隔离会话级段（sudo/2FA/agent 更新依赖 sessionId）。
    if (settingsSaving.value || connection.state.loading || connection.state.loadFailed) return;
    if (!isLocalSession.value && !connection.state.meta) return;
    settingsSaving.value = true;
    try {
      props.downloadPrefs.persistDir(transferDrafts.downloadDir);
      props.downloadPrefs.persistUseDefault(transferDrafts.downloadUseDefault);
      props.downloadPrefs.persistConflict(transferDrafts.downloadConflict);
      props.transferPrefs.persistConcurrency(Number.parseInt(transferDrafts.concurrency, 10) || 3);
      props.transferPrefs.persistDuplicatePolicy(transferDrafts.duplicatePolicy);
      props.transferPrefs.persistMaxActive(Number.parseInt(transferDrafts.maxActive, 10) || 3);
      props.transferPrefs.persistDownloadLimit(Math.max(0, Number.parseInt(transferDrafts.downloadLimit, 10) || 0));
      props.transferPrefs.persistCompatMode(transferDrafts.compatMode);
      props.transferPrefs.persistNameEncoding(transferDrafts.nameEncoding);
      props.transferPrefs.persistCompressMode(transferDrafts.compressMode);
      props.transferPrefs.persistCompressThreshold(Math.min(65536, Math.max(0, Number.parseInt(transferDrafts.compressThreshold, 10) || 0)));
      props.suggestionPrefs.persistEnabled(suggestionDrafts.enabled);
      props.suggestionPrefs.persistMinChars(Number.parseInt(suggestionDrafts.minChars, 10) || 1);
      props.suggestionPrefs.persistMaxChars(Number.parseInt(suggestionDrafts.maxChars, 10) || 64);
      props.editorPrefs.persistConfig({
        ...transferDrafts.editorConfig,
        defaultEditorId: transferDrafts.editorDefault.startsWith("known:") ? transferDrafts.editorDefault.slice("known:".length) : undefined,
        defaultCustomId: transferDrafts.editorDefault.startsWith("custom:") ? transferDrafts.editorDefault.slice("custom:".length) : undefined,
        // editorDefaultDraft === "system"（或意外值）时不落默认编辑器字段。
      });
      if (sudo.state.editing) await sudo.saveProfileDraft();
      // 会话级段：本地 Shell 工作台整段跳过（分类已隐藏，无表单可改）。
      if (!isLocalSession.value) {
      const rememberedCommands = sanitizeRememberedCommands(connection.state.draft.rememberedCommands);
      const updates: Record<string, unknown> = {
        quickSudo: connection.state.draft.quickSudo,
        sudoUsePty: connection.state.draft.sudoUsePty,
        authFlowMode: connection.state.draft.authFlowMode,
        passwordPromptHint: connection.state.draft.passwordPromptHint,
        totpPromptHint: connection.state.draft.totpPromptHint,
        quickSudoProfileId: connection.state.draft.quickSudoProfileId,
        agentTerminalMode: connection.state.draft.agentTerminalMode,
        rememberedCommands,
      };
      if (connection.state.draft.sudoPassword) updates.sudoPassword = connection.state.draft.sudoPassword;
      if (connection.state.draft.totpSecret.trim()) updates.totpSecret = connection.state.draft.totpSecret;
      const meta = await rpc<SshSettings>("ssh/settings/set", { sessionId: props.sessionId, ...updates });
      connection.state.meta = meta;
      // 存储迁移批 2：保存成功即镜像连接级 agent 两域（云同步载荷）。
      if (props.connectionId) {
        saveConnectionEntry(SSH_AGENT_MODES_STORE_KEY, props.connectionId, sanitizeAgentTerminalMode(connection.state.draft.agentTerminalMode));
        saveConnectionEntry(SSH_AGENT_APPROVED_STORE_KEY, props.connectionId, rememberedCommands);
      }
      connection.state.draft.sudoPassword = "";
      connection.state.draft.totpSecret = "";
      }
      await saveMcpSettings();
      options.emitNotice(t("settingsSaved"));
    } catch (cause) {
      options.emitError(cause);
    } finally {
      settingsSaving.value = false;
    }
  }

  async function clearStoredSecrets() {
    if (isLocalSession.value || settingsSaving.value || connection.state.loading || connection.state.loadFailed || !connection.state.meta) return;
    try {
      const meta = await rpc<SshSettings>("ssh/settings/set", {
        sessionId: props.sessionId,
        sudoPassword: "",
        totpSecret: "",
      });
      connection.state.meta = meta;
      options.emitNotice(t("settingsSecretsCleared"));
    } catch (cause) {
      options.emitError(cause);
    }
  }

  /// Esc 分层退出：先关编辑表单，再收起配置档 section；返回 false 表示已到
  /// 最底层，调用方（App Esc 链）应关闭整个弹窗。
  function consumeInlineEsc(): boolean {
    if (sudo.state.inlineOpen && sudo.state.editing) {
      sudo.cancelProfileEdit();
      return true;
    }
    if (sudo.state.inlineOpen) {
      sudo.state.inlineOpen = false;
      return true;
    }
    return false;
  }

  /// 下载询问弹窗勾选「设为默认」/目录选择器回填后，App 同步设置页草稿
  /// （弹窗开着也能立即看到）。
  function setDownloadDirDraft(dir: string) {
    transferDrafts.downloadDir = dir;
  }

  function setDownloadUseDefaultDraft(value: boolean) {
    transferDrafts.downloadUseDefault = value;
  }

  return {
    connection,
    sudo,
    font,
    settingsSaving,
    transferDrafts,
    suggestionDrafts,
    commandHistoryLimitDraft,
    knownHosts,
    knownHostsLoading,
    knownHostsError,
    localKeys,
    localKeysLoading,
    localKeysError,
    mcpDraft,
    mcpLoading,
    mcpError,
    mcpInputsValid,
    x11Enabled,
    x11ReadOnlyNote,
    localTogglePrefsFailed,
    rdpExperimentalEnabled,
    autoRecordEnabled,
    localShellOptions,
    isLocalSession,
    reloadSettings,
    loadKnownHosts,
    loadLocalKeys,
    removeKnownHost,
    loadMcpSettings,
    saveSettings,
    clearStoredSecrets,
    consumeInlineEsc,
    setDownloadDirDraft,
    setDownloadUseDefaultDraft,
    updateHotkeys,
    setX11Enabled,
    setRdpExperimentalEnabled,
    setAutoRecordEnabled,
  };
}

export type SettingsDialogModel = ReturnType<typeof useSettingsDialogModel>;
