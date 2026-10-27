<script setup lang="ts">
import type { CSSProperties } from "vue";
import { AGENT_MODES } from "../../features/agent/lib/agentTerminal";
import type { AgentTerminalMode } from "../../features/agent/lib/agentTerminal";
import type { QuickCommand, QuickCommandGroupView } from "../../features/quick-commands/lib/quickCommands";
import type { FolderUploadProgress } from "../../features/sftp/lib/transfer/folderUpload";
import type { TerminalColorScheme } from "../../features/terminal/lib/appearance/terminalScheme";
import type { TransferHistoryEntry } from "../../features/transfers/lib/transferHistory";
import type { ResumableUploadTask } from "../../features/transfers/lib/transferResume";
import type { BatchUploadSummary, TransferTask } from "../../features/transfers/lib/transferQueue";
import DockerWhaleLogo from "../../features/docker/components/DockerWhaleLogo.vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import AppearanceMenu from "./AppearanceMenu.vue";
import ConnectionInfoPopover from "./ConnectionInfoPopover.vue";
import DockMetricsStrip from "./DockMetricsStrip.vue";
import QuickCommandsMenu from "./QuickCommandsMenu.vue";
import RdpToolbarControls from "./RdpToolbarControls.vue";
import SessionMenu from "./SessionMenu.vue";
import TransferPopover from "./TransferPopover.vue";
import type { SshWorkbenchPaneOrder } from "../lib/workbenchLayout";
import type {
  DockConnectionEntry,
  LocalShellEntry,
  WorkbenchToolbarActions,
} from "../lib/workbenchToolbarActions";

