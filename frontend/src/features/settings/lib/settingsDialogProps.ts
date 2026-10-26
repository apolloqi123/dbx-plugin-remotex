// 设置主弹窗的 props 契约：从 SettingsDialog.vue 原样下沉，供组件（defineProps）
// 与 useSettingsDialogModel 的入参共用一份声明。类型名称与字段语义不变；
// 权威态仍在宿主 App，弹窗只读 + 经 emits 上抛改动意图。
import type { DownloadConflictPolicy } from "../../sftp/lib/transfer/downloadPrefs";
import type { EditorConfig, KnownEditor } from "../../sftp/lib/editor/editorRules";
import type { TransferDuplicatePolicy } from "../../transfers/lib/transferQueue";
import type { SftpNameEncoding } from "../../sftp/lib/entries/sftpName";
import type {
  TerminalAppearanceProfile,
  TerminalAppearanceState,
} from "../../terminal/lib/appearance/terminalAppearance";
import type { TerminalThemeLike } from "../../terminal/lib/appearance/terminalScheme";
import type { TerminalBehaviorSettings } from "../../terminal/lib/appearance/terminalBehavior";
import type { TerminalHotkeyBindings } from "../../terminal/lib/behavior/terminalHotkeys";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import type { AiSettings } from "../../agent/lib/aiSettings";
import type { ActionLinksSettings } from "../../terminal/lib/decorations/actionLinksMatcher";
import type { GutterSettings } from "../../terminal/lib/appearance/terminalGutter";
import type { HighlightDisplayMode, HighlightRuleView } from "../../terminal/lib/decorations/keywordHighlight";
import type { QuickCommand } from "../../quick-commands/lib/quickCommands";

