<script setup lang="ts">
import type { CSSProperties } from "vue";
import { AGENT_MODES } from "../../features/agent/lib/agentTerminal";
import type { AgentTerminalMode } from "../../features/agent/lib/agentTerminal";
import type { QuickCommand, QuickCommandGroup } from "../../features/quick-commands/lib/quickCommands";
import type { FolderUploadProgress } from "../../features/sftp/lib/transfer/folderUpload";
import type { TransferHistoryEntry } from "../../features/transfers/lib/transferHistory";
import type { ResumableUploadTask } from "../../features/transfers/lib/transferResume";
import type { BatchUploadSummary, TransferTask } from "../../features/transfers/lib/transferQueue";
import DockerWhaleLogo from "../../features/docker/components/DockerWhaleLogo.vue";
import PortForwardMenu from "../../features/forwarding/components/PortForwardMenu.vue";
import type { PortForwardsController } from "../../features/forwarding/composables/usePortForwards";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import ConnectionInfoPopover from "./ConnectionInfoPopover.vue";
import QuickCommandsMenu from "./QuickCommandsMenu.vue";
import RecordingMenu from "../../features/recording/components/RecordingMenu.vue";
import RdpToolbarControls from "./RdpToolbarControls.vue";
import VncToolbarControls from "./VncToolbarControls.vue";
import SessionMenu from "./SessionMenu.vue";
import TransferPopover from "./TransferPopover.vue";
import type { SshWorkbenchPaneOrder } from "../lib/workbenchLayout";
import type {
  DockConnectionEntry,
  LocalShellEntry,
  WorkbenchToolbarActions,
} from "../lib/workbenchToolbarActions";

/** 工作台工具条外壳（T19 自 App.vue 模板迁入）：<header class="toolbar"> 整段，
 *  含身份区（连接名/会话徽章/Info）、SSH 视图段与主操作位、传输弹层单实例。
 *  会话下拉 / 快速命令 / RDP / VNC 工具组拆成同级子组件；dock 面板不渲染本
 *  组件（顶部插件工具条整行消失）。
 *
 *  数据面全部显式下传：只读态走 props，可写开关（各弹层 open、RDP 自定义宽高、
 *  快速命令搜索词）走 v-model 链，动作统一经 `actions` 类型化对象上抛——组件内
 *  不 inject、不读全局，也不持有会话逻辑，与迁移前 App 模板的绑定名逐字一致。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
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
  /** VNC 连接级门控（协议命中或会话激活）：SSH/终端工具组整组隐藏。 */
  isVncConnection: boolean;
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
  recordCountdown: number | null;
  recordingElapsedSec: number;
  recordingsOpen: boolean;
  recordingsCount: number;
  /** 端口映射控制器：工具条按钮徽标与快捷弹层共用工作台的同一实例。 */
  forwards: PortForwardsController;
  activeTransfers: number;
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
  // 命令管理弹层（转发 QuickCommandsMenu）
  quickCommands: QuickCommand[];
  quickGroups: QuickCommandGroup[];
  quickSaving: boolean;
  quickImporting: boolean;
  // RDP 工具组（转发 RdpToolbarControls）
  applePlatform: boolean;
  rdpNativeFileClipboard: boolean;
  rdpServerFileClipboard: boolean;
  rdpServerFileClipboardKnown: boolean;
  rdpClipboardEnabled: boolean;
  resolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  rdpResizing: boolean;
  rdpAutoFit: boolean;
  rdpDesktopSize: { width: number; height: number } | null;
  rdpTypingBusy: boolean;
  rdpTypingSent: number;
  rdpTypingTotal: number;
  // VNC 工具组（转发 VncToolbarControls）
  vncResolutionPresets: ReadonlyArray<{ width: number; height: number }>;
  vncResizing: boolean;
  vncCanResize: boolean;
  vncAutoFit: boolean;
  vncDesktopSize: { width: number; height: number } | null;
  // 传输弹层单实例（工具条内）
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
  transferShownBytes: (task: TransferTask) => number;
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
const forwardsMenuOpen = defineModel<boolean>("forwardsMenuOpen", { required: true });
const forwardsOpen = defineModel<boolean>("forwardsOpen", { required: true });
const forwardsFocusForm = defineModel<boolean>("forwardsFocusForm", { required: true });
const sessionMenuOpen = defineModel<boolean>("sessionMenuOpen", { required: true });
const sessionMenuShellOpen = defineModel<boolean>("sessionMenuShellOpen", { required: true });
const sessionMenuShellPickOpen = defineModel<boolean>("sessionMenuShellPickOpen", { required: true });
const localShellSurfaceOpen = defineModel<boolean>("localShellSurfaceOpen", { required: true });
const quickMenuOpen = defineModel<boolean>("quickMenuOpen", { required: true });
const rdpScaleMode = defineModel<"fit" | "stretch" | "actual">("rdpScaleMode", { required: true });
const rdpResolutionOpen = defineModel<boolean>("rdpResolutionOpen", { required: true });
const rdpCustomWidth = defineModel<string>("rdpCustomWidth", { required: true });
const rdpCustomHeight = defineModel<string>("rdpCustomHeight", { required: true });
const rdpPasteOpen = defineModel<boolean>("rdpPasteOpen", { required: true });
const rdpPasteText = defineModel<string>("rdpPasteText", { required: true });
const rdpFileHintOpen = defineModel<boolean>("rdpFileHintOpen", { required: true });
const vncScaleMode = defineModel<"fit" | "stretch" | "actual">("vncScaleMode", { required: true });
const vncResolutionOpen = defineModel<boolean>("vncResolutionOpen", { required: true });
const vncCustomWidth = defineModel<string>("vncCustomWidth", { required: true });
const vncCustomHeight = defineModel<string>("vncCustomHeight", { required: true });

