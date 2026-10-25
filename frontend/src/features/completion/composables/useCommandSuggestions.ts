import type { Terminal } from "@xterm/xterm";
import type { QuickCommand } from "../../quick-commands/lib/quickCommands";
import { ref, type Ref } from "vue";
import { searchCommands, commandSuggestionQueryAcceptable, resolveSuggestionTabKey, type CommandSuggestion } from "../lib/commandSuggestions";
import { addToSuggestionBlocklist, loadSuggestionBlocklist } from "../lib/suggestionBlocklist";
import { pickGhostMatch } from "../../terminal/lib/decorations/terminalGhostSuggest";
import { canShowSuggestions, createSuggestionGuardState, type SuggestionGuardState } from "../lib/suggestionGuard";
import { useCompletionOverlay } from "./useCompletionOverlay";
import { useShellSniff } from "./useShellSniff";
import { useSuggestionAnchor } from "./useSuggestionAnchor";

/** 命令输入建议浮层（P1-1）+ 结构化补全（FIG wave-1）：历史建议与 fig 引擎
 * 两层浮层的采集→抑制门→检索→定位→按键消费，CompletionController 调度
 * 中枢（engine runner + shell 内建兜底 + 声明式 generator 通道）。行缓冲
 * 读写、终端 host 量测与会话 id 解析经依赖注入（基础件仍在 App.vue）。
 * 分工：锚点读取/重锚在 useSuggestionAnchor，shell 采样在 useShellSniff，
 * 结构化补全浮层与 controller 接线在 useCompletionOverlay；本函数保留历史
 * 建议浮层（检索/抑制门/宽限/仲裁）并组合三者。 */
