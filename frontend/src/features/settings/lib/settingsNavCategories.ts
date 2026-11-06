// 设置导航的分类表与搜索关键词表（纯数据 + 纯函数，无 Vue 依赖）。
//
// 分类顺序对齐 Tabby 的设置页优先级：外观 / 配色方案 / 终端 / 快捷键 四个终端
// 相关分类排在最前（Tabby 把 Appearance 与 Color scheme 标为 prioritized），
// 之后才是本插件特有的 sudo / 智能体 / 传输 / 安全 / MCP。
export const SETTINGS_CATEGORIES = [
  { id: "appearance", labelKey: "settingsNav.appearance" },
  { id: "scheme", labelKey: "settingsNav.scheme" },
  { id: "terminal", labelKey: "settingsNav.terminal" },
  // 从 Terminal 分栏拆出（#138 反馈：建议/补全/ghost 配置挤在终端页难找）：
  // 历史建议 + 结构化补全引擎 + 行内 ghost + 忽略列表 + 接受键换绑集中在此。
  { id: "suggestions", labelKey: "settingsNav.suggestions" },
  // 从 Terminal 分栏拆出（内容过大且与终端行为关联性弱）：高亮规则是规则
  // 管理器、快速命令是片段管理器，各自独立成栏，导航标签复用现有键。
  { id: "highlight", labelKey: "highlightRules.title" },
  { id: "commands", labelKey: "quickCommands" },
  // AI 助手（Warp AI 对齐批）：# 命令搜索 / 失败修复 / 唤起助手三开关 +
  // 能力缺失横幅 + 「不再询问」重置；权威态在 App（pluginStore 单键 JSON）。
  { id: "ai", labelKey: "settingsNav.ai" },
  { id: "hotkeys", labelKey: "settingsNav.hotkeys" },
  { id: "sudo", labelKey: "settingsNav.sudo" },
  { id: "agent", labelKey: "agentTerminalSection" },
  { id: "transfer", labelKey: "downloadSettings.title" },
  { id: "security", labelKey: "settingsNav.security" },
  { id: "mcp", labelKey: "mcpLimits.title" },
] as const;

export type SettingsCategory = (typeof SETTINGS_CATEGORIES)[number]["id"];

/** 会话级分类：这二者只对 SSH 会话有意义，本地终端下隐藏。 */
export const SESSION_CATEGORIES: readonly SettingsCategory[] = ["sudo", "agent"];

// 导航搜索只匹配分类名并不够：用户搜的是设置项（"font"、"字号"、"webgl"），
// 而这些词几乎从不出现在分类名里，所以每个分类补一份关键词表（中英 + 常见
// 同义词/字段名），分类名与关键词命中其一即可。
export const CATEGORY_KEYWORDS: Record<SettingsCategory, readonly string[]> = {
  appearance: [
    "appearance", "font", "font family", "font size", "line height", "letter spacing", "weight",
    "cursor", "cursor blink", "blinking cursor", "padding", "typography", "preview", "wallpaper",
    "background image", "ansi", "contrast", "bold",
    "外观", "字体", "字号", "行高", "字距", "光标", "闪烁", "内边距", "壁纸", "背景图", "预览",
  ],
  scheme: [
    "scheme", "color scheme", "theme", "palette", "dark", "light", "colors", "ansi colors",
    "import", "export", "solarized",
    "配色", "配色方案", "主题", "深浅", "颜色", "导入", "导出",
  ],
  terminal: [
    "terminal", "behavior", "rendering", "webgl", "gpu", "smooth scroll", "scrollback",
    "bell", "audible", "layout", "pane", "split", "direction", "local shell", "startup",
    "startup command", "record", "recording", "auto record", "x11", "rdp", "terminal behavior",
    "终端", "行为", "渲染", "滚动", "回滚", "响铃", "分栏", "布局", "本地 shell", "启动",
    "录制", "自动录制",
  ],
  suggestions: [
    "suggestions", "completion", "ghost", "inline", "history", "accept key", "ignore",
    "min chars", "max chars", "autocomplete",
    "建议", "补全", "历史", "接受键", "忽略",
  ],
  highlight: [
    "highlight", "highlighting", "rules", "regex", "pattern", "display mode", "keyword",
    "高亮", "高亮规则", "正则", "显示模式", "关键字",
  ],
  commands: [
    "quick commands", "snippets", "snippet", "groups", "paste", "run", "import", "limit",
    "快速命令", "片段", "命令组", "粘贴", "运行", "导入",
  ],
  ai: [
    "ai", "assistant", "agent mode", "fix", "search", "consent", "model",
    "智能", "助手", "智能体", "修复", "搜索", "授权",
  ],
  hotkeys: [
    "hotkeys", "shortcuts", "keybindings", "bindings", "keys", "conflict", "reset",
    "快捷键", "绑定", "按键", "冲突", "复位",
  ],
  sudo: [
    "sudo", "quick sudo", "profiles", "credential", "password", "pty", "totp",
    "提权", "凭据", "密码", "配置",
  ],
  agent: [
    "agent", "terminal sync", "prompt", "hint", "mode", "sync",
    "智能体", "终端同步", "提示",
  ],
  transfer: [
    "transfer", "download", "upload", "directory", "concurrency", "duplicate", "conflict",
    "encoding", "compression", "limit",
    "传输", "下载", "上传", "目录", "并发", "重名", "编码", "压缩", "限速",
  ],
  security: [
    "security", "known hosts", "host key", "keys", "ssh keys", "fingerprint", "remove",
    "安全", "已知主机", "主机密钥", "密钥", "指纹", "移除",
  ],
  mcp: [
    "mcp", "limits", "read bytes", "upload bytes", "download bytes", "exec permission",
    "scope",
    "限制", "大小", "读写", "权限", "范围",
  ],
};

/** 分类命中：已本地化的分类名或关键词表里任一命中即算。query 需已 trim + lowercase。 */
export function categoryMatchesQuery(
  categoryId: SettingsCategory,
  label: string,
  query: string,
): boolean {
  if (label.toLowerCase().includes(query)) {
    return true;
  }
  return CATEGORY_KEYWORDS[categoryId].some((keyword) => keyword.includes(query));
}
