// 编辑器语言识别：把「任意格式」的文件映射到 CodeMirror 语言包，识别不到
// 结构语言的日志按日志着色（log: true），其余按纯文本。
//
// language-data 只带扩展名与少量 filename 正则（Dockerfile / nginx.conf /
// Gemfile…），大量无扩展名或约定名的文件（.bashrc、.env、systemd unit、shebang
// 脚本、无后缀配置）会被当成纯文本。这里按下面顺序补齐，命中即停：
//   1. 日志文件名（*.log / syslog / access_log…）
//   2. language-data 的 filename + 扩展名
//   3. 约定文件名（dotfiles、unit、Ruby/Groovy 构建脚本…）
//   4. 插件补充的扩展名别名（.conf/.cfg/.env/.plist/.jsonc/.fish…）
//   5. shebang（#!/usr/bin/env python3）
//   6. 内容嗅探（JSON / XML / HTML / diff / YAML / INI / SQL / 日志）
//
// 纯函数（语言列表由调用方注入），文件名大小写不敏感。
//
// 结果里的 `override` 表示「用插件自带实现」：Shell（legacy 模式把 `${x#y}`、
// `$#`、URL 里的 `#` 都当注释）、Nginx（legacy 把 `modules/*.conf` 的 `/*` 当
// C 注释并吞掉后续全文）、Tcl（legacy 把引号/运算符当注释）分别由 shellMode /
// nginxMode / tclMode 接管；Makefile 在 language-data 里没有语言包，由
// makefileMode 提供。装配见 languageSupport.ts。

import type { LanguageDescription } from "@codemirror/language";
import { looksLikeLog } from "../../../../shared/lib/logTokens";

export interface DetectedEditorLanguage {
  /** 命中的语言包；未命中时 undefined（按纯文本渲染）。 */
  language?: LanguageDescription;
  /** 日志串：由调用方追加日志着色扩展。 */
  log: boolean;
  /** 插件自带语言实现（语言目录缺失或 legacy 模式有缺陷时接管）。 */
  override?: EditorLanguageOverride;
}

/** 插件自带实现的语言：修 legacy 模式的注释误判 / 补语言目录缺失。 */
export type EditorLanguageOverride = "shell" | "makefile" | "nginx" | "tcl";

/** 语言目录里能命中、但要用插件自带实现的条目。 */
const OVERRIDDEN_LANGUAGES: Record<string, EditorLanguageOverride> = {
  Shell: "shell",
  // legacy nginx 把 `modules/*.conf` 的 `/*` 当 C 注释，整篇后续都变注释灰。
  Nginx: "nginx",
  // legacy tcl 把双引号与运算符返回成 comment。
  Tcl: "tcl",
};

/** `*.log` / `*.log.1` / syslog、access_log 这类约定日志名。 */
const LOG_FILE_RE = /^(?:syslog|messages|(?:access|error|daemon|auth|kern|boot|yum|dpkg|mysql|slow|secure)\.?log|(?:access|error)_log|nohup\.out|catalina\.out|journalctl)$|\.log(?:\.\d+)?$|\.(?:trace|access)$/i;

/** 约定文件名 → language-data 语言名。 */
const SPECIAL_FILES: Array<[RegExp, string]> = [
  [/^dockerfile(?:[-_.][\w.-]+)?$/i, "Dockerfile"],
  [/^containerfile(?:[-_.][\w.-]+)?$/i, "Dockerfile"],
  [/^docker-compose\.ya?ml$/i, "YAML"],
  [/^\.(?:bashrc|bash_profile|bash_login|bash_logout|bash_aliases|profile|zshrc|zprofile|zshenv|zlogin|zlogout|kshrc|shrc|aliases|inputrc|shellrc|bash_functions)$/i, "Shell"],
  [/^\.env(?:\.[\w.-]+)?$/i, "Shell"],
  [/^(?:Gemfile|Rakefile|Vagrantfile|Guardfile|Podfile|Brewfile|Fastfile|Capfile|Thorfile|Berksfile|Puppetfile)(?:\.[\w.-]+)?$/i, "Ruby"],
  [/^Jenkinsfile(?:\.[\w.-]+)?$/i, "Groovy"],
  [/^\.(?:gitconfig|gitmodules|npmrc|yarnrc|editorconfig|pylintrc|flake8|coveragerc|pypirc|netrc|curlrc|wgetrc|inputrc)$/i, "Properties files"],
  [/^(?:hosts|fstab|crontab|resolv\.conf|ssh_config|sshd_config|sudoers|os-release|issue|machine-id)$/i, "Properties files"],
  [/\.(?:service|socket|timer|target|mount|automount|path|slice|scope|swap|device|nspawn|slice)$/i, "Properties files"],
];

/** 语言目录里过于宽泛的后缀覆盖（`.cfg` 在 language-data 里是 TTCN 专用，
 * 实际见到的绝大多数是普通 key/value 配置）。 */
const EXTENSION_OVERRIDES: Record<string, string> = {
  cfg: "Properties files",
};

