// 编辑器 Tcl 着色（插件自带实现，取代 language-data 的
// `@codemirror/legacy-modes/mode/tcl`）。
//
// 为什么不用 legacy tcl：它把双引号字符串和运算符都返回成 `comment`——
// `puts "value $a"` 的引号、`if {$a > 1}` 的 `>` 都会显示成注释灰。
// Tcl 的注释规则很简单：`#` 只有出现在「命令起始位置」（行首，或 `;` / `[` /
// `{` 之后）才开启注释；行中 `set a # b` 里的 `#` 是普通字符。
//
// 着色：注释、引号字符串（含 `$` 插值）、`$变量`（含 `${...}` / `$a(idx)`）、
// `{}`/`[]`/`;` 结构符、命令起始处的内建命令名。

import { StreamLanguage, type StreamParser, type StringStream } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { scanBalanced } from "./streamScan";

/** 常见 Tcl 内建命令（命令起始位置才着色）。 */
const KEYWORDS = new Set([
  "after", "append", "apply", "array", "break", "catch", "cd", "close", "concat",
  "continue", "coroutine", "dict", "else", "elseif", "eof", "error", "eval",
  "exec", "exit", "expr", "fblocked", "fconfigure", "file", "flush", "for",
  "foreach", "format", "gets", "glob", "global", "if", "incr", "info", "interp",
  "join", "lappend", "lindex", "linsert", "list", "llength", "lmap", "lrange",
  "lreplace", "lsearch", "lset", "lsort", "namespace", "open", "package", "pid",
  "proc", "puts", "pwd", "read", "regexp", "regsub", "rename", "return", "scan",
  "seek", "set", "socket", "source", "split", "string", "subst", "switch",
  "tailcall", "tell", "then", "time", "trace", "try", "unset", "uplevel",
  "upvar", "variable", "vwait", "while", "yield",
]);

/** 词内字符（`#`、`$`、引号、结构符另行处理）。 */
const WORD_CHARS = /[^\s;{}[\]"$#\\]/;

interface TclState {
  /** 命令起始位置：此处出现的 `#` 才是注释。 */
  commandStart: boolean;
  /** 引号字符串内的插值栈。 */
  stack: Array<(stream: StringStream, state: TclState) => string | null>;
}

/** `$name` / `${name}` / `$a(index)`（`$` 已消费）。 */
function variable(stream: StringStream): string | null {
  const next = stream.peek();
  if (next === "{") {
    scanBalanced(stream, "{", "}");
    return "variableName";
  }
  if (next && /[\w:]/.test(next)) {
    stream.eatWhile(/[\w:]/);
    if (stream.peek() === "(") scanBalanced(stream, "(", ")");
    return "variableName";
  }
  return null;
}

/** 字符串内插值入口：`$` 未消费。 */
function tokenInterpolation(stream: StringStream, state: TclState): string | null {
  state.stack.shift();
  stream.next();
  return variable(stream);
}

/** 引号字符串：`$` 处切出变量，其余按字符串。 */
function quotedString(stream: StringStream, state: TclState): string | null {
  while (!stream.eol()) {
    const ch = stream.next();
    if (ch === "\\") {
      stream.next();
    } else if (ch === '"') {
      state.stack.shift();
      return "string";
    } else if (ch === "$" && stream.peek() !== '"') {
      stream.backUp(1);
      state.stack.shift();
      state.stack.unshift(quotedString);
      state.stack.unshift(tokenInterpolation);
      return "string";
    }
  }
  return "string";
}

function tokenBase(stream: StringStream, state: TclState): string | null {
  if (stream.sol()) state.commandStart = true;
  if (stream.eatSpace()) return null;
  const ch = stream.next();
  if (!ch) return null;

  if (ch === "#") {
    if (state.commandStart) {
      stream.skipToEnd();
      return "comment";
    }
    stream.eatWhile(WORD_CHARS);
    return null;
  }
  if (ch === "\\") {
    stream.next();
    return null;
  }
  if (ch === '"') {
    state.commandStart = false;
    state.stack.unshift(quotedString);
    return quotedString(stream, state);
  }
  if (ch === "$") {
    state.commandStart = false;
    return variable(stream);
  }
  if (ch === ";" || ch === "[" || ch === "{") {
    state.commandStart = true;
    return "punctuation";
  }
  if (ch === "]" || ch === "}") {
    state.commandStart = false;
    return "punctuation";
  }

  stream.eatWhile(WORD_CHARS);
  const word = stream.current();
  if (!word) return null;
  const atCommandStart = state.commandStart;
  state.commandStart = false;
  return atCommandStart && KEYWORDS.has(word) ? "keyword" : null;
}

const tclParser: StreamParser<TclState> = {
  name: "tcl",
  startState: () => ({ commandStart: true, stack: [] }),
  copyState: (state) => ({ commandStart: state.commandStart, stack: state.stack.slice() }),
  token: (stream, state) => (state.stack.length ? state.stack[0]!(stream, state) : tokenBase(stream, state)),
  blankLine: () => false,
  languageData: {
    commentTokens: { line: "#" },
  },
};

let cached: Extension | undefined;

/** Tcl 语言扩展。 */
export function tclLanguage(): Extension {
  cached ??= StreamLanguage.define(tclParser);
  return cached;
}
