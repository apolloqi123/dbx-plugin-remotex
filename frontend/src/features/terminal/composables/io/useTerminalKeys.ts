import type { Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import {
  matchesGhostFullAccept,
  matchesGhostWordAccept,
  resolveCtrlFRoute,
} from "../../../completion/lib/ghostAcceptKey";
import {
  keyComboFromEvent,
  matchTerminalHotkey,
  type TerminalHotkeyBindings,
} from "../../lib/behavior/terminalHotkeys";
import { commandHistoryHotkeyPassthrough } from "../../lib/panels/historyPanel";
import {
  ARBITER_PRIORITIES,
  resolveKeyOwners,
  type ArbiterClaim,
  type ClaimKey,
  type KeyOwners,
  type OverlayId,
} from "../../../../shared/lib/overlayArbiter";

/** ghost 行内建议命中面（useGhostSuggest 持有）。 */
export interface GhostSuggestionMatch {
  command: string;
  remainder: string;
}

/**
 * 终端按键路由与浮层键权仲裁：热键注册表派发、ghost 逐词/整段接受、Ctrl+F
 * 搜索/接受分派、补全/建议/历史面板/Quick Select 的按键消费顺序，以及
 * 「这个键维度此刻归谁」的仲裁表（docs/plans/IMPL_PLAN_OVERLAY_KEY_ARBITER）。
 *
 * 迁移约束（Task 17）：分支顺序与仲裁可见性判定逐字迁移，未做任何重排；
 * 跨模块状态经 options 注入（Ref 直取或惰性 getter），命令执行仍由宿主
 * composable 回调承担。仲裁表只决定轮不轮到某个浮层，键语义在各 handler 内。
 */
export interface TerminalKeysOptions {
  terminalMenuOpen: Readonly<Ref<boolean>>;
  hideGhostSuggestion: () => void;
  ghostMatch: Readonly<Ref<GhostSuggestionMatch | null>>;
  ghostKeycapMenuOpen: Readonly<Ref<boolean>>;
  suggestionOpen: Readonly<Ref<boolean>>;
  completionOpen: Readonly<Ref<boolean>>;
  commandRunning: Readonly<Ref<boolean>>;
  terminalTransferBusy: Readonly<Ref<boolean>>;
  acceptGhostSuggestionWord: () => void;
  acceptGhostSuggestion: () => void;
  dismissPromptHints: () => void;
  ghostTabAccept: Readonly<Ref<boolean>>;
  openTerminalSearch: () => void;
  closeTerminalSearch: () => void;
  handleCompletionKey: (event: KeyboardEvent) => boolean;
  handleSuggestionKey: (event: KeyboardEvent) => boolean;
  historyPanelOpen: Readonly<Ref<boolean>>;
  handleHistoryPanelKey: (event: KeyboardEvent) => boolean;
  openHistoryPanel: (focusSearch?: boolean) => void;
  closeHistoryPanel: () => void;
  historyPanelGateOpen: () => boolean;
  historyPanelEnabled: Readonly<Ref<boolean>>;
  searchOpen: Readonly<Ref<boolean>>;
  quickSelectOpen: Readonly<Ref<boolean>>;
  handleQuickSelectKey: (event: KeyboardEvent) => boolean;
  openQuickSelect: () => void;
  closeQuickSelect: () => void;
  closeGhostKeycapMenu: () => void;
  toggleTerminalDiag: () => void;
  terminalHotkeys: Readonly<Ref<TerminalHotkeyBindings>>;
  openCompletionsManually: () => void;
  hasAiFixBar: () => boolean;
  aiFixFromBar: () => void;
  aiBridgeOk: () => boolean;
  isAiAssistEnabled: () => boolean;
  aiAssistFromTerminal: () => void;
  showNotice: (message: string) => void;
  t: (key: string) => string;
  terminal: () => Terminal | undefined;
  copyTerminalSelection: () => void;
  selectAllTerminal: () => void;
  clearTerminal: () => void;
  adjustTerminalZoom: (delta: number) => void;
  resetTerminalZoom: () => void;
  /** 引导条锚点浮层可见性（usePromptHints 在 setup 尾部装配，惰性读取）。 */
  isPromptHintsVisible: () => boolean;
}

export function useTerminalKeys(options: TerminalKeysOptions) {
  const {
    terminalMenuOpen,
    hideGhostSuggestion,
    ghostMatch,
    ghostKeycapMenuOpen,
    suggestionOpen,
    completionOpen,
    commandRunning,
    terminalTransferBusy,
    acceptGhostSuggestionWord,
    acceptGhostSuggestion,
    dismissPromptHints,
    ghostTabAccept,
    openTerminalSearch,
    closeTerminalSearch,
    handleCompletionKey,
    handleSuggestionKey,
    historyPanelOpen,
    handleHistoryPanelKey,
    openHistoryPanel,
    closeHistoryPanel,
    historyPanelGateOpen,
    historyPanelEnabled,
    searchOpen,
    quickSelectOpen,
    handleQuickSelectKey,
    openQuickSelect,
    closeQuickSelect,
    closeGhostKeycapMenu,
    toggleTerminalDiag,
    terminalHotkeys,
    openCompletionsManually,
    hasAiFixBar,
    aiFixFromBar,
    aiBridgeOk,
    isAiAssistEnabled,
    aiAssistFromTerminal,
    showNotice,
    t,
    terminal: terminalGet,
    copyTerminalSelection,
    selectAllTerminal,
    clearTerminal,
    adjustTerminalZoom,
    resetTerminalZoom,
    isPromptHintsVisible,
  } = options;

/**
 * 终端快捷键派发（对标 Tabby「Hotkeys」页）：先由 lib 侧把事件折算成规范组合串
 * （基于 event.code，Shift 恒保留为修饰键，故 Ctrl+= 与 Ctrl+Shift+= 不会塌成一个），
 * 再到用户可改写的注册表里查动作。lib 只做纯解析与匹配，命令执行留在 App。
 *
 * 默认表刻意不绑裸 Ctrl+A / Ctrl+C / Ctrl+F：这些要留给远端 shell 的
 * readline 与 SIGINT，只有 macOS 的 Cmd 系列、以及其余平台的 Ctrl+Shift 系列被占用。
 */
function handleTerminalKey(event: KeyboardEvent) {
  const terminal = terminalGet();
  if (event.type !== "keydown") return true;
  // 右键菜单打开时暂停终端键盘捕获（P2-8）：按键归菜单导航，不落远端 shell
  // （macOS 直写路径已在 onData 包装层同步暂停）。不取消浏览器默认动作。
  if (terminalMenuOpen.value) return false;
  // xterm 的 false 只跳过终端处理，不会取消浏览器默认动作或冒泡。
  const consume = () => {
    event.preventDefault();
    event.stopPropagation();
    return false;
  };
  // IME 组合中不出 ghost（组合文本尚未落行；提交后的 onData 会重算）。
  if (event.isComposing || event.keyCode === 229) hideGhostSuggestion();
  // ghost 逐词接受（Ctrl+→ / Ctrl+Shift+→，macOS / Windows·Linux 同收；
  // Warp word-accept）：只注入剩余文本的首个「空白串 + 词」块。#138 批 2 起
  // 与结构化补全菜单同屏共存——菜单只占用 ↑↓/Tab/Esc，带修饰的 → 归 ghost。
  // 复查 commandRunning/传输占用（与 evaluateGhost 同门）：update 与 accept
  // 之间远端可能已开跑（回车竞态），不能把剩余字节打进运行中的命令。
  if (
    ghostMatch.value &&
    !suggestionOpen.value &&
    matchesGhostWordAccept(event.key, event) &&
    !(commandRunning.value || terminalTransferBusy.value)
  ) {
    acceptGhostSuggestionWord();
    return consume();
  }
  // ghost 整段接受（Warp 口径三键恒可用：裸 → / Ctrl+F / Ctrl+E——ghost 态
  // 光标在行尾，截获 0x06/0x05 等价 → 与 End，无 ghost 时照发远端 readline；
  // 「Tab 接受建议」开关开启时 Tab 在无菜单态也接受，菜单开着则归菜单）：
  // #138 批 2 起不再被结构化补全菜单抑制（菜单不占用裸 →；ghost 是行内灰
  // 字、菜单在光标行上/下，同屏共存）。历史建议浮层开着时无 ghost（数据分
  // 工），此判保持。未命中接受键的按键原样放行给 shell。
  if (
    ghostMatch.value &&
    !suggestionOpen.value &&
    (matchesGhostFullAccept(event.key, event) || (ghostTabAccept.value && event.key === "Tab" && !completionOpen.value)) &&
    !(event.ctrlKey && event.key === "Tab") &&
    !(commandRunning.value || terminalTransferBusy.value)
  ) {
    // 用过即散：接受 ghost 命中引导条教的键位（→ / Ctrl+F / Ctrl+E），此后不再教。
    dismissPromptHints();
    acceptGhostSuggestion();
    return consume();
  }
  // Ctrl+F 终端搜索（issue #108a，iTerm2 同位）：ghost 建议可见待接受时上方
  // 分支已把 Ctrl+F 消费为整段接受（Warp 键位模型不变）；无 ghost 才轮到
  // 搜索。传输占用（trz/zmodem 协议拥有输入流）时放行——与 resolveTerminalInputRoute
  // 同判，不打断传输（Ctrl+F 的 0x06 由传输 filter/远端自行解释）。
  if (
    event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey &&
    event.key.toLowerCase() === "f" &&
    resolveCtrlFRoute({ ghostVisible: !!ghostMatch.value, transferBusy: terminalTransferBusy.value }) === "search"
  ) {
    openTerminalSearch();
    return consume();
  }
  // 命令建议浮层开启时优先消费导航/回填键；Tab 仅在延伸/显式选中时回填，
  // 其余关闭浮层放行 shell 补全（issue #138）。结构化补全浮层（线 2）优先级
  // 更高，按键语义相同（↑↓/Tab/Enter/Esc）。
  if (
    completionOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "completion") &&
    handleCompletionKey(event)
  )
    return consume();
  if (
    suggestionOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "suggestion") &&
    handleSuggestionKey(event)
  )
    return consume();
  // Warp 式 history 面板（↑ 唤起）：面板开启时优先消费导航/回填键（↑↓ 移动、
  // Enter/Tab 回填、Esc 关闭），其余按键原样放行——焦点回到终端时打字仍会
  // 走 onData 链刷新锚点。未开启时裸 ↑ 经仲裁器唤起门打开全量视图（过滤交给
  // 面板内搜索框，不读行缓冲）；alternate 屏（vim/htop 靠 ↑ 导航）、命令运行中、
  // 传输占用、既有浮层开启时不抢 ↑，shell 原生 readline 历史在那些场景依旧可达。
  if (
    historyPanelOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "historyPanel") &&
    handleHistoryPanelKey(event)
  )
    return consume();
  if (
    event.type === "keydown" &&
    event.key === "ArrowUp" &&
    !(event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) &&
    !event.isComposing &&
    event.keyCode !== 229 &&
    terminalOverlayKeyOwners().arrowUp === "historyPanel"
  ) {
    openHistoryPanel();
    return consume();
  }
  // 搜索框已打开时 Esc 先关面板，不参与快捷键匹配（关闭键不可改写）。
  {
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.shiftKey && (event.key === "d" || event.key === "D")) {
      // 快速输入丢失诊断浮层（#33/#71）：三计数锁定丢失层，双击浮层关闭。
      toggleTerminalDiag();
      return consume();
    }
  }
  if (event.key === "Escape" && terminalOverlayKeyOwners().escape === "search") {
    closeTerminalSearch();
    return consume();
  }
  // ghost 键位胶囊菜单开着时 Esc 先收菜单，不落远端（批 4e）。
  if (event.key === "Escape" && terminalOverlayKeyOwners().escape === "ghost") {
    closeGhostKeycapMenu();
    return consume();
  }
  // Quick Select 浮层按键（WT-1）：↑↓ 移动、Enter 复制当前项、Esc 关闭。
  // 浮层是被动的（焦点留在终端、不接管键盘），未命中的按键原样放行远端 shell。
  if (
    quickSelectOpen.value &&
    overlayOwnsKey(terminalOverlayKeyOwners(), event, "quickSelect") &&
    handleQuickSelectKey(event)
  )
    return consume();
  const combo = keyComboFromEvent(event);
  if (!combo) return true;
  switch (matchTerminalHotkey(terminalHotkeys.value, combo)) {
    case "search":
      openTerminalSearch();
      return consume();
    case "quick-select":
      if (quickSelectOpen.value) closeQuickSelect();
      else openQuickSelect();
      return consume();
    case "command-history":
      // ↑ 裸键之外的补充唤起（可配置键位，默认含 Ctrl+R——Warp Command Search
      // 的默认键 workspace:show_command_search）：同一 gate，开↔关 toggle；
      // 热键入口聚焦搜索框（批 3d）——热键用户要的就是 Warp Ctrl+R 式搜索流。
      // 总开关关闭（issue #169）或 alternate 屏（vim 的 Ctrl+R 是 redo，less/
      // tmux copy-mode 同理，#108 反馈二顺带修复）时不 consume：原样透传远端；
      // 其余 gate 关闭场景维持吞键（传输占用放行 ^R 会污染 zmodem/trzsz
      // 字节流，交互提示待答会把控制字符打进 OTP 输入）。
      if (commandHistoryHotkeyPassthrough({ enabled: historyPanelEnabled.value, alternateActive: terminal?.buffer.active.type === "alternate" })) return true;
      if (historyPanelOpen.value) closeHistoryPanel();
      else if (historyPanelGateOpen()) openHistoryPanel(true);
      return consume();
    case "completions":
      // 手动唤起结构化补全菜单（Warp completions 手动键同位）：只在输入面
      // 可用时开门——history 面板/终端搜索占用时不开（避免浮层叠在其下），
      // alternate 屏/命令运行中/传输占用与自动弹出路径同门；菜单已开时等于
      // 重算刷新。引擎 off 或无候选由 controller 原样关闭，无副作用。
      if (historyPanelOpen.value || searchOpen.value) return consume();
      if (commandRunning.value || terminalTransferBusy.value || terminal?.buffer.active.type === "alternate") return consume();
      // 用过即散：手动补全命中引导条教的键位（Ctrl+Space），此后不再教。
      dismissPromptHints();
      openCompletionsManually();
      return consume();
    case "ai-fix":
      // 失败命令 AI 修复（Warp Fix-with-AI 同位）：修复条在场才动作；缺席时
      // 放行远端（键位未占用任何远端常用语义，放行最诚实）。
      if (!hasAiFixBar()) return true;
      void aiFixFromBar();
      return consume();
    case "ai-assist":
      // 唤起 AI 助手（Warp Agent Mode 入口同位）：开关关闭/能力缺失给可见
      // 降级提示并消费；就绪则选中文本/屏幕尾部快照发起 ask 会话。
      if (!aiBridgeOk() || !isAiAssistEnabled()) {
        showNotice(t("aiAssist.unavailable"));
        return consume();
      }
      void aiAssistFromTerminal();
      return consume();
    case "copy":
      // 无选区时不消费：裸 Ctrl+C 仍要作为 SIGINT 发给远端。
      if (!terminal?.hasSelection()) return true;
      void copyTerminalSelection();
      return consume();
    case "paste":
      // 不取消默认动作：放行浏览器原生 paste 事件（自带真实 clipboardData，
      // 沙箱 iframe 中无需剪贴板读权限），由 terminalHost 的 capture 拦截器
      // 统一走风险确认。stopPropagation 挡住宿主/文档级快捷键；返回 false
      // 让 xterm 跳过该键，否则 Ctrl+V 会先作为 ^V 字符发给远端。
      event.stopPropagation();
      return false;
    case "select-all":
      selectAllTerminal();
      return consume();
    case "clear":
      clearTerminal();
      return consume();
    case "zoom-in":
      adjustTerminalZoom(1);
      return consume();
    case "zoom-out":
      adjustTerminalZoom(-1);
      return consume();
    case "reset-zoom":
      resetTerminalZoom();
      return consume();
    case "scroll-to-top":
      terminal?.scrollToTop();
      return consume();
    case "scroll-to-bottom":
      terminal?.scrollToBottom();
      return consume();
    default:
      return true;
  }
}

