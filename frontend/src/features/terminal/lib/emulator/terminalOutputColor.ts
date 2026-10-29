// 终端「输出着色」转换器（tailspin / Warp 风格）：写进 xterm 之前，把结构化的
// 明文行（JSON、logfmt、级别日志、systemd `Key: value`、树形行、含 URL/IP 的行）
// 按语义包上 ANSI SGR。远端程序、复制、搜索、AI 采集拿到的仍是原始字节——
// 这里只改「显示」。
//
// 与日志文件/容器日志页共用 shared/lib/logTokens 的同一套分词，颜色取 xterm 的
// 16 色（跟随用户终端配色方案），因此三处观感一致。
//
// 安全门（任一命中即原样透传，绝不改写）：
// - 用户关闭了设置开关（默认开）；
// - 备用屏（vim/top/less 绝对定位重绘）；
// - 行内出现 ESC（提示符 OSC 标记、systemd/ls 自带的颜色、TUI 画面）——
//   保留程序自己的颜色，不做二次上色；
// - 单块超过 64 KiB 或每秒解码预算超标（防止 `cat` 大文件/`yes` 这类吞吐拖慢）；
// - 没有完整行（保留输入回显、进度条、无换行的提示符的即时性）；
// - 前缀不是合法 UTF-8（串口/二进制流），或跨块落在多字节字符中间（头部字节
//   原样保留）。

import type { LogTokenKind } from "../../../../shared/lib/logTokens";
import { tokenizeLogLine } from "../../../../shared/lib/logTokens";

/** 单块解码上限：更大的输出通常是文件内容而非日志。 */
export const OUTPUT_COLOR_MAX_CHUNK_BYTES = 64 * 1024;
/** 每秒解码预算：超出后本窗口内原样透传（下一窗口恢复）。 */
export const OUTPUT_COLOR_BYTES_PER_SECOND = 1_000_000;

/** token → SGR 参数。级别加粗，URL 下划线；与终端 16 色方案联动。 */
const TOKEN_SGR: Partial<Record<LogTokenKind, string>> = {
  time: "2",
  stack: "2",
  key: "94",
  string: "33",
  number: "32",
  bool: "35",
  ip: "36",
  url: "4;94",
  "level-error": "1;91",
  "level-warn": "1;33",
  "level-info": "94",
  "level-debug": "35",
  "level-trace": "2",
};

const SGR_RESET = "\x1b[0m";

