// 日志文件的只读语法着色：把可见范围内的行按 logTokens 的语义切成装饰。
//
// 只处理可视区域（view.visibleRanges），十万行日志也不会有整篇扫描开销；
// 文档或视口变化时重建。颜色类名与容器日志页共用（app/styles/global/log.css），
// 亮暗主题由 CSS 变量跟随宿主，不需要在扩展里再分色系。

import type { Extension, Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { tokenizeLogLine } from "../../../../shared/lib/logTokens";

/** 超长行不装饰（压缩 JSON 日志、二进制残留），避免正则回溯。 */
const MAX_LINE_CHARS = 4000;

function decorateLine(out: Range<Decoration>[], lineFrom: number, text: string): void {
  if (!text || text.length > MAX_LINE_CHARS) return;
  const { tokens } = tokenizeLogLine(text);
  let offset = lineFrom;
  for (const token of tokens) {
    const end = offset + token.text.length;
    if (token.kind !== "plain" && end > offset) {
      out.push(Decoration.mark({ class: `log-${token.kind}` }).range(offset, end));
    }
    offset = end;
  }
}

function buildDecorations(view: EditorView): DecorationSet {
  const marks: Range<Decoration>[] = [];
  const doc = view.state.doc;
  for (const range of view.visibleRanges) {
    let line = doc.lineAt(range.from);
    while (line.from <= range.to) {
      decorateLine(marks, line.from, line.text);
      if (line.to >= doc.length) break;
      line = doc.lineAt(line.to + 1);
    }
  }
  return Decoration.set(marks, true);
}

const logHighlighter = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  {
    decorations: (plugin) => plugin.decorations,
  },
);

export function logHighlightExtension(): Extension {
  return logHighlighter;
}