// ---------------------------------------------------------------------------
// 浮层键权仲裁器接线（docs/plans/IMPL_PLAN_OVERLAY_KEY_ARBITER.zh-CN.md）：
// claims 的可见性沿用各浮层既有判定（can*/gate/open 布尔），判定零迁移——
// 仲裁器只把「这个键维度此刻归谁」收口为查表。惰性求值：仅浮层分支到达时
// 才构建/采样（普通键入不读 buffer，热路径零增量）。优先级一律取
// ARBITER_PRIORITIES 表，接线处不写数字。ghost 的 → 族接受分支不查表（其
// 组合键条件即让位编码的另一半，见 overlayArbiter.ts ClaimKey 注）。
function priority(key: ClaimKey, overlay: OverlayId): number {
  return ARBITER_PRIORITIES[key][overlay] ?? 0;
}

function terminalOverlayKeyOwners(): KeyOwners {
  const panelOpen = historyPanelOpen.value;
  return resolveKeyOwners([
    // 历史面板：开着全占导航/接受/Esc；关着仅持有裸 ↑ 的唤起——gate 判
    // false 即不可见，↑ 归远端（#150 交互提示待答态由此编码）。
    ...panelClaims(panelOpen),
    // 引导条锚点：唯一锚点 claimant；其他浮层在场时其自身 overlayOpen 门
    // （shouldShowPromptHints）已置不可见，仲裁器兜底不双持。
    { overlay: "promptHints", key: "anchor", priority: priority("anchor", "promptHints"), visible: isPromptHintsVisible() },
    // ghost：→ 族词块/整段接受（让位建议浮层——数据分工；运行中/传输占用
    // 在分支内复查回车竞态）与键位胶囊菜单的 Esc；Tab 接受仅在开关开启且
    // 菜单未开时可见（对 completion 让位）。→ 族分支本体不查表（其组合键
    // 条件即让位编码的另一半），Tab/Esc 维度由 handleSuggestionKey 等查表。
    { overlay: "ghost", key: "tab", priority: priority("tab", "ghost"), visible: !!ghostMatch.value && !suggestionOpen.value && ghostTabAccept.value && !completionOpen.value },
    { overlay: "ghost", key: "accept", priority: priority("accept", "ghost"), visible: !!ghostMatch.value && !suggestionOpen.value },
    { overlay: "ghost", key: "escape", priority: priority("escape", "ghost"), visible: ghostKeycapMenuOpen.value },
    // 补全/建议/quick-select/搜索：开着即声明各自键域（判定在各 handler
    // 内，查表只决定轮不轮到它）。
    ...fourKeyClaims("completion", completionOpen.value),
    ...fourKeyClaims("suggestion", suggestionOpen.value),
    { overlay: "quickSelect", key: "arrowUp", priority: priority("arrowUp", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "arrowDown", priority: priority("arrowDown", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "enter", priority: priority("enter", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "quickSelect", key: "escape", priority: priority("escape", "quickSelect"), visible: quickSelectOpen.value },
    { overlay: "search", key: "escape", priority: priority("escape", "search"), visible: searchOpen.value },
  ]);
}

/** 面板 claims：开 = 导航/Enter/Tab/Esc 全占；关 = 仅裸 ↑ 的唤起门。
 *  唤起门叠加 issue #169 的功能开关：关闭后 gate 不可见，↑ 归远端 shell
 *  原生历史；热键入口（Ctrl+R，见 command-history 分支）同样受开关控制。 */
function panelClaims(panelOpen: boolean): ArbiterClaim[] {
  const gate = priority("arrowUp", "historyPanel");
  const arrowUpGate = panelOpen || (historyPanelGateOpen() && historyPanelEnabled.value);
  return [
    { overlay: "historyPanel", key: "arrowUp", priority: gate, visible: arrowUpGate },
    { overlay: "historyPanel", key: "arrowDown", priority: gate, visible: panelOpen },
    { overlay: "historyPanel", key: "enter", priority: priority("enter", "historyPanel"), visible: panelOpen },
    { overlay: "historyPanel", key: "tab", priority: priority("tab", "historyPanel"), visible: panelOpen },
    { overlay: "historyPanel", key: "escape", priority: priority("escape", "historyPanel"), visible: panelOpen },
  ];
}

/** 导航类浮层（补全/建议）的标准四键 claims：↑↓/Enter/Tab/Esc。 */
function fourKeyClaims(overlay: OverlayId, visible: boolean): ArbiterClaim[] {
  return (["arrowUp", "arrowDown", "enter", "tab", "escape"] as const).map((key) => ({
    overlay,
    key,
    priority: priority(key, overlay),
    visible,
  }));
}

/** 事件 → 键维度；不映射的按键（字母/修饰系）不参与浮层仲裁。 */
function keyEventClaimKey(event: KeyboardEvent): ClaimKey | null {
  if (event.key === "ArrowUp") return "arrowUp";
  if (event.key === "ArrowDown") return "arrowDown";
  if (event.key === "Enter") return "enter";
  if (event.key === "Tab") return "tab";
  if (event.key === "ArrowRight") return "accept";
  if (event.key === "Escape") return "escape";
  return null;
}

function overlayOwnsKey(owners: KeyOwners, event: KeyboardEvent, overlay: OverlayId): boolean {
  const dimension = keyEventClaimKey(event);
  return dimension !== null && owners[dimension] === overlay;
}

  return {
    handleTerminalKey,
    priority,
    terminalOverlayKeyOwners,
    panelClaims,
    fourKeyClaims,
    keyEventClaimKey,
    overlayOwnsKey,
  };
}