/** 工作台工具条外壳（T19 自 App.vue 模板迁入）：<header class="toolbar"> 整段，
 *  含身份区（连接名/会话徽章/Info）、SSH 视图段与主操作位、dock 面板变体
 *  （panel-toolbar-left 系统信息带 + panel-actions 精简组）与传输弹层单实例。
 *  会话下拉 / 快速命令 / RDP 工具组 / 配色快切拆成同级子组件。
 *
 *  数据面全部显式下传：只读态走 props，可写开关（各弹层 open、RDP 自定义宽高、
 *  快速命令搜索词）走 v-model 链，动作统一经 `actions` 类型化对象上抛——组件内
 *  不 inject、不读全局，也不持有会话逻辑，与迁移前 App 模板的绑定名逐字一致。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  panelSurface: boolean;
  panelToolbarHidden: boolean;
  toolbarStyle: CSSProperties | undefined;
  connection: {
    name?: string;
    host?: string;
    port?: number;
    username?: string;
    color?: string;
    readOnly?: boolean;
  };
  connectionIdentity: string;
  connectionReadOnly: boolean;
  sessionStatus: string;
  sessionPillText: string;
  reconnectCountdown: { seconds: number; attempt: number; percent: number } | null;
  metricsDistroBadge: { color: string; name: string; label: string } | null;
  connectionAuthMethodLabel: string;
  connectionLatency: number | null;
  connectionLatencyBusy: boolean;
  connectionLatencyFailed: boolean;
  connected: boolean;
  isRdpConnection: boolean;
  isRdpMode: boolean;
  isSerialMode: boolean;
  isVncMode: boolean;
  isTelnetMode: boolean;
  localUiMode: boolean;
  isLocalMode: boolean;
  localShellRestored: boolean;
  paneOrder: SshWorkbenchPaneOrder;
  sftpPaneOpen: boolean;
  sftpViewActive: boolean;
  rightPanelView: string;
  terminalState: string;
  reconnectPending: boolean;
  serialUploadBusy: boolean;
  connectionId: string;
  dockerPaneOpen: boolean;
  /** 批量发送条开合（props 名避开 "batch-bar-open" 子串：结构守卫据此
   *  断言批量条 DOM 已移出终端 pane，工具条只传状态）。 */
  batchSendBarOpen: boolean;
  agentMode: AgentTerminalMode;
  agentModeBusy: boolean;
  agentModeHint: string;
  recordingActive: boolean;
  recordingTarget: { kind: "ssh" | "local"; sessionId: string } | undefined;
  recordingsOpen: boolean;
  activeTransfers: number;
  // dock 面板系统信息带
  panelMetricsVisible: boolean;
  panelMetricsFailed: boolean;
  panelCpuPercent: number | null;
  panelMemPercent: number | null;
  panelNetRates: { rx: number; tx: number } | null;
  panelMetricsErrorTitle: string;
  formatRate: (value: number) => string;
  // dock 面板配色快切（转发 AppearanceMenu）
  panelSchemeChoices: TerminalColorScheme[];
  panelActiveSchemeId: string | null;
  // 会话下拉（转发 SessionMenu）
  localShells: LocalShellEntry[];
  localShellsLoading: boolean;
  localShellPref: string;
  localShellIntegrationPref: boolean;
  selectedShellInjectable: boolean | undefined;
  localShellCwdPref: string;
  localShellEnvPref: Array<{ name: string; value: string }>;
  localState: string;
  canOpenLocalTab: boolean;
  dockConnections: DockConnectionEntry[];
  // 快速命令下拉（转发 QuickCommandsMenu）
  quickCommands: QuickCommand[];
  filteredQuickCommands: QuickCommand[];
  groupedQuickCommands: QuickCommandGroupView[];
  quickHasRealGroups: boolean;
  quickCollapsedGroups: string[];
  quickExpandedId: string | null;
  quickCommandGroupSummary: string;
  // RDP 工具组（转发 RdpToolbarControls）
  resolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  rdpResizing: boolean;
  rdpDesktopSize: { width: number; height: number } | null;
  rdpTypingBusy: boolean;
  rdpTypingSent: number;
  rdpTypingTotal: number;
  // 传输弹层单实例（tab/panel 两表面共用）
  transferList: TransferTask[];
  transferHistory: TransferHistoryEntry[];
  resumableTasks: ResumableUploadTask[];
  folderUploadProgress: FolderUploadProgress | undefined;
  fileBatchProgress: BatchUploadSummary | undefined;
  pausedTaskIds: Set<string>;
  transferSpeeds: Record<string, number>;
  transferHistoryMenu: { taskId: string } | undefined;
  transferHistoryLoading: boolean;
  transferHistoryFailed: boolean;
  resumableLoading: boolean;
  folderUploadPercent: (state: FolderUploadProgress) => number;
  batchUploadPercent: (summary: BatchUploadSummary) => number;
  formatBytes: (value?: number) => string;
  transferPercent: (task: TransferTask) => number;
  transferBarValue: (task: TransferTask) => number | undefined;
  transferShownBytes: (task: TransferTask) => number;
  transferBytesTitle: (task: TransferTask) => string | undefined;
  transferPausable: (status: string) => boolean;
  actions: WorkbenchToolbarActions;
}>();

// 可写开关（App 持有状态）：v-model 链，赋值即上抛 update，行为与迁移前的直接赋值一致。
const connectionInfoOpen = defineModel<boolean>("connectionInfoOpen", { required: true });
const agentModeOpen = defineModel<boolean>("agentModeOpen", { required: true });
const importPanelOpen = defineModel<boolean>("importPanelOpen", { required: true });
const transferPanelOpen = defineModel<boolean>("transferPanelOpen", { required: true });
const transferHistoryClearOpen = defineModel<boolean>("transferHistoryClearOpen", { required: true });
const serialUploadDialogOpen = defineModel<boolean>("serialUploadDialogOpen", { required: true });
const forwardsOpen = defineModel<boolean>("forwardsOpen", { required: true });
const sessionMenuOpen = defineModel<boolean>("sessionMenuOpen", { required: true });
const sessionMenuShellOpen = defineModel<boolean>("sessionMenuShellOpen", { required: true });
const sessionMenuShellPickOpen = defineModel<boolean>("sessionMenuShellPickOpen", { required: true });
const localShellSurfaceOpen = defineModel<boolean>("localShellSurfaceOpen", { required: true });
const quickMenuOpen = defineModel<boolean>("quickMenuOpen", { required: true });
const quickSearch = defineModel<string>("quickSearch", { required: true });
const panelPaletteOpen = defineModel<boolean>("panelPaletteOpen", { required: true });
const rdpScaleMode = defineModel<"fit" | "stretch" | "actual">("rdpScaleMode", { required: true });
const rdpResolutionOpen = defineModel<boolean>("rdpResolutionOpen", { required: true });
const rdpCustomWidth = defineModel<string>("rdpCustomWidth", { required: true });
const rdpCustomHeight = defineModel<string>("rdpCustomHeight", { required: true });
const rdpPasteOpen = defineModel<boolean>("rdpPasteOpen", { required: true });
const rdpPasteText = defineModel<string>("rdpPasteText", { required: true });