export interface SettingsDialogProps {
  open: boolean;
  profilesOpen: boolean;
  sessionId?: string;
  /** 当前 SSH 会话的连接 id：启动命令偏好按 connectionId 分桶（无会话时整区隐藏）。 */
  connectionId?: string;
  /** 终端当前生效字号（缩放链路在 App，设置页字号草稿以它为初始回显）。 */
  terminalFontSize: number;
  /** 宿主主题终端基准字号（「恢复默认」回到该值）。 */
  hostFontSize: number;
  /** 宿主终端字体族（用户未单独设置字体时预览用它）。 */
  hostFontFamily: string;
  localDownloadDir: string;
  localCanSave: boolean;
  webglEnabled: boolean;
  /** 本地终端默认 Shell（localShell 偏好，权威态在 App）：只读 + 上抛改动。 */
  localShell: string;
  /** 终端行为偏好（对标 Tabby「Terminal」页）：权威态在 App，本组件只读 + 上抛增量。 */
  terminalBehavior: TerminalBehaviorSettings;
  /** 分栏布局方向偏好（issue #187，权威态在 App）：只读 + 上抛改动。 */
  paneDirection: PaneDirectionPref;
  /** ghost 接受键（批 4e/反馈迭代）：权威态在 App，本组件只读 + 上抛改动。 */
  ghostTabAccept: boolean;
  /** AI 助手设置（Warp AI 对齐批）：权威态在 App（pluginStore 单键 JSON），
   *  本组件只读 + 上抛增量；bridgeReady 决定能力缺失横幅。 */
  aiSettings: AiSettings;
  aiBridgeReady: boolean;
  /** 空提示符快捷键引导条开关（权威态在 App）：只读 + 上抛改动。 */
  promptHintsEnabled: boolean;
  /** 终端快捷键绑定（对标 Tabby「Hotkeys」页）：权威态在 App。 */
  terminalHotkeys: TerminalHotkeyBindings;
  /** 是否 Apple 平台：决定快捷键修饰键的显示符号与默认键位口径。 */
  applePlatform: boolean;
  /** 动作链接偏好（权威态在 App，sidecar preferences 持久化）：只读 + 上抛增量。 */
  actionLinks: ActionLinksSettings;
  /** 行号/时间戳 gutter 偏好（权威态在 App）：只读 + 上抛增量。 */
  gutter: GutterSettings;
  /** 右键「在线搜索」引擎表原始文本（每行 name|url；权威态在 App，sidecar 持久化）。 */
  ctxSearchEngines: string;
  /** 背景图偏好（权威态在 App）：开关 / 透明度（10..=90 百分比）/ 会话内存态标记。 */
  wallpaperEnabled: boolean;
  wallpaperOpacity: number;
  wallpaperSessionOnly: boolean;
  /** 终端外观偏好（权威态在 App）：本组件只读 + 经 emits 上抛改动意图。 */
  appearance: TerminalAppearanceState;
  /** 用户保存的主题快照（内置预设由 lib 常量提供，不需经 props）。 */
  customThemes: TerminalAppearanceProfile[];
  /** 当前配置命中的主题 id（null = 已改动，不再等于任何主题）。 */
  activeThemeId: string | null;
  /** 宿主派生的基础终端主题（未启用配色方案时的最终结果）。 */
  hostTheme: TerminalThemeLike;
  hostColorScheme: "light" | "dark";
  /** 下载偏好的读写适配器（权威态与 sidecar preferences 同步在 App）。 */
  downloadPrefs: {
    loadDir(): string;
    loadUseDefault(): boolean;
    loadConflict(): DownloadConflictPolicy;
    persistDir(value: string): void;
    persistUseDefault(value: boolean): void;
    persistConflict(value: DownloadConflictPolicy): void;
  };
  /** 上传并发 / 重复目标策略的读写适配器（权威态在 App，sidecar preferences 同步）。 */
  transferPrefs: {
    loadConcurrency(): number;
    loadDuplicatePolicy(): TransferDuplicatePolicy;
    persistConcurrency(value: number): void;
    persistDuplicatePolicy(value: TransferDuplicatePolicy): void;
    /** M14-B：会话级并发深度 / 兼容模式 / 文件名编码。 */
    loadMaxActive(): number;
    loadCompatMode(): boolean;
    loadNameEncoding(): SftpNameEncoding;
    persistMaxActive(value: number): void;
    persistCompatMode(value: boolean): void;
    persistNameEncoding(value: SftpNameEncoding): void;
    /** Issue #66：下载限速（KiB/s，0=不限速）。 */
    loadDownloadLimit(): number;
    persistDownloadLimit(value: number): void;
    /** M33：压缩传输策略（auto/on/off）与生效阈值（MiB，0=不限下限）。 */
    loadCompressMode(): string;
    loadCompressThreshold(): number;
    persistCompressMode(value: string): void;
    persistCompressThreshold(value: number): void;
  };
  /** 命令输入建议（开关 + 查询长度上下限）的读写适配器，权威态同在 App。 */
  suggestionPrefs: {
    loadEnabled(): boolean;
    loadMinChars(): number;
    loadMaxChars(): number;
    persistEnabled(value: boolean): void;
    persistMinChars(value: number): void;
    persistMaxChars(value: number): void;
  };
  /** 外部编辑器配置（扩展名关联/默认编辑器/回传策略）的读写适配器 + 知名
   * 编辑器目录（权威态与探测在 App：pluginStore 单键 + local/editors/list）。 */
  editorPrefs: {
    loadConfig(): EditorConfig;
    persistConfig(value: EditorConfig): void;
    loadEditors(): KnownEditor[];
    reloadEditors(): Promise<KnownEditor[]>;
  };
  /** 关键词高亮规则（M32-A2：权威态 + RPC 在 App，本组件只读 + 上抛增量）。 */
  highlightRules: HighlightRuleView[];
  highlightSaving: boolean;
  /** 关键词显示方式（全局：背景色块 / 字体颜色；权威态在 App）。 */
  highlightDisplayMode: HighlightDisplayMode;
  /** 快速命令（M32-A3：权威态 + RPC 在 App，本组件只读 + 上抛增量）。 */
  quickCommands: QuickCommand[];
  quickSaving: boolean;
  quickImporting: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}
