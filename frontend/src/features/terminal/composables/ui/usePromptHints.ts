import { computed, ref, watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { SuggestionAnchor } from "../../../../shared/lib/overlayPlacement";
import { formatHotkeyDisplay, type TerminalHotkeyActionId, type TerminalHotkeyBindings } from "../../lib/terminalHotkeys";
import { isPasswordPromptLine, loadPromptHintsDismissed, loadPromptHintsEnabled, savePromptHintsDismissed, savePromptHintsEnabled, shouldShowPromptHints } from "../../lib/terminalPromptHints";
import { interactivePromptNearCursor, isInteractivePromptPending } from "../../lib/interactivePromptGuard";

/**
 * 空提示符快捷键引导条（TerminalPromptHints）：引导 Warp 对齐批次的键位
 * （↑ 历史 / Ctrl+R 搜索 / Ctrl+Space 补全 / → 接受行内建议）。开关与一次性
 * 消散旗标存 pluginStore；显隐门在 lib/terminalPromptHints.ts（纯函数），
 * 行空判定由调用方经 promptHintsLineEmpty 两个汇点镜像；用户实际用过任一被
 * 引导功能（开面板/手动补全/接受 ghost）即永久消散。
 *
 * 调用点必须留在 setup 尾部：watch 立即求值 computed（→ terminalTransferBusy
 * → zmodemBusy），依赖链上有后段才声明的 const，放前面必踩 TDZ。
 */
export function usePromptHints(options: {
  terminal: () => Terminal | undefined;
  isLocalMode: Ref<boolean>;
  session: Ref<{ sessionId?: string } | undefined>;
  commandRunning: Ref<boolean>;
  terminalTransferBusy: Ref<boolean>;
  suggestionOpen: Ref<boolean>;
  completionOpen: Ref<boolean>;
  historyPanelOpen: Ref<boolean>;
  quickSelectOpen: Ref<boolean>;
  searchOpen: Ref<boolean>;
  aiSearchActive: () => boolean;
  hostKeyPrompt: Ref<unknown>;
  terminalHotkeys: Ref<TerminalHotkeyBindings>;
  applePlatform: boolean;
  readTerminalSuggestionAnchor: () => SuggestionAnchor | null;
}) {
  const {
    terminal, isLocalMode, session, commandRunning, terminalTransferBusy,
    suggestionOpen, completionOpen, historyPanelOpen, quickSelectOpen, searchOpen,
    aiSearchActive, hostKeyPrompt, terminalHotkeys, applePlatform, readTerminalSuggestionAnchor,
  } = options;

  const promptHintsEnabled = ref(loadPromptHintsEnabled());
  const promptHintsDismissed = ref(loadPromptHintsDismissed());
  const promptHintsLineEmpty = ref(true);
  const promptHintsAnchor = ref<SuggestionAnchor | null>(null);
  // 输出活跃抑制（真机反馈：登录 banner 刷屏期引导条闪现遮挡输出）：写输出即
  // 置 quiet=false，600ms 静默后放行（防抖单 timer；会话复位归位）。
  const promptHintsOutputQuiet = ref(true);
  let promptHintsQuietTimer: ReturnType<typeof setTimeout> | undefined;

  function markTerminalOutputActive() {
    promptHintsOutputQuiet.value = false;
    if (promptHintsQuietTimer) clearTimeout(promptHintsQuietTimer);
    promptHintsQuietTimer = setTimeout(() => {
      promptHintsQuietTimer = undefined;
      promptHintsOutputQuiet.value = true;
    }, 600);
  }

  // 密码提示启发采样：光标行与上一行文本命中 password/passphrase 提示 → 抑制。
  function passwordPromptOnScreen(): boolean {
    try {
      const buffer = terminal()?.buffer.active;
      if (!buffer || buffer.type !== "normal") return false;
      const row = buffer.baseY + Math.min(buffer.cursorY, buffer.length - 1);
      for (let y = row; y >= Math.max(0, row - 1); y -= 1) {
        if (isPasswordPromptLine(buffer.getLine(y)?.translateToString(true) ?? "")) return true;
      }
    } catch {
      return false;
    }
    return false;
  }

  // 终端交互提示采样（issue #150 及其反馈扩展）：光标行邻域命中询问输入/
  // 选择提示（动态码、密码、选择菜单、y/n 确认）→「交互待答态」。纯判定在
  // lib/interactivePromptGuard，这里只做 xterm buffer 读取。
  function interactivePromptOnScreen(): boolean {
    try {
      const buffer = terminal()?.buffer.active;
      if (!buffer || buffer.type !== "normal") return false;
      const row = buffer.baseY + Math.min(buffer.cursorY, buffer.length - 1);
      return interactivePromptNearCursor(row, (line) => buffer.getLine(line)?.translateToString(true) ?? "");
    } catch {
      return false;
    }
  }

  /** 「终端交互提示待答」门：交互提示行在场（远端询问输入/要求选择）或连接
   *  挑战弹窗未决（host-key/连接挑战 workbench 回退）任一成立。惰性求值：仅
   *  键入/按键期采样，无 setup 顺序约束。 */
  function interactivePromptGatePending(): boolean {
    return isInteractivePromptPending({
      promptLineOnScreen: interactivePromptOnScreen(),
      challengeDialogPending: !!hostKeyPrompt.value,
    });
  }

  // 引导条键位标签跟随实时绑定（不写死键名）：历史搜索取 command-history 的
  // 首个 Ctrl 系绑定（默认表里即 Ctrl+R，⌘⇧H 在前但 mac 上 ⌘ 系不是终端键位
  // 心智），补全取 completions 首个绑定（默认 Ctrl+/）。解绑则隐藏对应项。
  function promptHintsKeyLabel(actionId: TerminalHotkeyActionId, preferCtrl: boolean): string {
    const combos = terminalHotkeys.value[actionId] ?? [];
    if (!combos.length) return "";
    const pick = preferCtrl ? (combos.find((combo) => combo.startsWith("Ctrl+")) ?? combos[0]) : combos[0];
    return formatHotkeyDisplay(pick, applePlatform);
  }
  const promptHintsHistoryKey = computed(() => promptHintsKeyLabel("command-history", true));
  const promptHintsCompletionsKey = computed(() => promptHintsKeyLabel("completions", false));
  const promptHintsVisible = computed(() =>
    shouldShowPromptHints({
      enabled: promptHintsEnabled.value,
      dismissed: promptHintsDismissed.value,
      sessionActive: isLocalMode.value || !!session.value?.sessionId,
      lineEmpty: promptHintsLineEmpty.value,
      alternateActive: terminal()?.buffer.active.type === "alternate",
      commandRunning: commandRunning.value,
      transferBusy: terminalTransferBusy.value,
      overlayOpen: suggestionOpen.value || completionOpen.value || historyPanelOpen.value || quickSelectOpen.value || searchOpen.value,
      aiSearchActive: aiSearchActive(),
      outputQuiet: promptHintsOutputQuiet.value,
      passwordPromptOnScreen: passwordPromptOnScreen(),
    }),
  );
  watch(promptHintsVisible, (visible) => {
    if (visible) promptHintsAnchor.value = readTerminalSuggestionAnchor();
  });

  function setPromptHintsEnabled(value: boolean) {
    promptHintsEnabled.value = value;
    savePromptHintsEnabled(value);
    // 重开开关即清除消散旗标——用户明确要再看引导。
    if (value && promptHintsDismissed.value) {
      promptHintsDismissed.value = false;
      savePromptHintsDismissed(false);
    }
  }

  /** 消散（✗ 或用过任一功能）：置位 + 持久化，本安装不再弹。 */
  function dismissPromptHints() {
    if (promptHintsDismissed.value) return;
    promptHintsDismissed.value = true;
    savePromptHintsDismissed(true);
  }

  /** 会话复位：清掉输出活跃抑制的防抖 timer 并直接放行。 */
  function resetPromptHintsQuiet() {
    if (promptHintsQuietTimer) {
      clearTimeout(promptHintsQuietTimer);
      promptHintsQuietTimer = undefined;
    }
    promptHintsOutputQuiet.value = true;
  }

  /** 卸载兜底：清 timer（状态本身随组件销毁）。 */
  function disposePromptHints() {
    if (promptHintsQuietTimer) {
      clearTimeout(promptHintsQuietTimer);
      promptHintsQuietTimer = undefined;
    }
  }

  return {
    promptHintsEnabled,
    promptHintsDismissed,
    promptHintsLineEmpty,
    promptHintsAnchor,
    markTerminalOutputActive,
    promptHintsHistoryKey,
    promptHintsCompletionsKey,
    promptHintsVisible,
    setPromptHintsEnabled,
    dismissPromptHints,
    interactivePromptGatePending,
    resetPromptHintsQuiet,
    disposePromptHints,
  };
}