const LOG_LEVEL_WORD = /\b(?:TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|ERR|CRITICAL|CRIT|FATAL|PANIC|ALERT|EMERG(?:ENCY)?|SEVERE)\b/;
const LOGFMT_PAIR = /(?:^|\s)[A-Za-z_][\w.-]*=\S/;
const KV_LEVEL = /\b(?:level|severity|lvl|levelname|level_name)["']?\s*[=:]\s*["']?(?:trace|debug|info|notice|warn|warning|error|err|critical|fatal|panic)\b/i;
const ISO_OR_CLOCK_START = /^\s*\[?(?:\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}|\d{2}:\d{2}:\d{2}\b)/;
const SYSLOG_START = /^\s*\[?(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}\b/i;
const SYSTEMD_KEY = /^\s*[A-Z][\w .-]{0,24}:\s/;
const TREE_PREFIX = /^\s*[├└│─]/;
const JSON_START = /^\s*["{[]/;
const JSON_KEY = /"\s*:/;
const URL = /\bhttps?:\/\/\S/i;

/** 结构化行判定：只有能明显看出「这是日志/结构化输出」的行才上色，正文与
 * 编译输出、`ls -l`、`git status` 等保持原样。 */
export function isStructuredLine(line: string): boolean {
  if (!line) return false;
  return (
    JSON_START.test(line) ||
    JSON_KEY.test(line) ||
    KV_LEVEL.test(line) ||
    LOG_LEVEL_WORD.test(line) ||
    LOGFMT_PAIR.test(line) ||
    ISO_OR_CLOCK_START.test(line) ||
    SYSLOG_START.test(line) ||
    SYSTEMD_KEY.test(line) ||
    TREE_PREFIX.test(line) ||
    URL.test(line)
  );
}

/** 单行 → 带 SGR 的文本（未命中结构化行、已带转义序列、或没有可着色 token 时
 * 原样返回）。行内 ESC 说明该行由程序自己控制显示，不做二次上色。 */
export function colorizeLine(line: string): string {
  if (!line || line.includes("\x1b")) return line;
  if (!isStructuredLine(line)) return line;
  const carriageReturn = line.endsWith("\r") ? "\r" : "";
  const body = carriageReturn ? line.slice(0, -1) : line;
  const { tokens } = tokenizeLogLine(body);
  let colored = false;
  let out = "";
  for (const token of tokens) {
    const sgr = TOKEN_SGR[token.kind];
    if (!sgr) {
      out += token.text;
      continue;
    }
    colored = true;
    out += `\x1b[${sgr}m${token.text}${SGR_RESET}`;
  }
  return colored ? out + carriageReturn : line;
}

/** 多行文本（已含换行）→ 逐行着色；没有完整行时保持不变。 */
export function colorizeText(text: string): string {
  if (!text.includes("\n")) return text;
  let out = "";
  let cursor = 0;
  while (cursor < text.length) {
    const newline = text.indexOf("\n", cursor);
    if (newline === -1) {
      out += text.slice(cursor);
      break;
    }
    out += colorizeLine(text.slice(cursor, newline));
    out += "\n";
    cursor = newline + 1;
  }
  return out;
}

// —— 全局开关（设置页写入；三处终端读取）——————————————
let colorizationEnabled = true;

export function setOutputColorizationEnabled(value: boolean): void {
  colorizationEnabled = value;
}

export function isOutputColorizationEnabled(): boolean {
  return colorizationEnabled;
}

export interface TerminalOutputColorizerOptions {
  /** 备用屏时透传（各渲染器提供自己的终端实例）。 */
  isAlternateScreen: () => boolean;
  /** 时钟（测试注入；生产用 Date.now）。 */
  now?: () => number;
}

export interface TerminalOutputColorizer {
  /** 返回可写入 xterm 的字节；无改动时原样返回同一引用。 */
  transform(data: Uint8Array): Uint8Array;
  /** 速率窗口复位（终端重建/会话切换）。 */
  reset(): void;
}

const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();

const NEWLINE = 0x0a;

function isContinuationByte(value: number): boolean {
  return (value & 0xc0) === 0x80;
}

export function createTerminalOutputColorizer(
  options: TerminalOutputColorizerOptions,
): TerminalOutputColorizer {
  const now = options.now ?? Date.now;
  let windowStartedAt = 0;
  let windowBytes = 0;

  function transform(data: Uint8Array): Uint8Array {
    if (!colorizationEnabled || data.byteLength === 0) return data;
    if (options.isAlternateScreen()) return data;
    if (data.byteLength > OUTPUT_COLOR_MAX_CHUNK_BYTES) return data;

    const timestamp = now();
    if (timestamp - windowStartedAt >= 1000) {
      windowStartedAt = timestamp;
      windowBytes = 0;
    }
    if (windowBytes + data.byteLength > OUTPUT_COLOR_BYTES_PER_SECOND) return data;

    const lastNewline = data.lastIndexOf(NEWLINE);
    if (lastNewline < 0) return data; // 半行/回显/进度：保持即时
    windowBytes += data.byteLength;

    // 上一块把多字节字符截断时，本块头部是续字节：先原样保留，避免解码替换。
    let start = 0;
    while (start < lastNewline && isContinuationByte(data[start])) start += 1;

    let text: string;
    try {
      text = decoder.decode(data.subarray(start, lastNewline + 1));
    } catch {
      return data; // 非法 UTF-8（串口/二进制）：原样透传
    }
    const colored = colorizeText(text);
    if (colored === text) return data;

    const tailStart = lastNewline + 1;
    const encoded = encoder.encode(colored);
    const out = new Uint8Array(start + encoded.length + (data.length - tailStart));
    out.set(data.subarray(0, start), 0);
    out.set(encoded, start);
    out.set(data.subarray(tailStart), start + encoded.length);
    return out;
  }

  return {
    transform,
    reset() {
      windowStartedAt = 0;
      windowBytes = 0;
    },
  };
}
