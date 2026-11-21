// CodeMirror `StreamLanguage` 的公共扫描工具（shell / makefile 两个自带模式共用）。

import type { StringStream } from "@codemirror/language";

/**
 * 从配对的括号起点扫到结束符：进入时 `open` 尚未消费，返回时 `close` 已消费。
 * 跳过单/双引号内容与 `\` 转义（`$(echo "a)b")` 不会提前结束）；到行尾仍未
 * 闭合就停在行尾，由下一行继续由调用方处理。
 */
export function scanBalanced(stream: StringStream, open: string, close: string): void {
  stream.next();
  let depth = 1;
  while (!stream.eol() && depth > 0) {
    const ch = stream.next();
    if (ch === "\\") {
      stream.next();
    } else if (ch === "'") {
      while (!stream.eol() && stream.next() !== "'") { /* 单引号内不解析 */ }
    } else if (ch === '"') {
      while (!stream.eol()) {
        const inner = stream.next();
        if (inner === "\\") stream.next();
        else if (inner === '"') break;
      }
    } else if (ch === open) {
      depth += 1;
    } else if (ch === close) {
      depth -= 1;
    }
  }
}
