import { ref, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { SuggestionAnchor } from "../../../shared/lib/overlayPlacement";
import { extractEchoedCommand } from "../lib/terminalEnterCapture";
import { settingsPreferences } from "../../settings/lib/settingsPreferences";
import {
  canOpenHistoryPanel,
  clampHistoryPanelIndex,
  decorateHistoryEntries,
  filterHistoryEntries,
  moveHistoryPanelIndex,
  resolveHistoryPanelKey,
  type HistoryMetaRow,
  type HistoryPanelEntry,
} from "../lib/historyPanel";

/** Warp 式 history 面板（↑ 唤起）：commandHistory 过滤结果可视化快速回填，
 *  选中即回填不执行（shell ↑ 语义）。纯逻辑（过滤/键位映射/门判定）在
 *  lib/historyPanel.ts；这里收口状态接线与按键消费，App 只做数据接线。 */
export function useHistoryPanel(options: {
  terminal: () => Terminal | undefined;
  commandHistory: () => readonly string[];
  commandHistoryTimes: () => Readonly<Record<string, number>>;
  commandHistoryMeta: () => Readonly<Record<string, HistoryMetaRow>>;
  getPendingTerminalInput: () => string;
  getSessionId: () => string | undefined;
  isLocalMode: () => boolean;
  commandRunning: () => boolean;
  terminalTransferBusy: () => boolean;
  completionOpen: () => boolean;
  suggestionOpen: () => boolean;
  quickSelectOpen: () => boolean;
  searchOpen: () => boolean;
  interactivePromptPending: () => boolean;
  terminalMenuOpen: Ref<boolean>;
  loadRemoteShellHistory: (sessionId: string) => Promise<void>;
  replaceTerminalLineWith: (line: string, pressEnter: boolean) => void;
  readEchoTextBeforeCursor: () => string | null;
  readTerminalSuggestionAnchor: () => SuggestionAnchor | null;
  hideGhostSuggestion: () => void;
  closeSuggestions: () => void;
  dismissPromptHints: () => void;
}) {
  const {
    terminal: terminalGet,
    commandHistory,
    commandHistoryTimes,
    commandHistoryMeta,
    getPendingTerminalInput,
    getSessionId,
    isLocalMode,
    commandRunning,
    terminalTransferBusy,
    completionOpen,
    suggestionOpen,
    quickSelectOpen,
    searchOpen,
    interactivePromptPending,
    terminalMenuOpen,
    loadRemoteShellHistory,
    replaceTerminalLineWith,
    readEchoTextBeforeCursor,
    readTerminalSuggestionAnchor,
    hideGhostSuggestion,
    closeSuggestions,
    dismissPromptHints,
  } = options;

  // 打开即全量视图 + 聚焦面板内搜索框（过滤只认搜索框 query，不读行缓冲——
  // 行缓冲模型在 Ctrl+U/方向键编辑后会残留残影）。打开后焦点留在命令行
  // （#138 交互跟进），↑↓/Enter/Tab/Esc 由 App 的面板分支统一消费；搜索框
  // 点击聚焦后字符键实时过滤，其余按键放行。
  const historyPanelOpen = ref(false);
  // 历史面板功能开关（issue #169）：默认开（现状）；关闭后提示符下的 ↑ 与
  // Ctrl+R / Ctrl+Shift+H 热键都归还远端 shell（↑/Ctrl+R 走 shell 原生历史与
  // 反向搜索），面板整体不可唤起。命令历史采集与建议浮层是独立功能（各有
  // 开关），不受此键影响。偏好权威 = settingsPreferences store（设置弹窗与
  // App 共享同一 ref + 唯一落盘点），此处不再自读 pluginStore。
  const historyPanelEnabled = settingsPreferences.historyPanelEnabled;
  function setHistoryPanelEnabled(next: boolean) {
    settingsPreferences.setHistoryPanelEnabled(next);
  }
  // 热键唤起时置 true（挂载即聚焦搜索框，批 3d）；裸 ↑ 唤起恒 false。
  const historyPanelFocusSearch = ref(false);
  // 面板内搜索框的 query（点击聚焦后打字即过滤；关闭随面板清空）。
  const historyPanelQuery = ref("");
  const historyPanelEntries = ref<HistoryPanelEntry[]>([]);
  const historyPanelActiveIndex = ref(0);
  const historyPanelAnchor = ref<SuggestionAnchor | null>(null);
  // 打开前的输入行快照：选中同步会改写行缓冲，取消导航（Esc / 底部再 ↓）时
  // 原样恢复，等价 shell readline 的历史栈回退。
  let historyPanelOriginalLine = "";

  /** 打开门：任一浮层/alternate 屏/命令运行中/传输占用/终端交互提示待答
   *  （issue #150 及其反馈扩展，询问输入/选择场景）时不拦截 ↑。 */
  function historyPanelGateOpen(): boolean {
    return canOpenHistoryPanel({
      completionOpen: completionOpen(),
      suggestionOpen: suggestionOpen(),
      quickSelectOpen: quickSelectOpen(),
      searchOpen: searchOpen(),
      alternateActive: terminalGet()?.buffer.active.type === "alternate",
      commandRunning: commandRunning(),
      transferBusy: terminalTransferBusy(),
      interactivePromptPending: interactivePromptPending(),
    });
  }

  /** focusSearch（批 3d）：热键唤起（⌘⇧H / Ctrl+Shift+H）传 true——打开即
   *  聚焦面板搜索框（Warp Ctrl+R 心智，打字即过滤）；裸 ↑ 唤起不传，焦点留
   *  在命令行（shell ↑ 心智）。两个入口两种心智，互不打断。 */
  function openHistoryPanel(focusSearch = false) {
    if (!terminalGet()) return;
    // 用过即散：面板是引导条教的键位之一，真用过就不再教（持久化旗标）。
    dismissPromptHints();
    terminalMenuOpen.value = false;
    historyPanelFocusSearch.value = focusSearch;
    // 每次打开都是全量视图:清掉上一次的搜索词,不读行缓冲(pendingTerminalInput
    // 是简单字符模型,Ctrl+U/方向键/Ctrl+W 等编辑后会残留残影,曾把面板过滤得
    // 只剩一条)。过滤只走面板内搜索框。
    historyPanelQuery.value = "";
    // 远端历史导入尚未成功时(首次拉取赶上会话未就绪/超时),打开面板顺带重试
    // 一次;loadRemoteShellHistory 按会话去重,成功过即跳过。
    const panelSessionId = getSessionId();
    if (panelSessionId && !isLocalMode()) void loadRemoteShellHistory(panelSessionId);
    const next = decorateHistoryEntries(filterHistoryEntries(commandHistory(), ""), commandHistoryTimes(), commandHistoryMeta());
    historyPanelEntries.value = next;
    // shell ↑ 语义:初始高亮最底部(最新执行的命令),↑ 一直往上翻更旧的;
    // 打开即回填——按下 ↑ 的瞬间输入行就出现最新一条,与 shell 完全一致。
    historyPanelActiveIndex.value = clampHistoryPanelIndex(next.length - 1, next.length);
    // 打开前输入行快照要做屏幕对账（issue #169）：模型行失真（Tab 补全后停在
    // 前缀）时直接存模型行，Esc 取消导航会把 `cd blen` 写回屏幕上本已是
    // `cd blender/` 的行，补全结果被吞。以回显提取的真实行为快照；屏幕不可
    // 测时回落模型行。
    historyPanelOriginalLine = extractEchoedCommand(readEchoTextBeforeCursor(), getPendingTerminalInput()) ?? getPendingTerminalInput();
    syncHistoryPanelLine();
    historyPanelAnchor.value = readTerminalSuggestionAnchor();
    hideGhostSuggestion();
    // 与建议/补全浮层互斥：打开时连带关闭结构化补全（含在途结果）并落输入门，
    // 后续选中回填触发的 lineChanged 重调度不再把补全菜单滑到面板之下；开启
    // 期间 onData 键入分流本就不开浮层，两侧单向互斥闭环。
    closeSuggestions();
    historyPanelOpen.value = true;
  }

  function closeHistoryPanel() {
    if (!historyPanelOpen.value) return;
    historyPanelOpen.value = false;
    historyPanelQuery.value = "";
    historyPanelEntries.value = [];
    historyPanelActiveIndex.value = 0;
    historyPanelAnchor.value = null;
    terminalGet()?.focus();
  }

  /** 高亮项实时回填输入行（shell ↑ 语义）：光标处内容与面板高亮一致；行内容
   *  已相同时跳过（重复激活/重过滤不产生无谓的擦除重打字节）。悬停不改行——
   *  面板条目不派发悬停激活，选中只认键盘 ↑↓ 与点击。 */
  function syncHistoryPanelLine() {
    const entry = historyPanelEntries.value[historyPanelActiveIndex.value];
    if (!entry || getPendingTerminalInput() === entry.command) return;
    replaceTerminalLineWith(entry.command, false);
  }

  /** 取消导航：恢复打开前的原输入行并收起面板。行未被改写过（无候选/未同步/
   *  用户此后自己在终端打字）时为零字节操作。 */
  function cancelHistoryPanelSelection() {
    if (getPendingTerminalInput() !== historyPanelOriginalLine) replaceTerminalLineWith(historyPanelOriginalLine, false);
    closeHistoryPanel();
  }

  function moveHistoryPanelActive(delta: number) {
    historyPanelActiveIndex.value = moveHistoryPanelIndex(historyPanelActiveIndex.value, delta, historyPanelEntries.value.length);
    syncHistoryPanelLine();
  }

  /** 开启期间的过滤联动:只认搜索框 query(点击聚焦后,打字即过滤;终端焦点
   *  路径的打字进命令行,不过滤);重算条目后高亮置底(最新/最相关匹配,shell ↑
   *  语义),锚点跟随光标。syncLine 仅搜索框驱动的重过滤置真——高亮变化同步进
   *  输入行;终端 onData 的无参重算路径不改行(用户正在终端打字,行内容不归
   *  面板管)。 */
  function updateHistoryPanelFilter(syncLine = false) {
    if (!historyPanelOpen.value) return;
    const next = decorateHistoryEntries(filterHistoryEntries(commandHistory(), historyPanelQuery.value), commandHistoryTimes(), commandHistoryMeta());
    historyPanelEntries.value = next;
    historyPanelActiveIndex.value = clampHistoryPanelIndex(next.length - 1, next.length);
    historyPanelAnchor.value = readTerminalSuggestionAnchor();
    if (syncLine) syncHistoryPanelLine();
  }

  /** 点击路径的选中回填：抹掉当前行缓冲后写入命令文本，不回车（用户可再
   *  编辑/自行执行）。键盘导航路径不再经此——行内容已实时同步，Enter/Tab 在
   *  handleHistoryPanelKey 按焦点来源分流，整行替换会覆盖用户对行的编辑。 */
  function selectHistoryEntry(command: string) {
    closeHistoryPanel();
    replaceTerminalLineWith(command, false);
  }

  /** 面板开启时的按键消费（键位→动作映射在 lib/historyPanel.resolveHistory
   *  PanelKey）：↑↓ 移动并实时同步行、Esc/底部再 ↓ 取消恢复原行，其余按键
   *  放行。Enter/Tab 确认收起按焦点来源分流（#138 交互跟进）：命令行路径
   *  return false 放行——行内容已与高亮实时同步，回车执行当前行（shell ↑
   *  语义）、Tab 交远端补全；搜索框路径消费按键并归还终端焦点，不代执行。
   *  两侧都不做整行替换——那会覆盖用户对行的编辑。 */
  function handleHistoryPanelKey(event: KeyboardEvent, fromSearchBox = false): boolean {
    if (event.type !== "keydown" || event.isComposing || event.keyCode === 229) return false;
    const action = resolveHistoryPanelKey(event.key, historyPanelActiveIndex.value, historyPanelEntries.value.length);
    if (!action) return false;
    if (action.kind === "move") moveHistoryPanelActive(action.delta);
    else if (action.kind === "stay") {
      // 顶部最旧一条再 ↑：消费按键、停住不回绕也不关面板（readline 到最旧即止；
      // #138 丝滑度 review P0-D——原先落进兜底 else 被当成 close，翻到顶面板
      // 直接消失）。
    }
    else if (action.kind === "fill" || action.kind === "close") {
      closeHistoryPanel();
      return fromSearchBox ? true : false;
    } else if (action.kind === "cancel") cancelHistoryPanelSelection();
    else closeHistoryPanel();
    return true;
  }

  /** 面板搜索框的键盘转发：焦点在输入框时导航/确认/关闭键由 App 的同一
   *  handleHistoryPanelKey 以搜索框路径消费（preventDefault 挡住输入框的
   *  光标移动/焦点转移/表单提交），其余字符键放行进 query。 */
  function handleHistoryPanelPanelKey(event: KeyboardEvent) {
    if (handleHistoryPanelKey(event, true)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  return {
    historyPanelOpen,
    historyPanelEnabled,
    setHistoryPanelEnabled,
    historyPanelFocusSearch,
    historyPanelQuery,
    historyPanelEntries,
    historyPanelActiveIndex,
    historyPanelAnchor,
    historyPanelGateOpen,
    openHistoryPanel,
    closeHistoryPanel,
    updateHistoryPanelFilter,
    selectHistoryEntry,
    handleHistoryPanelKey,
    handleHistoryPanelPanelKey,
  };
}
