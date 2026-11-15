# 2026-11-15 终端录制面板设计

## Intent

把终端录制从「一个通用浮层列表」变成更像可回放磁带的工作台：每段录制有清楚的时间脊线；录制中能直接停止、刷新列表；所有动作常驻可达。用户明确要求**不录制鼠标位置和终端光标**，因此本设计只改面板视觉与交互，不新增录制事件类型，不改动既有 asciicast v2 文件目录或权限模型。

## Approaches

1. **通用 SaaS 卡片组**：把每条录制做成相同圆角卡片，加渐变和悬浮阴影。否决：与项目现有 token 体系重复，且对「时间流」没有信息增益。
2. **操作系统级屏幕录制**：额外录屏或读取全局指针坐标。否决：权限、隐私、跨平台成本都超出插件边界，且用户已明确不需要鼠标/光标录制。
3. **终端原生磁带脊线（选用）**：沿用现有 `.cast` JSONL 与输出事件，只用一条连续竖线组织记录；所有动作保持可达、可刷新、可停止。理由：与主题一致、无需新依赖、旧录制不受影响、不采集任何指针/光标元数据。

## Design plan

### Color

使用项目 token，不硬编码新主题；以下为暗色兜底 token 的命名色，用来约束对比和层级：

- Ink `#131416`：终端/工作台底色。
- Paper `#D7D7DB`：主要文字与高对比图标。
- Slate `#2A2A2D`：次级表面、静默边框。
- Focus Blue `#3B82F6`：键盘焦点、主操作、搜索命中边线。
- Signal Rose `#EF4444`：录制中状态与删除危险态。
- Aubergine `#A855F7`：RemoteX 品牌紫，仅用于「磁带脊线」的录制状态标记。

所有实际 CSS 继续用 `var(--background)`, `var(--foreground)`, `var(--popover)`, `var(--primary)`, `var(--destructive)`, `var(--card-*)`, `color-mix(...)`；亮色宿主自动跟随。`--tone-violet` 是品牌紫的现有派生 token，若主题桥未覆盖则回退到 `var(--info)` 与 `var(--destructive)` 的混合色，不写死紫值。

### Type

- UI：`Inter, ui-sans-serif, system-ui, -apple-system, ...`，沿用 `var(--ui-font-family)`；标题 15px/600，行标题 12px/600，元信息 11px/400；行高 1.25–1.5，正文行宽不超过 80 字符。
- 时长、时间戳、文件大小：`JetBrains Mono, Cascadia Mono, Consolas, monospace`，沿用 `var(--terminal-font-family)` 与 `font-variant-numeric: tabular-nums`。
- 不新增字体、不把标题全大写、不加无意义的 eyebrow 标签、不把单个词换色强调。

### Layout

桌面：浮动面板宽 `min(520px, calc(100% - 16px))`，左对齐内容。头部只有一条明确的录制状态带；搜索和统计在第二行；记录列表以左侧连续竖线（tape spine）串起，每条在时间线节点上给出主机、时间、时长和操作。次要动作保持常驻可见，不再靠 hover 才显示。

```text
┌──────────────────────────────────────────────────────────┐
│ ● Terminal recording        [Refresh] [Stop] [×]          │
│ 3 recordings · Enter opens replay · Esc closes            │
│ [ Search recordings                              ] [×]   │
├──────────────────────────────────────────────────────────┤
│ │  server.demo.internal                  00:24  [▶] [⋯] │
│ │  Nov 15 14:02 · 4.0 KiB                              │
│ │  ╭ health check passed (200 OK)                       │
│ │                                                      │
│ │  db-02.internal                        01:03  [▶] [⋯] │
│ │  Nov 15 11:40 · 18.2 KiB                             │
├──────────────────────────────────────────────────────────┤
│ Enter opens replay · Esc closes · Start recording          │
└──────────────────────────────────────────────────────────┘
```

窄屏（≤480px / dock 面板）：宽度 `calc(100% - 16px)`；时长移到行标题右侧下方；五枚行内动作收进「更多」菜单或保持 28px 的最小可点区域并允许横向换行；头部刷新/停止/关闭按钮保持 28px；搜索框占满剩余宽度。回放弹窗宽 `min(920px, 92vw)`，控制条换行但进度条仍占满一行。

### Principles

1. **时间流优先**：连续竖线是唯一「大胆」元素，替代重复的卡片边框和渐变；它表达录制确实是一段时间序列。
2. **动作可预期**：开始/停止/刷新/清空/删除都使用现有动作名；按钮说什么，提示就说什么。
3. **只录终端输出**：录制内容仅包含 asciicast `o` 输出事件与结束标记，不采集鼠标、光标、终端文本之外的按键或剪贴板。
4. **静默降级**：旧录制没有新字段时回放照常播放；录制中会话消失时前端计时器同步停止。
5. **可达性不是装饰**：键盘焦点环、Esc、Enter 打开回放、可见标签、28px 点击区域、`prefers-reduced-motion` 全部作为质量下限。

## Review against generic defaults

- 没有 near-black + acid-green、cream + terracotta、broadsheet 边框、每张相同 SaaS 卡、全大写 eyebrow、渐变装饰、箭头后缀等模板信号。
- 竖线/节点来自「录制是时间流」这一具体内容，不是通用编号装饰；不使用 `01 / 02 / 03` 序号。
- 品牌紫只出现在录制状态节点，与 RemoteX 紫色图标呼应；其余沿用宿主 token，不另造一套主题。
- 动效只用于录制中的脉冲点和用户触发的展开/停止反馈；全局 `prefers-reduced-motion` 已统一关闭装饰动画。
