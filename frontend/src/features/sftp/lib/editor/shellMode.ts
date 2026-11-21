// 编辑器 Shell 着色（插件自带实现，取代 language-data 里的
// `@codemirror/legacy-modes/mode/shell`）。
//
// 为什么不用 legacy shell：它把任意位置的 `#` 都当成注释起点，于是
//   count=$#            →  "`#` 之后整段灰"
//   name=${path##*/}    →  同上（词中 `#` 是模式截断运算符）
//   url=http://h/a#b    →  同上（URL 片段）
//   case a#b)           →  同上（case 模式里的词中 #）
// 行里根本没有注释却整段显示成注释灰。POSIX 规则是只有「词首」的 `#` 才开启
// 注释：行首，或前一个字符是空白/`;`/`&`/`|`/`(`/`)`/`<`/`>`/反引号。`$#`、
// `${x#y}`、`a#b`、`a=#b` 的 `#` 都在词中，不是注释。
//
// 本实现按该规则判定注释，并补齐变量（`$var` / `${...}` / `$1` / `$@`）、
// 保留字、内建命令、字符串内插值、heredoc 正文（不当注释/命令着色）。

import { StreamLanguage, type StreamParser, type StringStream } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { scanBalanced } from "./streamScan";

/** bash 保留字。 */
const KEYWORDS = new Set([
  "if", "then", "elif", "else", "fi", "case", "esac", "for", "select", "while",
  "until", "do", "done", "in", "function", "time", "coproc",
]);

/** shell 内建命令（外部命令保持默认前景，避免满屏彩色）。 */
const BUILTINS = new Set([
  "alias", "bg", "bind", "break", "builtin", "caller", "cd", "command", "compgen",
  "complete", "continue", "declare", "dirs", "disown", "echo", "enable", "eval",
  "exec", "exit", "export", "false", "fc", "fg", "getopts", "hash", "help",
  "history", "jobs", "kill", "let", "local", "logout", "mapfile", "popd",
  "printf", "pushd", "pwd", "read", "readarray", "readonly", "return", "set",
  "shift", "shopt", "source", "suspend", "test", "times", "trap", "true",
  "type", "typeset", "ulimit", "umask", "unalias", "unset", "wait",
]);

/** 词中字符：路径、URL、glob、`--flag`、`${}` 之外的花括号块都按普通词吞掉。 */
const WORD_CHARS = /[^\s"'`$#;|&()<>=!\\]/;

/** `#` 在这些字符之后仍处于「词首」，可以开启注释（bash 实测）。 */
const COMMENT_PRECEDERS = ";|&()<>`";

/** 词中 `#`（`$#`、`${x#y}`、`a#b`、`a=#b`）不开启注释。 */
function startsComment(stream: StringStream): boolean {
  if (stream.start === 0) return true;
  const previous = stream.string[stream.start - 1] ?? "";
  return /\s/.test(previous) || COMMENT_PRECEDERS.includes(previous);
}

type ShellTokenizer = (stream: StringStream, state: ShellState) => string | null;

interface ShellState {
  /** 字符串/插值嵌套栈：栈顶 tokenizer 优先消费当前字符。 */
  stack: ShellTokenizer[];
  /** 待消费的 heredoc 正文（`<<EOF` 之后的行）。 */
  heredoc?: { delimiter: string; stripTabs: boolean };
}

/** heredoc 正文：整行按字符串着色，遇到结束标记的那一行恢复常规解析。 */
function heredocLine(stream: StringStream, state: ShellState): string | null {
  const heredoc = state.heredoc!;
  const line = stream.string.slice(stream.pos);
  const candidate = heredoc.stripTabs ? line.replace(/^\t+/, "") : line;
  if (candidate.trimEnd() === heredoc.delimiter) {
    state.heredoc = undefined;
    stream.skipToEnd();
    return null;
  }
  stream.skipToEnd();
  return "string";
}

/** 单引号字符串：不做插值，允许跨行。 */
function tokenSingleQuote(stream: StringStream, state: ShellState): string | null {
  while (!stream.eol()) {
    if (stream.next() === "'") {
      state.stack.shift();
      break;
    }
  }
  return "string";
}

/** `$` 之后的变量形态；返回 null 表示裸 `$`（按普通字符处理）。 */
function dollarAfter(stream: StringStream): string | null {
  const next = stream.peek();
  if (next === "{") {
    // `${var}` / `${#arr[@]}` / `${path##*/}`：整段一个变量 token，
    // 里面的 `#` 不再有注释语义。
    scanBalanced(stream, "{", "}");
    return "variableName";
  }
  if (next === "(") {
    // `$(cmd)` / `$((expr))`：内容不做着色（内部语法差异大，错误着色更扰人）。
    scanBalanced(stream, "(", ")");
    return null;
  }
  if (next && /[@*#?$!0-9-]/.test(next)) {
    stream.next();
    return "variableName";
  }
  if (next && /[A-Za-z_]/.test(next)) {
    stream.eatWhile(/[\w]/);
    return "variableName";
  }
  return null;
}

/** 字符串内插值入口：`$` 未消费，处理完变量后把字符串 tokenizer 顶回栈。 */
function tokenInterpolation(stream: StringStream, state: ShellState): string | null {
  state.stack.shift();
  stream.next();
  return dollarAfter(stream);
}

/** 双引号字符串：遇 `$` 切出变量 token，其余按字符串。 */
function tokenDoubleQuote(stream: StringStream, state: ShellState): string | null {
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
      state.stack.unshift(tokenDoubleQuote);
      state.stack.unshift(tokenInterpolation);
      return "string";
    }
  }
  return "string";
}