const emit = defineEmits<{
  /** 传输历史右键菜单收起：写的是 App 的 ref，留在 App 侧收口。 */
  (e: "close-history-menu"): void;
}>();

const t = props.t;
const actions = props.actions;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  toggleConnectionInfo,
  measureLatency,
  togglePaneOrder,
  toggleSftpPane,
  adjustTerminalZoom,
  reconnectNow,
  sendSudoRefresh,
  toggleDockerPane,
  toggleBatchBar,
  toggleAgentModeMenu,
  toggleForwardsMenu,
  applyAgentMode,
  toggleMetricsPanel,
  toggleRecording,
  toggleRecordings,
  cancelRecordCountdown,
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
const recordingActions = { toggleRecording, toggleRecordings, cancelRecordCountdown };

function openForwardManager(mode: "add" | "manage") {
  forwardsMenuOpen.value = false;
  forwardsFocusForm.value = mode === "add";
  forwardsOpen.value = true;
}
</script>

<template>
  <header class="toolbar" :style="toolbarStyle">
    <!-- 连接信息入口：Info 图标按钮紧跟标识区（状态徽章右侧），弹层左对齐锚定。 -->
    <div class="identity-side">
      <div class="identity">
        <span v-if="connection.color" class="connection-color" :style="{ backgroundColor: connection.color }" />
        <!-- 主标题跟随 ConnectingCard 的口径：连接名优先，未命名回退 user@host:port；
             hover 仍可看到技术标识（UUID 只在名称与 host 全缺失时兜底出现）。 -->
        <strong :title="connectionIdentity">{{ connection.name || connectionIdentity }}</strong>
        <span v-if="isRdpConnection || isVncConnection" class="protocol-badge" :class="isRdpConnection ? 'protocol-badge-rdp' : 'protocol-badge-vnc'">
          <WorkbenchGlyph :name="isRdpConnection ? 'protocol-rdp' : 'protocol-vnc'" :size="12" />
          <span class="protocol-badge-label">{{ isRdpConnection ? "RDP" : "VNC" }}</span>
        </span>
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
    <div class="toolbar-actions">
      <!-- —— 视图段（分栏互换 / 字号）：SFTP 面板入口已移到右缘文件组；
           连接协议门控（!isRdpConnection && !isVncConnection）在组级判一次，RDP 连接下整组不渲染。 -->
      <template v-if="!isRdpConnection && !isVncConnection">
        <button class="icon-button icon-neutral" :title="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" :aria-label="paneOrder === 'terminal-left' ? t('moveSftpLeft') : t('moveTerminalLeft')" @click="togglePaneOrder"><WorkbenchGlyph name="pane-swap" /></button>
        <button class="icon-button" :title="t('terminalFontDecrease')" :aria-label="t('terminalFontDecrease')" @click="adjustTerminalZoom(-1)"><span class="font-step-label" aria-hidden="true">A−</span></button>
        <button class="icon-button" :title="t('terminalFontIncrease')" :aria-label="t('terminalFontIncrease')" @click="adjustTerminalZoom(1)"><span class="font-step-label" aria-hidden="true">A+</span></button>
      </template>
      <!-- —— RDP 独立工具组（isRdpMode 会话激活期）：缩放三档 + 桌面分辨率 +
           大文本粘贴 + 断开。与 SSH 组互斥、各判一次门控。 -->
      <RdpToolbarControls
        v-if="isRdpMode"
        v-model:rdp-scale-mode="rdpScaleMode"
        v-model:rdp-resolution-open="rdpResolutionOpen"
        v-model:rdp-custom-width="rdpCustomWidth"
        v-model:rdp-custom-height="rdpCustomHeight"
        v-model:rdp-paste-open="rdpPasteOpen"
        v-model:rdp-paste-text="rdpPasteText"
        v-model:rdp-file-hint-open="rdpFileHintOpen"
        :t="t"
        :apple-platform="applePlatform"
        :native-file-clipboard="rdpNativeFileClipboard"
        :server-file-clipboard="rdpServerFileClipboard"
        :server-file-clipboard-known="rdpServerFileClipboardKnown"
        :clipboard-enabled="rdpClipboardEnabled"
        :resolution-presets="resolutionPresets"
        :rdp-resizing="rdpResizing"
        :rdp-auto-fit="rdpAutoFit"
        :rdp-desktop-size="rdpDesktopSize"
        :rdp-typing-busy="rdpTypingBusy"
        :rdp-typing-sent="rdpTypingSent"
        :rdp-typing-total="rdpTypingTotal"
        :actions="actions"
      />
      <!-- —— VNC 独立工具组（isVncMode 会话激活期）：缩放三档 + 桌面分辨率 +
           断开。与 SSH 组互斥、与 RDP 组互斥。 -->
      <VncToolbarControls
        v-if="isVncMode"
        v-model:vnc-scale-mode="vncScaleMode"
        v-model:vnc-resolution-open="vncResolutionOpen"
        v-model:vnc-custom-width="vncCustomWidth"
        v-model:vnc-custom-height="vncCustomHeight"
        :t="t"
        :resolution-presets="vncResolutionPresets"
        :vnc-resizing="vncResizing"
        :vnc-can-resize="vncCanResize"
        :vnc-auto-fit="vncAutoFit"
        :vnc-desktop-size="vncDesktopSize"
        :actions="actions"
      />
      <!-- 工具条语义分组分隔线：两侧（视图段/会话菜单）在 SSH 与远程桌面两种
           形态下都各有一侧可见，永远不会成为孤儿分隔符。 -->
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
      <!-- —— SSH 工具组·连接运维 / 效率 / 录制 / 右缘固定组：组级
           !isRdpConnection && !isVncConnection 判一次；组内按钮只保留 localUiMode/isSerialMode
           等自身条件。右缘固定组按「设置 → 审计 → SFTP 面板 → 传输 →
           容器 → 指标」从右往左排列，SFTP 面板与传输紧邻。 -->
      <template v-if="!isRdpConnection && !isVncConnection">
      <!-- 本地 shell 没有 SSH 会话，运维组为空：分隔线随组隐藏，避免双分隔线。 -->
      <span v-if="!localUiMode || isSerialMode" class="toolbar-separator" aria-hidden="true" />
      <!-- 连接与运维 -->
      <!-- 串口文件上传入口（NyaTerm 对齐 P0-3）：仅串口模式可用；传输中禁发。 -->
      <button v-if="isSerialMode" class="icon-button icon-emerald" :title="t('serial.upload.open')" :aria-label="t('serial.upload.open')" :disabled="serialUploadBusy" @click="serialUploadDialogOpen = true"><WorkbenchGlyph name="upload-file" /></button>
      <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('reconnect')" :aria-label="t('reconnect')" :disabled="terminalState === 'connecting' && !reconnectPending" @click="reconnectNow"><WorkbenchGlyph name="reconnect" /></button>
      <!-- 一键 sudo -v：向当前 PTY 写入命令刷新 sudo 凭据缓存。 -->
      <button v-if="!localUiMode" class="icon-button icon-emerald" :title="t('sudoRefresh.title')" :aria-label="t('sudoRefresh.title')" :disabled="!connected" @click="sendSudoRefresh"><WorkbenchGlyph name="sudo-shield" /></button>
      <!-- 端口转发入口：点击先出快捷弹层，运行中数量直接标在图标上。 -->
      <div v-if="!localUiMode && !isSerialMode && !isTelnetMode && !isVncMode">
        <Popover :open="forwardsMenuOpen" @update:open="(open) => { if (!open) forwardsMenuOpen = false; }">
          <PopoverAnchor as-child>
            <button
              class="icon-button forward-toolbar-button"
              :class="forwards.summary.value.running ? 'icon-cyan is-active' : 'icon-cyan'"
              :title="t('forwards.title')"
              :aria-label="t('forwards.title')"
              :aria-pressed="forwardsMenuOpen"
              :disabled="!connectionId"
              @pointerdown.stop
              @click.stop="toggleForwardsMenu"
            >
              <WorkbenchGlyph name="forwards" />
              <span v-if="forwards.summary.value.running" class="toolbar-count">{{ forwards.summary.value.running }}</span>
            </button>
          </PopoverAnchor>
          <PopoverContent class="popover forward-menu-popover" align="end" :side-offset="5">
            <PortForwardMenu :controller="forwards" :t="t" @add="openForwardManager('add')" @manage="openForwardManager('manage')" />
          </PopoverContent>
        </Popover>
      </div>
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 效率工具：快命令 → 批量发送 → Agent 模式 -->
      <QuickCommandsMenu
        v-model:quick-menu-open="quickMenuOpen"
        :t="t"
        :connected="connected"
        :quick-commands="quickCommands"
        :quick-groups="quickGroups"
        :quick-saving="quickSaving"
        :quick-importing="quickImporting"
        :actions="actions"
      />
      <button v-if="!localUiMode" class="icon-button icon-neutral" :class="{ 'is-active': batchSendBarOpen }" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" :aria-pressed="batchSendBarOpen" :disabled="!connected" @click="toggleBatchBar"><WorkbenchGlyph name="batch-send" /></button>
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
      <!-- 录制 -->
      <RecordingMenu
        :t="t"
        :can-record="!!recordingTarget"
        :recording-active="recordingActive"
        :record-countdown="recordCountdown"
        :recording-elapsed-sec="recordingElapsedSec"
        :recordings-count="recordingsCount"
        :recordings-open="recordingsOpen"
        :actions="recordingActions"
      />
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 右缘固定组（从右往左：设置 → 审计 → SFTP 面板 → 传输 → 容器 → 指标）：
           指标/容器一组，传输/SFTP 面板紧邻，审计/设置固定在右缘。 -->
      <button class="icon-button icon-emerald" :class="{ 'is-active': sftpPaneOpen && rightPanelView === 'metrics' }" :title="t('metrics')" :aria-label="t('metrics')" :aria-pressed="sftpPaneOpen && rightPanelView === 'metrics'" :disabled="!connected" @click="toggleMetricsPanel"><WorkbenchGlyph name="metrics" /></button>
      <!-- 容器管理底部分屏入口（鲸鱼 logo）：独立开关，不占用右侧栏、不与
           SFTP/指标互斥；local 模式保留——本机 daemon（Docker
           Desktop/OrbStack）场景照常可用。面板自治，收起即卸载停轮询。 -->
      <button class="icon-button icon-blue" :class="{ 'is-active': dockerPaneOpen }" :title="t('docker.toolbarTitle')" :aria-label="t('docker.toolbarTitle')" :aria-pressed="dockerPaneOpen" @click.stop="toggleDockerPane"><DockerWhaleLogo /></button>
      <span class="toolbar-separator" aria-hidden="true" />
      <div>
        <!-- 历史卡右键菜单（ContextMenu）打开时忽略弹层的外点关闭请求：
             菜单项 pointerdown 相对弹层是"外部"，不加守卫会在 select 前把宿主弹层
             连同菜单一起卸载，动作丢失。 -->
        <button class="icon-button icon-blue" :title="t('transfers')" :aria-label="t('transfers')" @pointerdown.stop @click.stop="toggleTransferPanel"><WorkbenchGlyph name="transfers" /><span v-if="activeTransfers" class="activity-dot" /></button>
      </div>
      <button v-if="!localUiMode" class="icon-button icon-cyan" :class="{ 'is-active': sftpPaneOpen && sftpViewActive }" :title="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-label="sftpPaneOpen && sftpViewActive ? t('sftpPane.close') : t('sftpPane.open')" :aria-pressed="sftpPaneOpen && sftpViewActive" @click="toggleSftpPane"><WorkbenchGlyph :name="sftpPaneOpen && sftpViewActive ? 'panel-close' : 'panel-sftp'" /></button>
      <span class="toolbar-separator" aria-hidden="true" />
      <!-- 设置弹窗承载 SSH/SFTP/终端专属分节；审计日志只记录 SSH/SFTP 操作。 -->
      <button class="icon-button icon-amber" :title="t('auditLog.title')" :aria-label="t('auditLog.title')" @click="openAuditLog"><WorkbenchGlyph name="audit" /></button>
      <button class="icon-button icon-violet" :title="t('settings')" :aria-label="t('settings')" @click="openSettings"><WorkbenchGlyph name="settings" /></button>
      </template>
    </div>
    <!-- 传输弹层单实例：Popover 挂在 header 层，锚点是工具条右缘下的隐形
         定位点（.transfer-popover-anchor，固定定位不随工具条高度变化）；
         工具条的传输按钮只是 toggleTransferPanel 的普通开关，共享锚点让同
         一个弹层在工具条下方弹出（内容单拷贝零复制）。 -->
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
      :transfer-shown-bytes="transferShownBytes"
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
/* 录制中图标按钮（原 App.vue scoped 样式随 DOM 迁入）。 */
.is-recording { color: var(--destructive); }
/* 终端 MCP 模式快速开关（工具栏弹出层，与设置弹窗共用三档文案；根规则见 style.css 全局） */
.agent-mode-popover h3 { margin: 4px 0 8px; font-size: 12px; }
.agent-mode-option { display: flex; align-items: center; gap: 8px; padding: 4px 0; font-size: 12px; cursor: pointer; }
.agent-mode-note { margin: 8px 0 0; font-size: 11px; }
</style>