/** 插件补充的扩展名别名（language-data 未收录的常见后缀）。 */
const EXTENSION_LANGUAGES: Record<string, string> = {
  conf: "Properties files",
  cnf: "Properties files",
  repo: "Properties files",
  desktop: "Properties files",
  env: "Shell",
  sh: "Shell",
  bash: "Shell",
  zsh: "Shell",
  ksh: "Shell",
  fish: "Shell",
  xaml: "XML",
  plist: "XML",
  csproj: "XML",
  vcxproj: "XML",
  props: "XML",
  resx: "XML",
  wsdl: "XML",
  jsonc: "JSON",
  json5: "JSON",
  ndjson: "JSON",
  geojson: "JSON",
  ps1: "PowerShell",
  psm1: "PowerShell",
  psd1: "PowerShell",
  dockerfile: "Dockerfile",
  http: "HTTP",
  vm: "Velocity",
  vtl: "Velocity",
  tcl: "Tcl",
  tk: "Tcl",
};

const SHEBANG_LANGUAGES: Record<string, string> = {
  python: "Python",
  node: "JavaScript",
  nodejs: "JavaScript",
  deno: "JavaScript",
  bun: "JavaScript",
  bash: "Shell",
  sh: "Shell",
  zsh: "Shell",
  dash: "Shell",
  ksh: "Shell",
  fish: "Shell",
  perl: "Perl",
  ruby: "Ruby",
  php: "PHP",
  lua: "Lua",
  tclsh: "Tcl",
  wish: "Tcl",
  expect: "Tcl",
  rscript: "R",
  groovy: "Groovy",
  scala: "Scala",
  pwsh: "PowerShell",
  powershell: "PowerShell",
  swift: "Swift",
};