/** 反引号命令替换：与双引号同一套插值处理，结束符是反引号。 */
function tokenBacktick(stream: StringStream, state: ShellState): string | null {
  while (!stream.eol()) {
    const ch = stream.next();
    if (ch === "\\") {
      stream.next();
    } else if (ch === "`") {
      state.stack.shift();
      return "string";
    } else if (ch === "$" && stream.peek() !== "`") {
      stream.backUp(1);
      state.stack.shift();
      state.stack.unshift(tokenBacktick);
      state.stack.unshift(tokenInterpolation);
      return "string";
    }
  }
  return "string";
}

/** `<<EOF` / `<<-EOF` / `<<'EOF'`：登记正文结束标记。 */
function heredocStart(stream: StringStream, state: ShellState): string | null {
  stream.next(); // 第二个 "<"
  if (stream.peek() === "<") {
    stream.next(); // <<< here-string
    return "operator";
  }
  const stripTabs = stream.eat("-") !== undefined;
  stream.eatSpace();
  const quote = stream.peek();
  let delimiter = "";
  if (quote === "'" || quote === '"') {
    stream.next();
    const end = stream.string.indexOf(quote, stream.pos);
    if (end >= 0) {
      delimiter = stream.string.slice(stream.pos, end);
      stream.pos = end + 1;
    }
  } else {
    const match = /^[A-Za-z_][\w-]*/.exec(stream.string.slice(stream.pos));
    if (match) {
      delimiter = match[0];
      stream.pos += match[0].length;
    }
  }
  if (!delimiter) return "operator";
  state.heredoc = { delimiter, stripTabs };
  return "string.special";
}

function tokenBase(stream: StringStream, state: ShellState): string | null {
  if (stream.sol() && state.heredoc) return heredocLine(stream, state);
  if (stream.eatSpace()) return null;
  const ch = stream.next();
  if (!ch) return null;

  if (ch === "\\") {
    stream.next();
    return null;
  }
  if (ch === "#") {
    if (startsComment(stream)) {
      stream.skipToEnd();
      return "comment";
    }
    stream.eatWhile(WORD_CHARS);
    return null;
  }
  if (ch === "'") {
    state.stack.unshift(tokenSingleQuote);
    return tokenSingleQuote(stream, state);
  }
  if (ch === '"') {
    state.stack.unshift(tokenDoubleQuote);
    return tokenDoubleQuote(stream, state);
  }
  if (ch === "`") {
    state.stack.unshift(tokenBacktick);
    return tokenBacktick(stream, state);
  }
  if (ch === "$") return dollarAfter(stream);
  if (ch === "<" && stream.peek() === "<") return heredocStart(stream, state);
  if (/\d/.test(ch)) {
    stream.eatWhile(/\d/);
    return "number";
  }
  if ("=!;|&<>()".includes(ch)) return "operator";

  stream.eatWhile(WORD_CHARS);
  const word = stream.current();
  if (!word) return null;
  // `NAME=value` 的赋值名（`=` 之后单独着色）。
  if (stream.peek() === "=") return "variableName.definition";
  if (KEYWORDS.has(word)) return "keyword";
  if (BUILTINS.has(word)) return "variableName.standard";
  return null;
}

const shellParser: StreamParser<ShellState> = {
  name: "shell",
  startState: () => ({ stack: [] }),
  copyState: (state) => ({
    stack: state.stack.slice(),
    heredoc: state.heredoc ? { ...state.heredoc } : undefined,
  }),
  token: (stream, state) => (state.stack.length ? state.stack[0]!(stream, state) : tokenBase(stream, state)),
  blankLine: () => false,
  languageData: {
    commentTokens: { line: "#" },
    closeBrackets: { brackets: ["(", "[", "{", "'", '"', "`"] },
  },
};

/** Shell 语言扩展（含 `#` 注释 token，供 basicSetup 的注释快捷键使用）。 */
let cached: Extension | undefined;

export function shellLanguage(): Extension {
  cached ??= StreamLanguage.define(shellParser);
  return cached;
}
