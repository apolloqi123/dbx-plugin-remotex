import type { AgentTerminalMode } from "../../features/agent/lib/agentTerminal";
import type { QuickCommand } from "../../features/quick-commands/lib/quickCommands";
import type { RecordingMenuActions } from "../../features/recording/lib/recordingMenu";
import type { TerminalColorScheme } from "../../features/terminal/lib/appearance/terminalScheme";
import type { TransferHistoryEntry } from "../../features/transfers/lib/transferHistory";
import type { ResumableUploadTask } from "../../features/transfers/lib/transferResume";
import type { TransferTask } from "../../features/transfers/lib/transferQueue";

/**
 * 工具条组件的动作对象（T19 工具栏拆分）。
 *
 * 工具栏原先直接内联在 App.vue 模板里，绑定的是 App/composables 的同名状态与
 * 函数。拆成组件后，状态走显式 props（`v-model` 用于可写开关），动作统一由
 * App 组装成这一个类型化对象下发——组件不 inject、不读全局上下文，也不持有
 * 会话态；换组件即换调用点，行为与绑定名保持原样。
 *
 * 本文件只含类型（无运行时代码）：各子组件按自身需要声明 `Pick` 子集，
 * WorkbenchToolbar 收全量再原样下传。
 */

/** `useLocalShellPreferences` 的 shell 发现条目（结构等价，源类型未导出）。 */
export type LocalShellEntry = {
  program: string;
  name: string;
  isDefault: boolean;
  isUserShell: boolean;
  injectable?: boolean;
};

/** dock 连接列表条目（dock 面板「shell 表面」菜单用；结构等价）。 */
export type DockConnectionEntry = {
  id: string;
  name: string;
  providerId: string;
  connectionType?: string;
  readOnly?: boolean;
};

/** 会话下拉（本地/远程/复制/命令会话、shell 选择、环境变量、连接列表）。 */
export interface SessionMenuActions {
  toggleSessionMenu(): void;
  openNewSessionTab(): void;
  openCopiedSessionTab(): void;
  openCommandSessionTab(): void;
  toggleLocalTerminal(): void;
  openSessionShellPick(): void;
  openSessionShellPrefs(): void;
  pickLocalShell(program?: string): void;
  setLocalShellPref(program: string): void;
  setLocalShellIntegrationPref(value: boolean): void;
  setLocalShellCwd(value: string): void;
  setLocalShellEnv(): void;
  addLocalShellEnvRow(): void;
  removeLocalShellEnvRow(index: number): void;
  openLocalTerminalTab(): void;
  openLocalShellSurfaceMenu(): void;
  openLocalShellSurface(program?: string): void;
  openConnectionSurface(connection: DockConnectionEntry): void;
  restartLocalTerminal(): void;
  requestLocalTerminal(): void;
}

/** 工具栏「命令管理」弹层：执行 + 完整管理（保存/删除/分组/导入/错误提示）。 */
export interface QuickCommandsMenuActions {
  toggleQuickMenu(): void;
  sendQuickCommand(item: QuickCommand): void;
  pasteQuickCommand(item: QuickCommand): void;
  saveQuickCommand(command: { id?: string; name: string; command: string; groupId?: string; groupName?: string }): void;
  deleteQuickCommand(id: string): void;
  importQuickCommands(items: Array<{ name: string; command: string; groupName?: string }>): void;
  createQuickCommandGroup(name: string): void;
  renameQuickCommandGroupById(groupId: string, name: string): void;
  dissolveQuickCommandGroupById(groupId: string): void;
  showNotice(message: string): void;
  showError(cause: unknown): void;
}

/** RDP 独立工具组（缩放三档 + 分辨率 + 大文本粘贴 + 断开）。 */
export interface RdpToolbarActions {
  toggleRdpResolutionMenu(): void;
  applyRdpResolution(width: number, height: number): void;
  rdpResolutionLabel(width: number, height: number): string;
  isRdpResolutionActive(width: number, height: number): boolean;
  rdpWindowProportionPreset(): { width: number; height: number };
  applyRdpCustomResolution(): void;
  toggleRdpPasteMenu(): void;
  rdpWriteRemoteClipboard(): void;
  rdpTypeAsKeystrokes(): void;
  closeRdpSession(): void;
}

/** dock 面板配色快切弹层。 */
export interface AppearanceMenuActions {
  togglePanelPaletteMenu(): void;
  pickPanelScheme(id: string | null): void;
  openSettings(): void;
}

/** 工具条外壳、身份区、主操作位与 panel 变体的全部动作。 */
export interface WorkbenchToolbarActions
  extends SessionMenuActions,
    QuickCommandsMenuActions,
    RdpToolbarActions,
    AppearanceMenuActions,
    RecordingMenuActions {
  togglePanelToolbar(): void;
  toggleConnectionInfo(): void;
  measureLatency(): void;
  togglePaneOrder(): void;
  toggleSftpPane(): void;
  adjustTerminalZoom(delta: number): void;
  reconnectNow(): void;
  sendSudoRefresh(): void;
  toggleDockerPane(): void;
  toggleBatchBar(): void;
  toggleAgentModeMenu(): void;
  toggleForwardsMenu(): void;
  applyAgentMode(mode: AgentTerminalMode): void;
  toggleMetricsPanel(): void;
  openAuditLog(): void;
  toggleTransferPanel(): void;
  showTransferHistoryMenu(event: MouseEvent, entry: TransferHistoryEntry): void;
  toggleTransferPause(task: TransferTask): void;
  cancelTransfer(task: TransferTask): void;
  revealTransferTarget(path: string): void;
  openTransferTarget(path: string): void;
  refreshTransferPanel(): void;
  beginResumeUpload(task: ResumableUploadTask): void;
  onResumeFilePicked(event: Event): void;
}

/** 配色快切列表条目（`useWorkbenchToolbar` 的派生视图）。 */
export type PanelSchemeChoice = TerminalColorScheme;