const NGINX_BLOCK_RE = /^\s*(?:http|server|location|upstream|events|map|stream)\b[^{}\n]*\{/m;
/** nginx 独有指令（`以分号结尾`）：只有指令、没有块的片段文件（如被 include 的
 *  `ssl.conf`、`proxy_params.conf`）也按 nginx 着色。 */
const NGINX_DIRECTIVE_RE = /^\s*(?:listen|server_name|proxy_pass|proxy_set_header|proxy_redirect|fastcgi_pass|fastcgi_param|ssl_certificate(?:_key)?|ssl_protocols|ssl_ciphers|worker_processes|worker_connections|client_max_body_size|add_header|try_files|rewrite|access_log|error_log|log_format|limit_req_zone|upstream)\b[^\n;]*;/m;

function looksLikeNginx(text: string): boolean {
  return NGINX_BLOCK_RE.test(text) || NGINX_DIRECTIVE_RE.test(text);
}

/** 文件名（basename）→ 语言名，供内容嗅探与特殊名共用。 */
function findLanguage(languages: readonly LanguageDescription[], name: string): LanguageDescription | undefined {
  const target = name.toLowerCase();
  return languages.find(
    (language) =>
      language.name.toLowerCase() === target ||
      (language.alias ?? []).some((alias) => alias.toLowerCase() === target),
  );
}

/** 命中语言 → 检测结果（附上需要插件自带实现的 override）。 */
function detectedLanguage(language: LanguageDescription): DetectedEditorLanguage {
  const override = OVERRIDDEN_LANGUAGES[language.name];
  return override ? { language, log: false, override } : { language, log: false };
}

/** 语言名 → 检测结果。Shell 的自带实现不依赖语言目录是否收录该条目。 */
function detectedByName(
  languages: readonly LanguageDescription[],
  name: string,
): DetectedEditorLanguage | undefined {
  const language = findLanguage(languages, name);
  if (language) return detectedLanguage(language);
  const override = OVERRIDDEN_LANGUAGES[name];
  return override ? { log: false, override } : undefined;
}

export function extensionOf(fileName: string): string {
  const base = fileName.split(/[\\/]/).pop() ?? fileName;
  const dot = base.lastIndexOf(".");
  // dotfile（.bashrc）不算扩展名。
  if (dot <= 0) return "";
  return base.slice(dot + 1).toLowerCase();
}

function basenameOf(fileName: string): string {
  return fileName.split(/[\\/]/).pop() ?? fileName;
}

/** Makefile 约定名与扩展名（language-data 没有对应语言，走插件自带模式）。 */
const MAKEFILE_NAME_RE = /^(?:gnu)?makefile(?:\.(?:am|in|inc|mk|mak|win|dist))?$/i;
const MAKEFILE_EXTENSIONS = new Set(["mk", "mak"]);

export function isMakefileName(fileName: string): boolean {
  const base = basenameOf(fileName);
  return MAKEFILE_NAME_RE.test(base) || MAKEFILE_EXTENSIONS.has(extensionOf(base));
}

export function isLogFileName(fileName: string): boolean {
  return LOG_FILE_RE.test(basenameOf(fileName));
}

/** 首行 shebang → 语言名（`env python3` / `python3.11` 归一化）。 */
export function shebangLanguage(text: string): string | undefined {
  if (!text.startsWith("#!")) return undefined;
  const newline = text.indexOf("\n");
  const line = text.slice(2, newline === -1 ? undefined : newline).trim();
  const parts = line.split(/\s+/);
  let executable = (parts[0] ?? "").split("/").pop() ?? "";
  if (executable === "env") executable = parts[1] ?? "";
  executable = executable.replace(/[\d.]+$/, "").toLowerCase();
  return SHEBANG_LANGUAGES[executable];
}

interface SniffResult {
  language?: string;
  log?: boolean;
}

/** 内容嗅探：只在前几步都没命中时调用，样本取前 32KB。 */
export function sniffContent(text: string): SniffResult {
  const sample = text.slice(0, 32_768);
  const trimmed = sample.trimStart();
  if (!trimmed) return {};

  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    // JSON-lines 不是合法 JSON，parse 失败后继续往下嗅探日志。
    if (text.length <= 1_000_000) {
      try {
        JSON.parse(text);
        return { language: "JSON" };
      } catch {
        /* fall through */
      }
    }
  }
  if (/^<\?xml\b/i.test(trimmed)) return { language: "XML" };
  if (/^<!doctype html\b/i.test(trimmed) || /^<html[\s>]/i.test(trimmed)) return { language: "HTML" };
  if (/^<(?:svg|plist|project|rss|feed|xsd|xsl)\b/i.test(trimmed)) return { language: "XML" };
  if (/^(?:diff --git |--- a\/|\+\+\+ b\/)/.test(trimmed)) return { language: "diff" };

  const lines = sample.split("\n").slice(0, 80).filter((line) => line.trim().length > 0);
  const yamlKeys = lines.filter((line) => /^\s*["']?[\w./-]+["']?\s*:\s+\S/.test(line)).length;
  const iniSections = lines.some((line) => /^\s*\[[^\]\n]+\]\s*$/.test(line));
  const iniPairs = lines.filter((line) => /^\s*[\w.-]+\s*=/.test(line)).length;
  if (iniSections && iniPairs >= 1) return { language: "Properties files" };
  if (yamlKeys >= 3 && yamlKeys / lines.length >= 0.5 && !/^\s*\{/.test(trimmed)) {
    return { language: "YAML" };
  }
  if (iniPairs >= 3 && iniPairs / lines.length >= 0.5) return { language: "Properties files" };
  if (/^\s*(?:SELECT\b|INSERT\s+INTO\b|UPDATE\b|DELETE\s+FROM\b|CREATE\s+(?:TABLE|DATABASE|INDEX)\b|ALTER\s+TABLE\b|WITH\b)/i.test(trimmed) && /\b(?:FROM|WHERE|VALUES|SET|TABLE)\b/i.test(sample)) {
    return { language: "SQL" };
  }
  if (looksLikeLog(text)) return { log: true };
  return {};
}

/**
 * 主入口。顺序见文件头；`text` 只用于 shebang/内容嗅探，纯文本大文件可传
 * 空串（此时只看文件名）。
 */
export function detectEditorLanguage(
  fileName: string,
  text: string,
  languages: readonly LanguageDescription[],
): DetectedEditorLanguage {
  const base = basenameOf(fileName);
  if (!base) return { log: false };
  if (isLogFileName(base)) return { log: true };
  if (isMakefileName(base)) return { log: false, override: "makefile" };

  const overrideName = EXTENSION_OVERRIDES[extensionOf(base)];
  if (overrideName) {
    const detected = detectedByName(languages, overrideName);
    if (detected) return detected;
  }

  const byData = catalogLanguage(languages, base);
  if (byData) return detectedLanguage(byData);

  for (const [pattern, name] of SPECIAL_FILES) {
    if (!pattern.test(base)) continue;
    const detected = detectedByName(languages, name);
    if (detected) return detected;
  }

  const extension = extensionOf(base);
  if (extension === "log") return { log: true };
  const extensionName = EXTENSION_LANGUAGES[extension];
  if (extensionName) {
    if (extension === "conf" || extension === "cfg" || extension === "cnf") {
      // 通用 *.conf：先看 nginx 结构块，否则按 key/value 配置处理。
      if (looksLikeNginx(text)) {
        const nginx = detectedByName(languages, "Nginx");
        if (nginx) return nginx;
      }
    }
    const detected = detectedByName(languages, extensionName);
    if (detected) return detected;
  }

  const shebang = shebangLanguage(text);
  if (shebang) {
    const detected = detectedByName(languages, shebang);
    if (detected) return detected;
  }

  const sniffed = sniffContent(text);
  if (sniffed.language) {
    const detected = detectedByName(languages, sniffed.language);
    if (detected) return detected;
  }
  return { log: sniffed.log === true };
}

/** language-data 自带的 filename 正则与扩展名匹配（filename 优先于扩展名）。 */
function catalogLanguage(
  languages: readonly LanguageDescription[],
  base: string,
): LanguageDescription | undefined {
  for (const language of languages) {
    if (language.filename && language.filename.test(base)) return language;
  }
  const extension = extensionOf(base);
  if (!extension) return undefined;
  return languages.find((language) =>
    language.extensions.some((candidate) => candidate.toLowerCase() === extension),
  );
}
