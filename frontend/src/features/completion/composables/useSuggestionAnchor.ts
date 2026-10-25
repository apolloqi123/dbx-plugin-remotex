import type { Terminal } from "@xterm/xterm";
import type { SuggestionAnchor } from "../../../shared/lib/overlayPlacement";
import { ref } from "vue";
import { readTerminalCellFrame as readTerminalCellFrameAt, type TerminalCellFrame } from "../../terminal/lib/emulator/terminalAnchor";

/** 浮层锚点读取与回显后重锚（issue #120「浮层离焦点太远」）：建议浮层与
 *  结构化补全浮层共用同一光标格锚点源与生命周期钩子；两浮层的开合态以
 *  惰性 getter 注入（构造期另一侧 ref 可能尚未创建，rAF 回调期才读取）。 */
export function useSuggestionAnchor(options: {
  terminal: () => Terminal | undefined;
  getTerminalHost: () => HTMLElement | null | undefined;
  isSuggestionOpen: () => boolean;
  isCompletionOpen: () => boolean;
  isHistoryPanelOpen: () => boolean;
  syncHistoryPanelAnchor: () => void;
}) {
  const { terminal: terminalGet, getTerminalHost, isSuggestionOpen, isCompletionOpen, isHistoryPanelOpen, syncHistoryPanelAnchor } = options;

  // 两个浮层的锚点（同一光标格来源）：建议浮层由本 composable 的历史建议
  // 分支写，补全浮层由 useCompletionOverlay 写。
  const suggestionAnchor = ref<SuggestionAnchor | null>(null);
  const completionAnchor = ref<SuggestionAnchor | null>(null);

  // 建议浮层锚点的回显同步：onData 时刻回显往往还在路上（SSH RTT），buffer
  // 光标停在旧位置，浮层锚点持续滞后于输入；每批输出解析完成
  // （settleOutputChunk）后重读锚点，浮层吸附到真实光标位。rAF 合帧：大量
  // 输出（tail -f）时每帧至多测一次 rect。
  let suggestionAnchorSyncScheduled = false;
  function syncSuggestionAnchorsOnSettle() {
    if (suggestionAnchorSyncScheduled) return;
    suggestionAnchorSyncScheduled = true;
    requestAnimationFrame(() => {
      suggestionAnchorSyncScheduled = false;
      if (isSuggestionOpen()) suggestionAnchor.value = readTerminalSuggestionAnchor();
      if (isCompletionOpen()) completionAnchor.value = readTerminalSuggestionAnchor();
      if (isHistoryPanelOpen()) syncHistoryPanelAnchor();
    });
  }

  // 翻转定位用的可视底界（terminal-host 实际高度，含让位后的净高）：
  // anchor 计算时顺带刷新，两个建议浮层据此决定下方/上方放置。
  const suggestionViewport = ref({ height: 0 });

  /**
   * 光标像素锚点：xterm 私有渲染尺寸（css.cell 宽高）× 光标缓冲坐标。
   * 读不到（渲染器未就绪/内部结构变化）返回 null，浮层降级贴终端底部。
   * 浮层与 ghost 共用的光标格锚点源（issue #120）：以 .xterm-screen 为原点，
   * 加光标网格坐标；具体测量收口在 terminalAnchor（主终端与分屏窗格共用）。
   */
  function readTerminalCellFrame(): TerminalCellFrame | null {
    return readTerminalCellFrameAt(terminalGet(), getTerminalHost());
  }

  function readTerminalSuggestionAnchor(): SuggestionAnchor | null {
    const frame = readTerminalCellFrame();
    if (!frame || !getTerminalHost()) return null;
    // 翻转定位的可视底界 = terminal-host 实际高度：batch-bar/标记条让位
    // （inset-bottom）后它比包含块 pane 矮，必须用 host 高度，否则浮层会
    // 越过终端文字区盖住 footer/批量条（issue #120 实机反馈）。
    suggestionViewport.value = { height: getTerminalHost()?.clientHeight ?? 0 };
    return {
      x: Math.round(frame.originLeft + frame.cursorX * frame.cellWidth),
      y: Math.round(frame.originTop + frame.visibleRow * frame.cellHeight),
      cellHeight: frame.cellHeight,
      cellWidth: frame.cellWidth,
    };
  }

  return {
    suggestionAnchor,
    completionAnchor,
    suggestionViewport,
    readTerminalCellFrame,
    readTerminalSuggestionAnchor,
    syncSuggestionAnchorsOnSettle,
  };
}
