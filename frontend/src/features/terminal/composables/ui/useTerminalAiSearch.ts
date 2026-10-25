// `#` AI 命令搜索的共享运行时（主终端 + 分屏/新页签）：状态机、等待冻结、
// 锚点/浮层定位、直连生成与面板会话降级都在这里，调用方只提供终端、行缓冲
// 判定、能力门与“把命令写回输入行但不执行”的动作。纯 UI 状态，不触 PTY。
import { computed, ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import {
  canActivateAiSearch,
  classifyAiSearchInput,
  createAiSearchState,
  nextAiSearchState,
  type AiSearchGates,
  type AiSearchState,
} from "../../../agent/lib/aiSearchMode";
import {
  aiCompletionAvailable,
  generateAiText,
  listAiModels,
  openAiConversation,
  pickDefaultAiModel,
} from "../../../agent/lib/aiBridge";
import { buildAiSearchPrompt, buildAiSearchRequest, type AiTerminalContext } from "../../../agent/lib/aiRequests";
import { parseAiResultText } from "../../../agent/lib/aiResultParse";
import { readTerminalCellFrame } from "../../lib/emulator/terminalAnchor";
import { choosePromptHintsPlacement, PROMPT_HINTS_FALLBACK_BAR_HEIGHT } from "../../lib/emulator/terminalPromptHints";
import { flippedOverlayBottom, overlayBelowTop, type SuggestionAnchor } from "../../../../shared/lib/overlayPlacement";

export interface TerminalAiSearchOptions {
  terminal: () => Terminal | undefined;
  /** 终端宿主（锚点与浮层包含块的度量基准）。 */
  getHost: () => HTMLElement | null | undefined;
  t: (key: string, values?: Record<string, string | number>) => string;
  /** 调用方的本地行缓冲：空行才允许 `#` 激活。 */
  getLine: () => string;
  /** 能力位 + 用户开关，每次触发时新鲜探测。 */
  isEnabled: () => boolean;
  /** 既有浮层/执行态门。 */
  gates: () => AiSearchGates;
  /** 回填命令到 shell 输入行（不回车）；调用方负责送 PTY 与本地行模型。 */
  replaceLine: (command: string) => void;
  notify?: (message: string) => void;
  context: () => AiTerminalContext;
  /** `#` 激活成功后的额外副作用（例如收起提示条）。 */
  onActivate?: () => void;
}

export function useTerminalAiSearch(options: TerminalAiSearchOptions) {
  const state = ref<AiSearchState>(createAiSearchState());
  const pending = ref(false);
  const pendingQuery = ref("");
  const anchor = ref<SuggestionAnchor | null>(null);
  const barEl = ref<HTMLElement | null>(null);
  let token = 0;

  function readAnchor(): SuggestionAnchor | null {
    const frame = readTerminalCellFrame(options.terminal(), options.getHost());
    if (!frame) return null;
    return {
      x: Math.round(frame.originLeft + frame.cursorX * frame.cellWidth),
      y: Math.round(frame.originTop + frame.visibleRow * frame.cellHeight),
      cellHeight: frame.cellHeight,
      cellWidth: frame.cellWidth,
    };
  }

  function syncAnchor() {
    anchor.value = readAnchor();
  }

  const barStyle = computed(() => {
    const current = anchor.value;
    if (!current) return undefined;
    const containerHeight = barEl.value?.parentElement?.clientHeight || options.getHost()?.clientHeight || 0;
    if (!(containerHeight > 0)) return undefined;
    const barHeight = barEl.value?.offsetHeight || PROMPT_HINTS_FALLBACK_BAR_HEIGHT;
    if (choosePromptHintsPlacement(current.y, containerHeight, barHeight) === "below") {
      return { left: `${current.x}px`, top: `${overlayBelowTop(current.y, current.cellHeight ?? 0)}px`, bottom: "auto" };
    }
    return { left: `${current.x}px`, bottom: `${flippedOverlayBottom(current.y, containerHeight)}px` };
  });

  function close() {
    state.value = createAiSearchState();
    anchor.value = null;
  }

  function clearPending() {
    pending.value = false;
    pendingQuery.value = "";
    anchor.value = null;
  }

  async function submit(query: string) {
    const trimmed = query.trim();
    close();
    if (!trimmed) return;
    const api = window.dbxPlugin;
    if (aiCompletionAvailable(api)) {
      const requestToken = ++token;
      pendingQuery.value = trimmed;
      pending.value = true;
      // close() 清了锚点，等待条仍需要它——同一 tick 光标未动，重读一次。
      anchor.value = readAnchor();
      try {
        const model = pickDefaultAiModel(await listAiModels(api));
        if (!model) throw new Error("no configured ai model");
        const result = (
          await generateAiText(api, {
            configId: model.configId,
            model: model.model,
            prompt: buildAiSearchPrompt({ query: trimmed }),
          })
        ).trim();
        if (requestToken !== token) return;
        const parsed = parseAiResultText(result);
        if (!parsed) throw new Error("empty ai response");
        options.replaceLine(parsed.command);
        options.notify?.(parsed.why || options.t("aiSearch.filledNotice"));
        return;
      } catch {
        // 用户已在等待期放弃（卸载/会话重启/Esc）：不再回退面板会话。
        if (requestToken !== token) return;
        // 直连失败（无模型/宿主确认取消/供应商错误）→ 回退面板会话。
      } finally {
        if (requestToken === token) clearPending();
      }
    }
    const sent = await openAiConversation(api, buildAiSearchRequest({ query: trimmed, context: options.context() })).catch(() => false);
    options.notify?.(options.t(sent ? "aiSearch.sentNotice" : "aiSearch.unavailable"));
  }

  /**
   * onData 入口：消费返回 true（字节不进 PTY），否则交回调用方。
   * 顺序与主终端一致：等待冻结 → 模式内编辑 → 空行 `#` 激活。
   */
  function handleData(data: string): boolean {
    if (pending.value) {
      if (data === "\u001b") {
        token += 1;
        clearPending();
        options.notify?.(options.t("aiSearch.abandonedNotice"));
      }
      return true;
    }
    if (state.value.active) {
      const previous = state.value;
      const next = nextAiSearchState(state.value, classifyAiSearchInput(data));
      state.value = next.state;
      if (next.action === "submit") void submit(previous.query);
      else if (next.action === "cancel") close();
      else syncAnchor();
      return true;
    }
    if (data === "#" && options.getLine() === "" && options.isEnabled() && canActivateAiSearch(options.gates())) {
      state.value = { active: true, query: "" };
      options.onActivate?.();
      syncAnchor();
      return true;
    }
    return false;
  }

  /** 会话重建/卸载：作废在途结果并清空全部 AI 状态。 */
  function reset() {
    token += 1;
    state.value = createAiSearchState();
    clearPending();
  }

  return { state, pending, pendingQuery, anchor, barEl, barStyle, handleData, reset };
}