const emit = defineEmits<{
  /** 传输历史右键菜单收起：写的是 App 的 ref，留在 App 侧收口。 */
  (e: "close-history-menu"): void;
}>();

const t = props.t;
const actions = props.actions;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  togglePanelToolbar,
  toggleConnectionInfo,
  measureLatency,
  togglePaneOrder,
  toggleSftpPane,
  adjustTerminalZoom,
  reconnectNow,
  sendSudoRefresh,
  openAlertTriage,
  toggleDockerPane,
  toggleBatchBar,
  toggleAgentModeMenu,
  applyAgentMode,
  toggleMetricsPanel,
  toggleRecording,
  toggleRecordings,
  openSettings,
  openAuditLog,
  toggleTransferPanel,
  showTransferHistoryMenu,
  toggleTransferPause,
  cancelTransfer,
  revealTransferTarget,
  openTransferTarget,
  refreshTransferPanel,
  beginResumeUpload,
  onResumeFilePicked,
  openNewSessionTab,
} = props.actions;
</script>

<template>
  <header v-if="!panelSurface || !panelToolbarHidden" class="toolbar" :style="toolbarStyle">
    <!-- 连接信息入口：Info 图标按钮紧跟标识区（状态徽章右侧），弹层左对齐锚定。
         Dock 面板表面整体换用下方 panel-actions 精简组（宿主 chrome 已有标题/关闭）。 -->
    <div v-if="!panelSurface" class="identity-side">
      <div class="identity">
        <span v-if="connection.color" class="connection-color" :style="{ backgroundColor: connection.color }" />
        <!-- 主标题跟随 ConnectingCard 的口径：连接名优先，未命名回退 user@host:port；
             hover 仍可看到技术标识（UUID 只在名称与 host 全缺失时兜底出现）。 -->
        <strong :title="connectionIdentity">{{ connection.name || connectionIdentity }}</strong>
        <span v-if="connection.readOnly || connectionReadOnly" class="read-only-badge">{{ t("readOnly") }}</span>
        <span class="session-pill" :class="`session-${sessionStatus}`"><span class="session-dot" aria-hidden="true" />{{ sessionPillText }}<span v-if="sessionStatus === 'reconnecting' && reconnectCountdown" class="session-pill-countdown mono">{{ t("sessionStatus.reconnectCountdown", { seconds: reconnectCountdown.seconds, attempt: reconnectCountdown.attempt }) }}</span></span>
      </div>
      <ConnectionInfoPopover
        :connection-info-open="connectionInfoOpen"
        :connection="connection"
        :metrics-distro-badge="metricsDistroBadge"
        :connection-auth-method-label="connectionAuthMethodLabel"
        :connection-read-only="connectionReadOnly"
        :connection-latency="connectionLatency"
        :connection-latency-busy="connectionLatencyBusy"
        :connection-latency-failed="connectionLatencyFailed"
        :connected="connected"
        :t="t"
        @update:connectionInfoOpen="(value) => { if (!value) connectionInfoOpen = false; }"
        @toggle="toggleConnectionInfo"
        @measure="measureLatency"
      />
    </div>
    <div v-if="!panelSurface" class="toolbar-actions">
      <!-- —— 视图段（分栏互换 / SFTP 面板 / 字号）：连接协议门控（!isRdpConnection）
           在组级判一次；RDP 连接下整组不渲染，不产生孤儿按钮。 -->
      <template v-if="!isRdpConnection">
        <button class="icon-button icon-neutral" :title="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" :aria-label="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" :disabled="panelSurface" @click="togglePaneOrder"><WorkbenchGlyph name="pane-swap" /></button>
        <!-- Local terminal UI hides SSH-only actions outright (not disabled): the local
             shell has no SSH session to act on. -->
        <button v-if="!localUiMode" class="icon-button icon-cyan" :class="{ 'is-active': sftpPaneOpen && sftpViewActive }" :title="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-label="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-pressed="sftpPaneOpen && sftpViewActive" :disabled="panelSurface" @click="toggleSftpPane"><WorkbenchGlyph :name="sftpPaneOpen && sftpViewActive ? 'panel-close' : 'panel-sftp'" /></button>
        <button class="icon-button" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
        <button class="icon-button" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
      </template>
      <!-- —— RDP 独立工具组（isRdpMode 会话激活期）：缩放三档 + 桌面分辨率 +
           大文本粘贴 + 断开。与 SSH 组互斥、各判一次门控；SSH 工具整类隐藏后，
           这是远程桌面自己需要的操作面。 -->
      <RdpToolbarControls
        v-if="isRdpMode"
        v-model:rdp-scale-mode="rdpScaleMode"
        v-model:rdp-resolution-open="rdpResolutionOpen"
        v-model:rdp-custom-width="rdpCustomWidth"
        v-model:rdp-custom-height="rdpCustomHeight"
        v-model:rdp-paste-open="rdpPasteOpen"
        v-model:rdp-paste-text="rdpPasteText"
        :t="t"
        :resolution-presets="resolutionPresets"
        :rdp-resizing="rdpResizing"
        :rdp-desktop-size="rdpDesktopSize"
        :rdp-typing-busy="rdpTypingBusy"
        :rdp-typing-sent="rdpTypingSent"
        :rdp-typing-total="rdpTypingTotal"
        :actions="actions"
      />
      <!-- 工具条语义分组分隔线：两侧（视图段/会话菜单）在 SSH 与 RDP 两种形态
           下都各有一侧可见，永远不会成为孤儿分隔符。 -->
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 会话下拉：新建 / 复制 / 命令会话 / 本地终端 / Shell 选项 / 导入会话
           收敛为单入口。Shell 选项为同弹层视图切换（数据与 local/shells/list
           懒加载逻辑不变）。 -->
      <SessionMenu
        v-model:session-menu-open="sessionMenuOpen"
        v-model:session-menu-shell-open="sessionMenuShellOpen"
        v-model:session-menu-shell-pick-open="sessionMenuShellPickOpen"
        v-model:local-shell-surface-open="localShellSurfaceOpen"
        :t="t"
        :connection-id="connectionId"
        :connected="connected"
        :local-ui-mode="localUiMode"
        :is-rdp-connection="isRdpConnection"
        :is-rdp-mode="isRdpMode"
        :is-vnc-mode="isVncMode"
        :is-serial-mode="isSerialMode"
        :is-telnet-mode="isTelnetMode"
        :local-shell-restored="localShellRestored"
        :is-local-mode="isLocalMode"
        :local-shells="localShells"
        :local-shells-loading="localShellsLoading"
        :local-shell-pref="localShellPref"
        :local-shell-integration-pref="localShellIntegrationPref"
        :selected-shell-injectable="selectedShellInjectable"
        :local-shell-cwd-pref="localShellCwdPref"
        :local-shell-env-pref="localShellEnvPref"
        :local-state="localState"
        :can-open-local-tab="canOpenLocalTab"
        :dock-connections="dockConnections"
        :actions="actions"
        @open-import="importPanelOpen = true"
      />
      <!-- —— SSH 工具组·运维 / 效率 / 监控 / 文件与系统：组级 !isRdpConnection
           判一次，段内三条语义分隔线随组渲染；组内按钮只保留
           localUiMode/isSerialMode 等自身条件。 -->
      <template v-if="!isRdpConnection">
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 连接与运维 -->
      <!-- 串口文件上传入口（NyaTerm 对齐 P0-3）：仅串口模式可用；传输中禁发。 -->
      <button v-if="isSerialMode" class="icon-button icon-emerald" :title="t('serial.upload.open')" :aria-label="t('serial.upload.open')" :disabled="serialUploadBusy" @click="serialUploadDialogOpen = true"><WorkbenchGlyph name="upload-file" /></button>
      <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('reconnect')" :aria-label="t('reconnect')" :disabled="terminalState === 'connecting' && !reconnectPending" @click="reconnectNow"><WorkbenchGlyph name="reconnect" /></button>
      <!-- 一键 sudo -v：向当前 PTY 写入命令刷新 sudo 凭据缓存；quick sudo 自动应答
           是否启用由连接设置决定（设置弹窗），工作台不再提供开关。 -->
      <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('sudoRefresh.title')" :aria-label="t('sudoRefresh.title')" :disabled="!connected" @click="sendSudoRefresh"><WorkbenchGlyph name="sudo-shield" /></button>
      <!-- main 新增的端口转发入口同属 SSH 专属：沿用 A4 惯例在本地模式整体隐藏。 -->
      <button v-if="!localUiMode" class="icon-button icon-cyan" :title="t('forwards.title')" :aria-label="t('forwards.title')" :disabled="!connectionId" @click="forwardsOpen = true"><WorkbenchGlyph name="forwards" /></button>
      <button v-if="!localUiMode" class="icon-button icon-cyan" :title="t('alertTriage.title')" :aria-label="t('alertTriage.title')" @click="openAlertTriage"><WorkbenchGlyph name="alert" /></button>
      <!-- 容器管理底部分屏入口（鲸鱼 logo）：独立开关，不占用右侧栏、不与
           SFTP/指标互斥；local 模式保留——本机 daemon（Docker
           Desktop/OrbStack）场景照常可用。面板自治，收起即卸载停轮询。 -->
      <button class="icon-button icon-blue" :class="{ 'is-active': dockerPaneOpen }" :title="t('docker.toolbarTitle')" :aria-label="t('docker.toolbarTitle')" :aria-pressed="dockerPaneOpen" @click.stop="toggleDockerPane"><DockerWhaleLogo /></button>
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 效率工具：快命令 → 批量发送 → Agent 模式 -->
      <QuickCommandsMenu
        v-model:quick-menu-open="quickMenuOpen"
        v-model:quick-search="quickSearch"
        :t="t"
        :connected="connected"
        :quick-commands="quickCommands"
        :filtered-quick-commands="filteredQuickCommands"
        :grouped-quick-commands="groupedQuickCommands"
        :quick-has-real-groups="quickHasRealGroups"
        :quick-collapsed-groups="quickCollapsedGroups"
        :quick-expanded-id="quickExpandedId"
        :quick-command-group-summary="quickCommandGroupSummary"
        :actions="actions"
      />
      <button v-if="!localUiMode && !panelSurface" class="icon-button icon-neutral" :class="{ 'is-active': batchSendBarOpen }" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" :aria-pressed="batchSendBarOpen" :disabled="!connected" @click="toggleBatchBar"><WorkbenchGlyph name="batch-send" /></button>
      <div>
        <Popover :open="agentModeOpen" @update:open="(open) => { if (!open) agentModeOpen = false; }">
          <PopoverAnchor as-child>
            <button class="icon-button" :class="agentMode === 'off' ? 'icon-neutral' : 'icon-emerald is-active'" :title="t('agentTerminalQuickHint')" :aria-label="t('agentTerminalQuickHint')" :aria-pressed="agentMode !== 'off'" :disabled="!connected" @pointerdown.stop @click.stop="toggleAgentModeMenu"><WorkbenchGlyph name="agent" /></button>
          </PopoverAnchor>
          <PopoverContent class="popover agent-mode-popover" align="end" :side-offset="5">
          <h3>{{ t("agentTerminalSection") }}</h3>
          <label v-for="mode in AGENT_MODES" :key="mode" class="agent-mode-option">
            <input type="radio" name="agent-mode" :checked="agentMode === mode" :disabled="agentModeBusy" @change="applyAgentMode(mode)" />
            <span>{{ t(`agentTerminal${mode === "off" ? "Off" : mode === "auto" ? "Auto" : "Strict"}`) }}</span>
          </label>
          <p class="muted agent-mode-note">{{ agentModeHint }}</p>
          </PopoverContent>
        </Popover>
      </div>
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 监控与记录 -->
      <button class="icon-button icon-emerald" :class="{ 'is-active': sftpPaneOpen && rightPanelView === 'metrics' }" :title="t('metrics')" :aria-label="t('metrics')" :aria-pressed="sftpPaneOpen && rightPanelView === 'metrics'" :disabled="!connected" @click="toggleMetricsPanel"><WorkbenchGlyph name="metrics" /></button>
      <button class="icon-button" :class="{ 'is-recording': recordingActive }" :title="recordingActive ? t('recordingStop') : t('recordingTitle')" :aria-label="recordingActive ? t('recordingStop') : t('recordingTitle')" :disabled="!recordingTarget" @click="toggleRecording"><WorkbenchGlyph name="record" /></button>
      <button class="icon-button" :class="{ 'is-active': recordingsOpen }" :title="t('recordingsTitle')" :aria-label="t('recordingsTitle')" :aria-pressed="recordingsOpen" @click="toggleRecordings"><WorkbenchGlyph name="recordings" /></button>
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 文件与系统：传输 → 审计 → 设置（设置固定在右缘） -->
      <div>
        <!-- 历史卡右键菜单（ContextMenu）打开时忽略弹层的外点关闭请求：
             菜单项 pointerdown 相对弹层是"外部"，不加守卫会在 select 前把宿主弹层
             连同菜单一起卸载，动作丢失。 -->
        <button class="icon-button icon-blue" :title="t('transfers')" :aria-label="t('transfers')" :disabled="panelSurface" @pointerdown.stop @click.stop="toggleTransferPanel"><WorkbenchGlyph name="transfers" /><span v-if="activeTransfers" class="activity-dot" /></button>
      </div>
      <!-- 设置弹窗承载 SSH/SFTP/终端专属分节；审计日志只记录 SSH/SFTP 操作。 -->
      <button class="icon-button icon-amber" :title="t('auditLog.title')" :aria-label="t('auditLog.title')" @click="openAuditLog"><WorkbenchGlyph name="audit" /></button>
      <button class="icon-button icon-violet" :title="t('settings')" :aria-label="t('settings')" @click="openSettings"><WorkbenchGlyph name="settings" /></button>
      </template>
    </div>
    <!-- dock 面板左组：系统信息带（CPU/内存/网速，5s 轮询），flex:1 把
         右侧按钮组推到右缘；数据未就绪/不支持时整条隐藏不占位。 -->
    <div v-if="panelSurface" class="panel-toolbar-left">
      <DockMetricsStrip
        :panel-metrics-visible="panelMetricsVisible"
        :panel-metrics-failed="panelMetricsFailed"
        :panel-cpu-percent="panelCpuPercent"
        :panel-mem-percent="panelMemPercent"
        :panel-net-rates="panelNetRates"
        :panel-metrics-error-title="panelMetricsErrorTitle"
        :format-rate="formatRate"
        :t="t"
      />
    </div>
    <!-- Dock 面板表面（底部栏）的精简工具区：宿主 dock chrome 已带标题与
         关闭/收起/最大化，这里只留终端快捷操作——字号、录屏、传输、配色快切、
         设置与工具条收起，密度对齐底部栏（紧凑 30px 工具条 + 透明 ghost 按钮）。 -->
    <div v-if="panelSurface" class="toolbar-actions panel-actions">
      <!-- 面板紧凑组：新标签 → SFTP → 字号 → 录制/传输 → 配色 → 设置 → 收起。 -->
      <button v-if="!localUiMode && !isRdpConnection" class="icon-button compact" :title="t('newSessionTab')" :aria-label="t('newSessionTab')" :disabled="!connectionId" @click="openNewSessionTab"><WorkbenchGlyph name="new-tab" /></button>
      <button v-if="!localUiMode && !isRdpConnection" class="icon-button compact" :class="{ 'is-active': sftpPaneOpen && sftpViewActive }" :title="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-label="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-pressed="sftpPaneOpen && sftpViewActive" @click="toggleSftpPane"><WorkbenchGlyph :name="sftpPaneOpen && sftpViewActive ? 'panel-close' : 'panel-sftp'" /></button>
      <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
      <button v-if="!isRdpMode" class="icon-button compact" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
      <button v-if="!isRdpConnection" class="icon-button compact" :class="{ 'is-recording': recordingActive }" :title="recordingActive ? t('recordingStop') : t('recordingTitle')" :aria-label="recordingActive ? t('recordingStop') : t('recordingTitle')" :disabled="!recordingTarget" @click="toggleRecording"><WorkbenchGlyph name="record" /></button>
      <button v-if="!isRdpConnection" class="icon-button compact" :title="t('transfers')" :aria-label="t('transfers')" @pointerdown.stop @click.stop="toggleTransferPanel"><WorkbenchGlyph name="transfers" /><span v-if="activeTransfers" class="activity-dot" /></button>
      <AppearanceMenu
        v-model:panel-palette-open="panelPaletteOpen"
        :t="t"
        :is-rdp-mode="isRdpMode"
        :panel-scheme-choices="panelSchemeChoices"
        :panel-active-scheme-id="panelActiveSchemeId"
        :actions="actions"
      />
      <button v-if="!isRdpConnection" class="icon-button compact" :title="t('settings')" :aria-label="t('settings')" @click="openSettings"><WorkbenchGlyph name="settings" /></button>
      <button class="icon-button compact" :title="t('panel.hideToolbar')" :aria-label="t('panel.hideToolbar')" :aria-pressed="panelToolbarHidden" @click="togglePanelToolbar"><WorkbenchGlyph name="collapse-up" /></button>
    </div>
    <!-- 传输弹层单实例（tab/panel 两表面共用）：Popover 挂在 header 层，锚点是
         工具条右缘下的隐形定位点（.transfer-popover-anchor，固定定位不随表面
         工具条高度变化）；tab 工具条与 dock panel-actions 里的按钮都只是
         toggleTransferPanel 的普通开关——panel 没有这组 tab 工具条实例，
         共享锚点让同一个弹层在两个表面都能弹出（内容单拷贝零复制）。 -->
    <TransferPopover
      :transfer-panel-open="transferPanelOpen"
      :transfer-list="transferList"
      :transfer-history="transferHistory"
      :resumable-tasks="resumableTasks"
      :folder-upload-progress="folderUploadProgress"
      :file-batch-progress="fileBatchProgress"
      :paused-task-ids="pausedTaskIds"
      :transfer-speeds="transferSpeeds"
      :transfer-history-menu="transferHistoryMenu"
      :transfer-history-loading="transferHistoryLoading"
      :transfer-history-failed="transferHistoryFailed"
      :resumable-loading="resumableLoading"
      :folder-upload-percent="folderUploadPercent"
      :batch-upload-percent="batchUploadPercent"
      :format-bytes="formatBytes"
      :transfer-percent="transferPercent"
      :transfer-bar-value="transferBarValue"
      :transfer-shown-bytes="transferShownBytes"
      :transfer-bytes-title="transferBytesTitle"
      :transfer-pausable="transferPausable"
      :t="t"
      @update:transferPanelOpen="(value) => { if (!value && !transferHistoryMenu) transferPanelOpen = false; }"
      @close-history-menu="emit('close-history-menu')"
      @show-history-menu="showTransferHistoryMenu"
      @toggle-pause="toggleTransferPause"
      @cancel="cancelTransfer"
      @reveal="revealTransferTarget"
      @open-file="openTransferTarget"
      @refresh="refreshTransferPanel"
      @open-clear="transferHistoryClearOpen = true"
      @begin-resume="beginResumeUpload"
      @resume-picked="onResumeFilePicked"
    />
  </header>
</template>

<style scoped>
/* 工具栏 A+/A- 字号步进按钮（复用 Ctrl+滚轮的 clampFontSize 语义） */
.font-step-label { font-size: 11px; font-weight: 600; line-height: 1; letter-spacing: 0; }
/* 录制中图标按钮（原 App.vue scoped 样式随 DOM 迁入；dock 面板的
   .workbench.panel-surface 覆盖仍在全局 toolbar.css）。 */
.is-recording { color: var(--destructive); }
/* 终端 MCP 模式快速开关（工具栏弹出层，与设置弹窗共用三档文案；根规则见 style.css 全局） */
.agent-mode-popover h3 { margin: 4px 0 8px; font-size: 12px; }
.agent-mode-option { display: flex; align-items: center; gap: 8px; padding: 4px 0; font-size: 12px; cursor: pointer; }
.agent-mode-note { margin: 8px 0 0; font-size: 11px; }
</style>
