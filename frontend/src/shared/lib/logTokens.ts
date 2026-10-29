// 日志/终端输出的着色词法：把一行日志切成带语义的片段（日期时间、级别、
// 键值、字符串、数字、IP、URL、堆栈帧），外加 ANSI SGR 颜色解析。
//
// 两处复用（只此一份实现，避免两套正则漂移）：
// - 文件编辑器：日志文件的 CodeMirror 装饰（features/sftp/lib/editor/logHighlight.ts）
// - 容器日志页：流式日志的按行着色渲染（features/docker/components/ContainerLogsTab.vue）
//
// 纯函数、无 Vue / CodeMirror 依赖。颜色类名统一为 `log-<kind>`，样式集中在
// app/styles/global/log.css（v-html 内容不受 scoped 样式约束，故走全局表）。

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export type LogTokenKind =
  | "plain"
  | "time"
  | "key"
  | "string"
  | "number"
  | "bool"
  | "ip"
  | "url"
  | "stack"
  | `level-${LogLevel}`
  | `ansi-${string}`;

export interface LogToken {
  kind: LogTokenKind;
  text: string;
}

export interface LogLineTokens {
  /** 整行级别（用于标题/汇总；token 级别的着色见 tokens）。 */
  level: LogLevel | null;
  tokens: LogToken[];
}

/** 单行解析上限：超长行（压缩 JSON、二进制残留）按纯文本返回，避免正则回溯。 */
const MAX_LINE_CHARS = 4000;
/** 单行 token 上限：越界部分并入一个 plain token。 */
const MAX_TOKENS = 512;

const LEVEL_ALIASES: Record<string, LogLevel> = {
  trace: "trace",
  verbose: "trace",
  debug: "debug",
  dbg: "debug",
  info: "info",
  information: "info",
  notice: "info",
  warn: "warn",
  warning: "warn",
  error: "error",
  err: "error",
  critical: "error",
  crit: "error",
  fatal: "error",
  panic: "error",
  alert: "error",
  emerg: "error",
  emergency: "error",
  severe: "error",
};

const LEVEL_WORD = "trace|verbose|debug|dbg|info|notice|warn|warning|error|err|critical|crit|fatal|panic|alert|emerg(?:ency)?|severe";

const LEVEL_KV_RE = new RegExp(
  `\\b(?:level|severity|lvl|levelname|level_name)["']?\\s*[=:]\\s*["']?(${LEVEL_WORD})\\b`,
  "i",
);
const LEVEL_BRACKET_RE = new RegExp(`\\[(?:level\\s*[=:]\\s*)?(${LEVEL_WORD})\\]`, "i");
const LEVEL_START_RE = new RegExp(`^\\s*\\[?(${LEVEL_WORD})\\]?\\s*[\\]:|-]`, "i");
const LEVEL_UPPER_RE = /\b(?:TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|ERR|CRITICAL|CRIT|FATAL|PANIC|ALERT|EMERG(?:ENCY)?|SEVERE)\b/;

function levelOf(word: string): LogLevel | null {
  return LEVEL_ALIASES[word.toLowerCase()] ?? null;
}

/**
 * 整行级别识别。优先级：`level=info` 键值 > `[ERROR]` 括号 > 行首级别 >
 * 行内大写级别（LOG_LEVEL、INFO:root 这类）。刻意不做「行内任意小写级别词」
 * 匹配——英文日志正文里的 "error" 是描述，不是级别。
 */
export function detectLogLevel(line: string): LogLevel | null {
  const head = line.slice(0, 240);
  const kv = LEVEL_KV_RE.exec(head);
  if (kv) return levelOf(kv[1]);
  const bracketed = LEVEL_BRACKET_RE.exec(head);
  if (bracketed) return levelOf(bracketed[1]);
  const start = LEVEL_START_RE.exec(head);
  if (start) return levelOf(start[1]);
  const upper = LEVEL_UPPER_RE.exec(head);
  if (upper) return levelOf(upper[0]);
  return null;
}

const TIME_SOURCE = [
  // ISO 8601（可带时区）与纯日期
  "\\d{4}-\\d{2}-\\d{2}(?:[T ]\\d{2}:\\d{2}:\\d{2}(?:[.,]\\d{1,9})?(?:Z|[+-]\\d{2}:?\\d{2})?)?",
  // syslog：Oct 29 12:00:00
  "(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\\s+\\d{1,2}\\s+\\d{2}:\\d{2}:\\d{2}",
  // 裸时间/耗时：12:00:00,123
  "\\b\\d{1,2}:\\d{2}:\\d{2}(?:[.,]\\d{1,9})?\\b",
].join("|");

const TIME_TEST_RE = new RegExp(TIME_SOURCE, "i");

