import { useTerminalHighlight, type TerminalHighlightOptions } from "./useTerminalHighlight";
import { useTerminalActionLinks, type TerminalActionLinksOptions } from "./useTerminalActionLinks";
import { useTerminalGutter, type TerminalGutterOptions } from "./useTerminalGutter";

export type TerminalDecorationsOptions =
  & TerminalHighlightOptions
  & TerminalActionLinksOptions
  & TerminalGutterOptions;

/**
 * 主终端三类装饰引擎（关键词高亮 / 动作链接 / 行号时间戳 gutter）的聚合入口：
 * 各自独立装配、返回值平铺合并，供工作台聚合器一次性接走。
 */
export function useTerminalDecorations(options: TerminalDecorationsOptions) {
  const highlight = useTerminalHighlight(options);
  const actionLinks = useTerminalActionLinks(options);
  const gutter = useTerminalGutter(options);
  return { ...highlight, ...actionLinks, ...gutter };
}
