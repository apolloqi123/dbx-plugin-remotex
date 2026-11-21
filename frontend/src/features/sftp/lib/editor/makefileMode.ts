// 编辑器 Makefile 着色（插件自带实现）。
//
// language-data 里没有 Makefile 语言，这类文件此前整篇按纯文本显示；服务器上
// 又很常见。这里给出一套够用的着色：注释、变量引用（`$(VAR)` / `${VAR}` /
// `$@`）、赋值名、目标名、指令（`include` / `ifeq` / `define`…）与字符串。
//
// 注释规则：`#` 到行尾（`$(shell echo '#')` 这类括号内的 `#` 已被整段变量
// 引用吞掉，不会误判）。

import { StreamLanguage, type StreamParser, type StringStream } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { scanBalanced } from "./streamScan";

/** make 指令（行首出现时着色为关键字）。 */
const DIRECTIVES = new Set([
  "include", "-include", "sinclude", "ifeq", "ifneq", "ifdef", "ifndef",
  "else", "endif", "define", "endef", "export", "unexport", "override",
  "undefine", "vpath",
]);

/** 词中字符：目标/依赖/变量名里的 `.`、`-`、`/`、`%`、`*` 都算词内。 */
const WORD_CHARS = /[^\s:=#|;(){}"'\\]/;

function tokenBase(stream: StringStream): string | null {
  if (stream.eatSpace()) return null;
  const ch = stream.next();
  if (!ch) return null;

  if (ch === "#") {
    stream.skipToEnd();
    return "comment";
  }
  if (ch === "\\") {
    stream.next();
    return null;
  }
  if (ch === "$") {
    const next = stream.peek();
    if (next === "(" || next === "{") {
      scanBalanced(stream, next, next === "(" ? ")" : "}");
      return "variableName";
    }
    if (next && /[@<^?*+%]/.test(next)) {
      stream.next();
      return "variableName";
    }
    return null;
  }
  if (ch === '"' || ch === "'") {
    const quote = ch;
    while (!stream.eol()) {
      const inner = stream.next();
      if (inner === "\\") stream.next();
      else if (inner === quote) break;
    }
    return "string";
  }
  if (ch === "=") return "operator";
  if ((ch === ":" || ch === "?" || ch === "+" || ch === "!") && stream.peek() === "=") {
    stream.next();
    return "operator";
  }
  if (ch === ":" || ch === "|" || ch === ";" || ch === "?" || ch === "+") return "punctuation";

  stream.eatWhile(WORD_CHARS);
  const word = stream.current();
  if (!word) return null;
  // 配方行（Tab 开头）交给 shell，`--flag=value` 不是 make 赋值。
  const recipe = stream.string.startsWith("\t");
  const rest = stream.string.slice(stream.pos);
  // `VAR =` / `VAR :=` / `VAR ?=` / `VAR +=`：赋值名（`=` 前允许空格）。
  if (!recipe && /^\s*(?::=|\?=|\+=|!=|=)/.test(rest)) return "variableName.definition";
  // `目标:`（行首或紧跟上一条规则之后；`:=` 已在上面排除）。
  if (!recipe && /^\s*:(?!=)/.test(rest)) {
    const linePrefix = stream.string.slice(0, stream.start).trim();
    return linePrefix === "" || linePrefix.endsWith(":") ? "propertyName" : null;
  }
  if (DIRECTIVES.has(word)) return "keyword";
  return null;
}

const makefileParser: StreamParser<Record<string, never>> = {
  name: "makefile",
  startState: () => ({}),
  token: (stream) => tokenBase(stream),
  blankLine: () => false,
  languageData: {
    commentTokens: { line: "#" },
  },
};

/** Makefile 语言扩展。 */
let cached: Extension | undefined;

export function makefileLanguage(): Extension {
  cached ??= StreamLanguage.define(makefileParser);
  return cached;
}