const SCAN_SOURCE = [
  `(?<time>${TIME_SOURCE})`,
  // 量词全部设上限：未闭合引号/超长单词在整行回溯里会退化成 O(n²)。
  `(?<str>"(?:[^"\\\\\\n]|\\\\.){0,1000}"|'(?:[^'\\\\\\n]|\\\\.){0,1000}')`,
  `(?<url>https?:\\/\\/[^\\s"'<>]{0,2048})`,
  `(?<level>\\b(?:${LEVEL_WORD})\\b)`,
  // key= / key: 的键名部分（`=`/`:` 本身留给后续匹配）
  `(?<key>[A-Za-z_][\\w.-]{0,63}(?==))`,
  `(?<ip>\\b\\d{1,3}(?:\\.\\d{1,3}){3}\\b)`,
  `(?<bool>\\b(?:true|false|null|nil|none)\\b)`,
  `(?<num>\\b\\d+(?:[.,]\\d+)?\\b)`,
].join("|");

const SCAN_RE = new RegExp(SCAN_SOURCE, "gi");

/** 堆栈帧：`    at foo (bar.js:1:2)` / `  File "x.py", line 3`。 */
const STACK_FRAME_RE = /^\s+(?:at\s|File\s+")/;
const TRACEBACK_RE = /^\s*(?:Traceback\b|Caused by:)/;

const JSON_KEY_LEVEL = new Set(["level", "severity", "lvl", "levelname", "level_name"]);

function isLevelKey(raw: string): boolean {
  return JSON_KEY_LEVEL.has(raw.replace(/^["']|["']$/g, "").toLowerCase());
}

function mergeToken(tokens: LogToken[], kind: LogTokenKind, text: string): void {
  if (!text) return;
  const last = tokens[tokens.length - 1];
  if (last && last.kind === kind) last.text += text;
  else tokens.push({ kind, text });
}

/** 语义着色（不处理 ANSI；带 ESC 的行由 tokenizeAnsiLine 负责）。 */
export function tokenizeLogLine(line: string): LogLineTokens {
  const level = detectLogLevel(line);
  if (line.length > MAX_LINE_CHARS) return { level, tokens: [{ kind: "plain", text: line }] };
  if (!line) return { level, tokens: [] };
  if (TRACEBACK_RE.test(line)) return { level: level ?? "error", tokens: [{ kind: "level-error", text: line }] };
  if (STACK_FRAME_RE.test(line)) return { level, tokens: [{ kind: "stack", text: line }] };

  const tokens: LogToken[] = [];
  let cursor = 0;
  SCAN_RE.lastIndex = 0;
  for (const match of line.matchAll(SCAN_RE)) {
    const index = match.index ?? 0;
    if (index > cursor) mergeToken(tokens, "plain", line.slice(cursor, index));
    const groups = match.groups ?? {};
    if (groups.time !== undefined) mergeToken(tokens, "time", groups.time);
    else if (groups.str !== undefined) {
      // JSON 键：字符串紧跟冒号（"level": …）→ 键着色。
      const after = line[index + match[0].length];
      mergeToken(tokens, after === ":" ? "key" : "string", groups.str);
    } else if (groups.url !== undefined) mergeToken(tokens, "url", groups.url);
    else if (groups.level !== undefined) {
      const mapped = levelOf(groups.level);
      mergeToken(tokens, mapped ? `level-${mapped}` : "plain", groups.level);
    } else if (groups.key !== undefined) mergeToken(tokens, "key", groups.key);
    else if (groups.ip !== undefined) mergeToken(tokens, "ip", groups.ip);
    else if (groups.bool !== undefined) mergeToken(tokens, "bool", groups.bool);
    else if (groups.num !== undefined) mergeToken(tokens, "number", groups.num);
    cursor = index + match[0].length;
    if (tokens.length > MAX_TOKENS) break;
  }
  if (cursor < line.length) mergeToken(tokens, "plain", line.slice(cursor));
  applyJsonLevelValue(tokens);
  return { level: level ?? lineLevelFromTokens(tokens), tokens };
}

/** `"level":"info"`：值是个字符串 token，把该 token 改成级别色。 */
function applyJsonLevelValue(tokens: LogToken[]): void {
  for (let index = 0; index < tokens.length - 1; index += 1) {
    const current = tokens[index];
    if (current.kind !== "key" || !isLevelKey(current.text)) continue;
    for (let next = index + 1; next < tokens.length; next += 1) {
      const candidate = tokens[next];
      if (candidate.kind === "plain") continue;
      if (candidate.kind !== "string") break;
      const mapped = levelOf(candidate.text.replace(/^["']|["']$/g, ""));
      if (mapped) candidate.kind = `level-${mapped}`;
      break;
    }
  }
}

function lineLevelFromTokens(tokens: LogToken[]): LogLevel | null {
  for (const token of tokens) {
    if (token.kind.startsWith("level-")) return token.kind.slice("level-".length) as LogLevel;
  }
  return null;
}

// —— ANSI SGR（容器日志里应用自带颜色的输出）——————————————————

const ANSI_NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

// 一条 ESC 序列：SGR（带捕获）优先，其余 OSC/CSI 直接剥掉。
const ANSI_RE = /\x1b\[([0-9;]*)m|\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[A-Za-z]/g;

interface AnsiStyle {
  color: string | null;
  bold: boolean;
}

function ansiKind(style: AnsiStyle): LogTokenKind {
  if (style.color) return `ansi-${style.color}`;
  return style.bold ? "ansi-bold" : "plain";
}

function ansi256Color(value: number): string | null {
  if (!Number.isFinite(value) || value < 0 || value > 255) return null;
  if (value < 8) return ANSI_NAMES[value];
  if (value < 16) return `bright-${ANSI_NAMES[value - 8]}`;
  if (value >= 232) return value >= 244 ? "white" : "bright-black";
  const index = value - 16;
  const red = Math.floor(index / 36);
  const green = Math.floor((index % 36) / 6);
  const blue = index % 6;
  const max = Math.max(red, green, blue);
  if (max === 0) return "black";
  if (red === max && green === max && blue === max) return max >= 4 ? "white" : "bright-black";
  if (red === max) return green >= 3 || blue >= 3 ? "magenta" : "red";
  if (green === max) return blue >= 3 ? "cyan" : "green";
  return "blue";
}

function applySgr(raw: string, style: AnsiStyle): void {
  const codes = raw.length === 0 ? [0] : raw.split(";").map((code) => Number.parseInt(code, 10) || 0);
  for (let index = 0; index < codes.length; index += 1) {
    const code = codes[index];
    if (code === 0) {
      style.color = null;
      style.bold = false;
    } else if (code === 1) style.bold = true;
    else if (code === 22) style.bold = false;
    else if (code >= 30 && code <= 37) style.color = ANSI_NAMES[code - 30];
    else if (code === 39) style.color = null;
    else if (code >= 90 && code <= 97) style.color = `bright-${ANSI_NAMES[code - 90]}`;
    else if (code === 38) {
      if (codes[index + 1] === 5 && codes[index + 2] !== undefined) {
        style.color = ansi256Color(codes[index + 2]);
        index += 2;
      } else if (codes[index + 1] === 2) {
        // 24 位真彩降级为默认前景：不引入内联样式，避免与主题色冲突。
        style.color = null;
        index += 4;
      }
    }
  }
}

/**
 * 带 ANSI 的行：按 SGR 状态切片并剥掉所有转义序列。没有 ESC 时返回 null，
 * 调用方回落 `tokenizeLogLine`。
 */
export function tokenizeAnsiLine(line: string): LogToken[] | null {
  if (!line.includes("\x1b")) return null;
  const tokens: LogToken[] = [];
  const style: AnsiStyle = { color: null, bold: false };
  let cursor = 0;
  ANSI_RE.lastIndex = 0;
  for (const match of line.matchAll(ANSI_RE)) {
    const index = match.index ?? 0;
    if (index > cursor) mergeToken(tokens, ansiKind(style), line.slice(cursor, index));
    cursor = index + match[0].length;
    if (match[1] !== undefined) applySgr(match[1], style);
  }
  if (cursor < line.length) mergeToken(tokens, ansiKind(style), line.slice(cursor));
  return tokens.length ? tokens : null;
}

/** 容器日志行入口：有 ANSI 走 ANSI 解析，否则走语义着色。 */
export function tokenizeOutputLine(line: string): LogToken[] {
  const ansi = tokenizeAnsiLine(line);
  if (ansi) return ansi;
  if (line.length > MAX_LINE_CHARS) return [{ kind: "plain", text: line }];
  return tokenizeLogLine(line).tokens;
}

/** 内容像日志吗（无扩展名文件的语言兜底）：一半以上行有级别词，或时间戳 + 两个级别词。 */
export function looksLikeLog(text: string): boolean {
  const sample = text.slice(0, 24000);
  const lines = sample
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .slice(0, 120);
  if (lines.length < 2) return false;
  let leveled = 0;
  let stamped = 0;
  for (const line of lines) {
    if (detectLogLevel(line)) leveled += 1;
    if (TIME_TEST_RE.test(line)) stamped += 1;
  }
  if (leveled / lines.length >= 0.5) return true;
  if (leveled >= 2 && stamped / lines.length >= 0.5) return true;
  // 无级别词的纯时间戳流（journald / 访问日志片段）：行行带时间即按日志着色。
  return stamped / lines.length >= 0.8;
}

/** 行 HTML 渲染（容器日志 v-html）：只转义文本必要字符。 */
export function escapeLogHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** 一行 → `log-<kind>` span HTML（plain 段不加 span）。 */
export function renderLogLineHtml(line: string): string {
  const tokens = tokenizeOutputLine(line);
  let html = "";
  for (const token of tokens) {
    if (token.kind === "plain") html += escapeLogHtml(token.text);
    else html += `<span class="log-${token.kind}">${escapeLogHtml(token.text)}</span>`;
  }
  return html;
}