export function useCommandSuggestions(options: {
  terminal: () => Terminal | undefined;
  getTerminalHost: () => HTMLElement | null | undefined;
  sendTerminalBytes: (data: Uint8Array) => void;
  getPendingTerminalInput: () => string;
  setPendingTerminalInput: (value: string) => void;
  commandRunning: Ref<boolean>;
  isTerminalTransferBusy: () => boolean;
  commandHistory: Ref<string[]>;
  quickCommands: Ref<QuickCommand[]>;
  suggestionsEnabledState: Ref<boolean>;
  suggestionMinCharsState: Ref<number>;
  suggestionMaxCharsState: Ref<number>;
  terminalCwd: Ref<string>;
  session: Ref<{ sessionId?: string } | undefined>;
  getSessionId: () => string;
  getLocalSessionId: () => string | undefined;
  persistCommandHistory: () => void;
  /** Warp 式 history 面板开启时，锚点同步帧连带刷新面板锚点。 */
  isHistoryPanelOpen: () => boolean;
  syncHistoryPanelAnchor: () => void;
  /** 面板开启期间的键入分流：按最新行缓冲重算过滤结果（App.vue 状态接线）。 */
  updateHistoryPanelFilter: () => void;
  /** 统一采集口：命令环 + 执行时间映射一并推进（回填后回车执行路径）。 */
  pushTerminalCommandHistory: (command: string) => void;
  /** 行内 ghost 建议是否启用（数据分工：前缀延伸命中让位 ghost，#138 批 2）。
   *  惰性求值——useGhostSuggest 在本 composable 之后初始化，闭包在 onData
   *  期才调用，无 TDZ 风险。 */
  inlineGhostActive: () => boolean;
  /** 终端交互提示待答（issue #150 及其反馈扩展，MFA/验证码/密码待输入、
   *  选择菜单与 y/n 确认）：惰性求值，键入期采样——交互应答不是命令键入，
   *  不开建议/补全浮层。 */
  isInteractivePromptPending: () => boolean;
}) {
  const { terminal: terminalGet, terminalCwd, getTerminalHost, sendTerminalBytes, getPendingTerminalInput, setPendingTerminalInput, commandRunning, isTerminalTransferBusy, commandHistory, quickCommands, suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState, session, getSessionId, getLocalSessionId, persistCommandHistory, isHistoryPanelOpen, syncHistoryPanelAnchor, updateHistoryPanelFilter, pushTerminalCommandHistory, inlineGhostActive, isInteractivePromptPending } = options;

// 命令输入建议浮层（P1-1）运行时状态：条目/选中项/光标锚点与抑制门锁存。
// 开关与长度上下限的权威值在上方 suggestions*State（sidecar 偏好）。
const suggestionOpen = ref(false);
const suggestionItems = ref<CommandSuggestion[]>([]);
const suggestionActiveIndex = ref(0);
// Tab 接受的显式选中锁（issue #138）：↑↓ 导航置位，候选随键入重算时复位。
// 仅锁存态允许 Tab 回填与当前行无关的模糊命中项；非响应式，不进渲染。
let suggestionTabArmed = false;
const suggestionQuery = ref("");
// 抑制门锁存（跟随型程序命中后保持抑制，Ctrl+C/q 解除）：非响应式即可，
// 只有 canShowSuggestions 的返回值会进渲染。
let suggestionGuardState: SuggestionGuardState = createSuggestionGuardState();
// 最近一次执行的命令行（onData 回车行 + OSC 633 E 帧），抑制门据此判定。
const lastTerminalCommand = ref<string | null>(null);

function closeSuggestionsOnly() {
  cancelSuggestionCloseGrace();
  suggestionOpen.value = false;
  suggestionItems.value = [];
  suggestionActiveIndex.value = 0;
}

// —— 空结果宽限（批 3b 防闪烁）：快速打词的中间态常瞬时无匹配（"doc"→
// "dock"→"docker"），逐键即关即开会把浮层闪成频闪灯。空结果先挂 150ms
// 宽限：窗口内下一次键入若重新命中则照常刷新（取消挂起关闭），真正无匹配
// 才收起（VS Code 补全同款迟滞）。注意所有「立即关」路径都走
// closeSuggestionsOnly（其内取消宽限），宽限只兜「打字中瞬时空洞」。 ——
let suggestionCloseTimer: ReturnType<typeof setTimeout> | undefined;
const SUGGESTION_CLOSE_GRACE_MS = 150;

function cancelSuggestionCloseGrace() {
  if (suggestionCloseTimer === undefined) return;
  clearTimeout(suggestionCloseTimer);
  suggestionCloseTimer = undefined;
}

function scheduleSuggestionCloseGrace() {
  cancelSuggestionCloseGrace();
  if (!suggestionOpen.value) return;
  suggestionCloseTimer = setTimeout(() => {
    suggestionCloseTimer = undefined;
    closeSuggestionsOnly();
  }, SUGGESTION_CLOSE_GRACE_MS);
}

// 浮层锚点（suggestionAnchor/completionAnchor/suggestionViewport）与回显后
// 重锚收口在 useSuggestionAnchor；开合态用惰性 getter——completionOpen 由
// 下方 useCompletionOverlay 创建，rAF 回调期才读取。
const { suggestionAnchor, completionAnchor, suggestionViewport, readTerminalCellFrame, readTerminalSuggestionAnchor, syncSuggestionAnchorsOnSettle } = useSuggestionAnchor({
  terminal: terminalGet,
  getTerminalHost,
  isSuggestionOpen: () => suggestionOpen.value,
  isCompletionOpen: () => completionOpen.value,
  isHistoryPanelOpen,
  syncHistoryPanelAnchor,
});

// Windows shell 提示符采样缓存（shellBuiltins 兜底源的 detect 回调）。
const { sniffTerminalShell } = useShellSniff({ terminal: terminalGet });

// 结构化补全浮层（FIG wave-1）：唯一来源 = fig 引擎，经冻结接缝
// FigCompletionSource 注入 CompletionController（解析→防抖→guard→菜单）；
// 状态/按键/双浮层仲裁与 controller 接线收口在 useCompletionOverlay。
const completionOverlay = useCompletionOverlay({
  terminal: terminalGet,
  getPendingTerminalInput,
  commandRunning,
  isTerminalTransferBusy,
  terminalCwd,
  session,
  getSessionId,
  getLocalSessionId,
  sniffTerminalShell,
  completionAnchor,
  readTerminalSuggestionAnchor,
  closeSuggestionsOnly,
  // 整行替换机制不变：接受候选项仍走 replaceTerminalLineWith（函数声明
  // 提升，调用发生在首次响应之后；其内 controller 引用届时已初始化）。
  replaceTerminalLine: (nextLine) => replaceTerminalLineWith(nextLine, false),
});
const { completionOpen, completionItems, completionActiveIndex, completionLoading, completionController, closeCompletionMenu, refreshCompletionMenu, openCompletionsManually, acceptCompletionRow, handleCompletionKey, setCompletionInputAllowed } = completionOverlay;

function closeSuggestions() {
  closeSuggestionsOnly();
  // 结构化补全浮层与历史建议浮层同一生命周期（Ctrl+C/回车/Esc 同步关闭）；
  // dismiss 同步作废挂起调度与在途结果（revision 前进）。
  completionController.dismiss();
  // 同步落输入门：关闭后 lineChanged/settle 触发的重调度（如 history 面板
  // 开启期间选中回填触发 lineChanged）不再把补全菜单重新拉起——面板与建议/
  // 补全互斥；常规键入路径（refreshSuggestionsAfterInput）会重新放行。
  setCompletionInputAllowed(false);
  closeCompletionMenu();
}

function suggestionSearchBounds() {
  return {
    minLength: Math.max(1, suggestionMinCharsState.value),
    maxLength: Math.max(suggestionMinCharsState.value, suggestionMaxCharsState.value),
  };
}

// 建议黑名单（批 4c，Warp IgnoredSuggestions 语义）：行内 ✗ 永久排除单条
// 建议，pluginStore 持久化；运行时 ref 供设置页计数与清空联动。
const suggestionBlocklist = ref(loadSuggestionBlocklist());

function runSuggestionSearch(query: string): CommandSuggestion[] {
  const bounds = suggestionSearchBounds();
  const blocklisted = new Set(suggestionBlocklist.value);
  return searchCommands(query, { history: commandHistory.value, quickCommands: quickCommands.value }, { ...bounds, limit: 12 })
    .filter((item) => !blocklisted.has(item.command));
}

/** 行内 ✗：永久排除该建议并即时收起浮层（下次检索不再出现）。 */
function ignoreSuggestion(command: string) {
  suggestionBlocklist.value = addToSuggestionBlocklist(command, suggestionBlocklist.value);
  closeSuggestionsOnly();
}

/** 单字符输入事件抽取：多字符粘贴 / 控制序列 / 回车返回 null。 */
function suggestionTypingChar(data: string): string | null {
  if (data.length !== 1) return null;
  const char = data.charAt(0);
  if (char < " " || char === "\u007f") return null;
  return char;
}

/**
 * onData 每次输入后调用：推进抑制门状态并按需刷新浮层。
 * lineBefore 是本次输入前的行缓冲快照（\r 清空后仍能取到被执行的命令行）。
 * 结构化补全（fig 引擎）经 CompletionController 调度：本函数是唯一放行
 * completionInputAllowed 的地方（常规键入路径），旁路写入（粘贴/快速命令）
 * 与 guard 抑制面一律关门。
 */
function refreshSuggestionsAfterInput(data: string, lineBefore: string) {
  // Warp 式 history 面板开启期间：建议/补全/ghost 浮层全部让位（同屏不叠
  // 两层浮层），面板按最新行缓冲实时过滤；面板关闭后的下一次键入恢复常规
  // 调度（guard 锁存暂停推进——面板期间 ↑↓/Enter 均被吞键，不会产生命令）。
  if (isHistoryPanelOpen()) {
    updateHistoryPanelFilter();
    return;
  }
  const alternateActive = terminalGet()?.buffer.active.type === "alternate";
  const typingChar = suggestionTypingChar(data);

  if (data.includes("\u0003")) {
    // Ctrl+C：打断当前行与跟随程序，锁存解除，浮层关闭。
    setCompletionInputAllowed(false);
    suggestionGuardState = canShowSuggestions({ alternateActive, lastCommand: null, typingChar: "\u0003" }, suggestionGuardState).state;
    lastTerminalCommand.value = null;
    closeSuggestions();
    return;
  }
  if (data.includes("\r") || data.includes("\n")) {
    const executed = lineBefore.trim();
    if (executed) lastTerminalCommand.value = executed;
    setCompletionInputAllowed(false);
    suggestionGuardState = canShowSuggestions({ alternateActive, lastCommand: lastTerminalCommand.value, typingChar: null }, suggestionGuardState).state;
    closeSuggestions();
    return;
  }
  if (data.includes("\u001b")) {
    // 方向键/控制序列：不当作输入，浮层保持原状之外直接隐藏（无法追踪行内容）。
    setCompletionInputAllowed(false);
    closeSuggestions();
    return;
  }
  if (data === "\t") {
    // 裸 Tab 已按 #138 语义放行 shell 做路径补全：本次键入不开浮层——guard 对
    // 控制键返回 show，不拦会把刚关闭的浮层在同一按键的 onData 里重开。
    // shell 补全回显后的下一次常规键入照常重新调度。
    setCompletionInputAllowed(false);
    closeSuggestions();
    return;
  }

  const guard = canShowSuggestions(
    { alternateActive, lastCommand: lastTerminalCommand.value, typingChar, lineEmpty: lineBefore.length === 0, interactivePromptPending: isInteractivePromptPending() },
    suggestionGuardState,
  );
  suggestionGuardState = guard.state;
  if (!guard.show || !suggestionsEnabledState.value) {
    setCompletionInputAllowed(false);
    closeSuggestions();
    return;
  }
  // 结构化补全（fig 引擎）优先：request 发起解析——worker 模式下响应异步
  // 回来（三重 guard 保证过期结果不进 UI）；pass-through 关浮层并回落下方
  // 历史建议浮层（两者并存、不替换，历史建议分支一行未动）。
  setCompletionInputAllowed(true);
  completionController.request("typing");
  if (completionOpen.value) {
    suggestionOpen.value = false;
    suggestionItems.value = [];
    return;
  }
  closeCompletionMenu();
  const query = getPendingTerminalInput();
  const bounds = suggestionSearchBounds();
  if (!commandSuggestionQueryAcceptable(query, bounds.minLength, bounds.maxLength)) {
    // 历史回落分支只收历史浮层：此处 dismiss 会顶掉本次键入在途的补全响应
    // （见 closeSuggestionsOnly 注释），worker 模式下表现为补全永久失效。
    closeSuggestionsOnly();
    return;
  }
  const items = runSuggestionSearch(query);
  if (!items.length) {
    // 空结果走宽限（批 3b 防闪烁）：瞬时空洞（打词中间态）不立即收浮层，
    // 150ms 内下一次键入重新命中则无感刷新；真正无匹配才关。
    scheduleSuggestionCloseGrace();
    return;
  }
  // 数据分工（#138 丝滑度批 2，P0-C——ghost 默认不可见的根因修复）：候选里
  // 存在前缀延伸命中时让位给行内 ghost（Warp/fish 口径——ghost 是光标后的
  // 灰字不遮输出，→/Ctrl+→ 接受；模糊浮层弹出会把它互斥掉）。同一份
  // searchCommands 结果复用判定，零额外检索；ghost 开关关闭时分工不生效，
  // 浮层照旧。结构化补全不受影响（上方 request 照发，与 ghost 同屏共存）。
  if (inlineGhostActive() && pickGhostMatch(query, items)) {
    closeSuggestionsOnly();
    return;
  }
  suggestionQuery.value = query;
  suggestionItems.value = items;
  suggestionActiveIndex.value = 0;
  // 候选随键入重算即回到「自动高亮」态：Tab 显式选中锁复位（issue #138）；
  // 空结果宽限一并取消（本键已重新命中，不得再被挂起的关闭收走）。
  suggestionTabArmed = false;
  cancelSuggestionCloseGrace();
  suggestionAnchor.value = readTerminalSuggestionAnchor();
  suggestionOpen.value = true;
}

/** 浮层开启时的按键消费：↑↓ 选择（置 Tab 显式选中锁）、Tab 按 #138 语义
 *  裁决（延伸/显式选中回填，其余放行 shell 补全）、Enter 执行当前行、Esc 关闭。 */
function handleSuggestionKey(event: KeyboardEvent): boolean {
  if (event.type !== "keydown" || !suggestionOpen.value || !suggestionItems.value.length) return false;
  const items = suggestionItems.value;
  if (event.key === "ArrowDown") {
    suggestionActiveIndex.value = (suggestionActiveIndex.value + 1) % items.length;
    suggestionTabArmed = true;
    return true;
  }
  if (event.key === "ArrowUp") {
    suggestionActiveIndex.value = (suggestionActiveIndex.value - 1 + items.length) % items.length;
    suggestionTabArmed = true;
    return true;
  }
  if (event.key === "Tab") {
    // issue #138：Tab 默认归远端 shell（路径补全是 Tab 的本职）——仅当高亮
    // 建议是当前行的严格延伸、或用户已 ↑↓ 显式选中（tabArmed）时才回填；
    // 自动高亮的模糊命中不再被 Tab 默认选上。放行路径关闭浮层后 return
    // false 不消费，Tab 字节照发 PTY；onData 的 "\t" 短路分支保证浮层不会
    // 被同一按键立即重开。
    const item = items[suggestionActiveIndex.value];
    if (item && resolveSuggestionTabKey(item.command, getPendingTerminalInput(), suggestionTabArmed) === "fill") {
      fillSuggestion(item);
      return true;
    }
    closeSuggestions();
    return false;
  }
  if (event.key === "Enter") {
    // 执行当前输入行（review #120：浮层自动出现 ≠ 接管 Enter）——关闭浮层
    // 后不消费，回车字节原样进 PTY；要执行建议先 Tab 填充再回车。
    closeSuggestions();
    return false;
  }
  if (event.key === "Escape") {
    closeSuggestions();
    return true;
  }
  return false;
}

/** 把当前输入行替换为建议命令（readline 清行后重新写入）。清行用 Ctrl+U
 *  （\u0015，bash/zsh/busybox/fish 均默认绑定为删至行首）而非按模型长度发
 *  N 个退格：pendingTerminalInput 是纯键入模型，Tab 补全/历史展开等回显改写
 *  不进模型，按模型长度退格会擦不干净或擦过头，回填与「Esc 恢复原输入行」
 *  因此破坏屏幕上真实的行（issue #169）；Ctrl+U 让远端 readline 自己删，天然
 *  与屏幕一致。 */
function replaceTerminalLineWith(nextLine: string, pressEnter: boolean) {
  if (!terminalGet()) return;
  const payload = "\u0015" + nextLine + (pressEnter ? "\r" : "");
  setPendingTerminalInput(pressEnter ? "" : nextLine);
  if (pressEnter) {
    lastTerminalCommand.value = nextLine;
    pushTerminalCommandHistory(nextLine);
  }
  sendTerminalBytes(new TextEncoder().encode(payload));
  // 整行替换也是行缓冲变更点（FIG wave-1 锚点）：作废在途结果并防抖刷新；
  // 显式刷新面（acceptCompletionRow）会紧跟 request 即时冲掉本次防抖。
  completionController.lineChanged();
}

function fillSuggestion(item: CommandSuggestion) {
  replaceTerminalLineWith(item.command, false);
  // 填充后按新行内容刷新候选（可能只剩自身），保持浮层继续可微调。
  const items = runSuggestionSearch(item.command);
  if (items.length) {
    suggestionItems.value = items;
    suggestionActiveIndex.value = Math.max(0, items.findIndex((entry) => entry.command === item.command));
    suggestionQuery.value = item.command;
    suggestionAnchor.value = readTerminalSuggestionAnchor();
  } else {
    closeSuggestions();
  }
  // 本次回填即接受动作：显式选中锁复位，下一次 Tab 回到延伸/重新选择裁决。
  suggestionTabArmed = false;
  terminalGet()?.focus();
}

/** 会话切换/断开：建议浮层与抑制门锁存一并复位（P1-1）；结构化补全在途
 * 结果经 resetSession 作废（sessionId guard 另有兜底）。由命令标记复位点调用。 */
function resetSuggestionsForSession() {
  closeSuggestions();
  suggestionGuardState = createSuggestionGuardState();
  setCompletionInputAllowed(false);
  completionController.resetSession();
  lastTerminalCommand.value = null;
}


  return {
    suggestionOpen,
    suggestionItems,
    suggestionActiveIndex,
    suggestionAnchor,
    suggestionQuery,
    suggestionViewport,
    completionOpen,
    completionItems,
    completionActiveIndex,
    completionAnchor,
    completionLoading,
    closeSuggestions,
    closeSuggestionsOnly,
    handleSuggestionKey,
    handleCompletionKey,
    refreshSuggestionsAfterInput,
    refreshCompletionMenu,
    openCompletionsManually,
    replaceTerminalLineWith,
    fillSuggestion,
    acceptCompletionRow,
    syncSuggestionAnchorsOnSettle,
    readTerminalCellFrame,
    readTerminalSuggestionAnchor,
    completionController,
    /** 提示符采样判远端 shell（powershell/cmd/null）：AI 上下文的 shell 语义
     *  来源（Warp AI 对齐批）——复用既有采样，不为 AI 另写探测。 */
    sniffTerminalShell,
    lastTerminalCommand,
    resetSuggestionsForSession,
    ignoreSuggestion,
  };
}
