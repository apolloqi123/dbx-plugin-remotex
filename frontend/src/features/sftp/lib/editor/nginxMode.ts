// 编辑器 Nginx 配置着色（插件自带实现，取代 language-data 的
// `@codemirror/legacy-modes/mode/nginx`）。
//
// 为什么不用 legacy nginx：它是从 CSS 模式改出来的，会把 `/*` 当成 C 风格注释
// 起点。nginx 路径里的通配符恰好长这样：
//   include /usr/share/nginx/modules/*.conf;
// 于是从 `/*` 开始整篇被当成注释，直到文件末尾再也没有 `*/`——用户看到的就是
// 「后面所有行都灰了」。nginx 本身只有 `#` 注释（见 ngx_conf_file.c：
// `case '#': sharp_comment = 1` 只出现在 `last_space` 分支），因此：
//   - `#` 仅在「词首」开启注释；`proxy_pass http://h/a#b;` 里的 `#` 属于词内；
//   - 不再识别 `/* */` 与 `<!-- -->`。
//
// 着色：指令名（语句首词）、`$变量`、引号字符串（含 `$` 插值）、数字（含
// `10m`/`65s` 这类单位）、`;{}()` 与 `~`/`=` 运算符。

import { StreamLanguage, type StreamParser, type StringStream } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { scanBalanced } from "./streamScan";

/** 词内字符：nginx 的词吃到空白或 `;{}()` 为止（`#`、`*`、`/` 都在词内）；
 *  `$` 单独切出，裸词里的 `$var` / `$1` 也要着色。 */
const WORD_CHARS = /[^\s;{}()"'=~!\\$]/;

/** 数字参数：`1024`、`1.5`、`300m`、`65s`、`2h`。 */
const NUMBER = /^\d+(?:\.\d+)?(?:ms|[kmgsdhwMy])?$/i;

interface NginxState {
  /** 语句起点：下一个词是指令名。 */
  statementStart: boolean;
  /** 引号字符串内的插值栈。 */
  stack: Array<(stream: StringStream, state: NginxState) => string | null>;
}

/** `$name` / `${name}` / `$1`（rewrite 捕获组）。 */
function variable(stream: StringStream): string | null {
  const next = stream.peek();
  if (next === "{") {
    scanBalanced(stream, "{", "}");
    return "variableName";
  }
  if (next && /[\w]/.test(next)) {
    stream.eatWhile(/[\w]/);
    return "variableName";
  }
  return null;
}

/** 字符串内插值入口：`$` 未消费。 */
function tokenInterpolation(stream: StringStream, state: NginxState): string | null {
  state.stack.shift();
  stream.next();
  return variable(stream);
}

/** 引号字符串：`$` 处切出变量，其余按字符串。 */
function quotedToken(quote: string) {
  return (stream: StringStream, state: NginxState): string | null => {
    while (!stream.eol()) {
      const ch = stream.next();
      if (ch === "\\") {
        stream.next();
      } else if (ch === quote) {
        state.stack.shift();
        return "string";
      } else if (ch === "$" && stream.peek() !== quote) {
        stream.backUp(1);
        state.stack.shift();
        state.stack.unshift(quotedToken(quote));
        state.stack.unshift(tokenInterpolation);
        return "string";
      }
    }
    return "string";
  };
}

function tokenBase(stream: StringStream, state: NginxState): string | null {
  if (stream.eatSpace()) return null;
  const ch = stream.next();
  if (!ch) return null;

  // 词首的 `#` 才是注释（nginx 的 sharp_comment 只在 last_space 时置位）。
  if (ch === "#") {
    stream.skipToEnd();
    return "comment";
  }
  if (ch === "\\") {
    stream.next();
    return null;
  }
  if (ch === '"' || ch === "'") {
    state.statementStart = false;
    state.stack.unshift(quotedToken(ch));
    return state.stack[0]!(stream, state);
  }
  if (ch === "$") {
    state.statementStart = false;
    return variable(stream);
  }
  if (ch === ";" || ch === "{" || ch === "}") {
    state.statementStart = true;
    return "punctuation";
  }
  if (ch === "(" || ch === ")") {
    state.statementStart = false;
    return "punctuation";
  }
  if (ch === "=" || ch === "~" || ch === "!") {
    if (ch !== "=" && stream.peek() === "~") stream.next();
    if (stream.peek() === "*") stream.next();
    state.statementStart = false;
    return "operator";
  }

  stream.eatWhile(WORD_CHARS);
  const word = stream.current();
  if (!word) return null;
  if (state.statementStart) {
    state.statementStart = false;
    return "keyword";
  }
  return NUMBER.test(word) ? "number" : null;
}

const nginxParser: StreamParser<NginxState> = {
  name: "nginx",
  startState: () => ({ statementStart: true, stack: [] }),
  copyState: (state) => ({ statementStart: state.statementStart, stack: state.stack.slice() }),
  token: (stream, state) => (state.stack.length ? state.stack[0]!(stream, state) : tokenBase(stream, state)),
  blankLine: () => false,
  languageData: {
    commentTokens: { line: "#" },
  },
};

let cached: Extension | undefined;

/** Nginx 配置语言扩展。 */
export function nginxLanguage(): Extension {
  cached ??= StreamLanguage.define(nginxParser);
  return cached;
}
