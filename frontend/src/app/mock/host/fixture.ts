/**
 * 可视化夹具的启动参数与环境镜像：URL 开关、宿主 context、appearance/theme、
 * locale 初始值与 ?noanim 样式注入。
 *
 * 语义与拆分前 mockDbxHost.ts 顶部逐字一致：这些副作用在模块求值时就发生
 * （pluginStore 种子 → context/appearance/theme → <html> 令牌与 data 属性）。
 */

import { pluginStore } from "../../../shared/lib/pluginStore";
import { emitHostEvent } from "./shared";

export const fixtureParams = new URLSearchParams(location.search);
// ?render=dom 强制关闭终端 WebGL 加速（pluginStore 偏好，见 lib/pluginStore.ts）：
// mock walkthrough 的终端断言读 DOM 文本，WebGL 渲染下文本只存在于 GPU canvas，
// 必须锁定 DOM 渲染器路径（WebGL 自身的成功/回退由 terminalWebgl 单测覆盖）。
// 种子必须走 pluginStore（写缓存 + 写穿持久档）：App 经 store 首读，直写
// localStorage 会被水合时序吃掉（宿主档水合发生在种子写入之后也读不到）。
if (fixtureParams.get("render") === "dom") {
  // 快速输入诊断（#33/#71）：默认强制 DOM 渲染器。要复刻真实工作台的
  // WebGL 渲染路径时，把下行改为 pluginStore.setItem("ssh-terminal-webgl", "1")
  //（注意：Safari 的 vite fixture 下 WebGL 终端渲染为空白，仅 Chromium 可用）。
  pluginStore.setItem("ssh-terminal-webgl", "0");
}
// ?rw=1 模拟可写连接（默认只读），供拖放上传等写路径 UI 验证。
export const writable = fixtureParams.get("rw") === "1";
// ?err=disconnect 在会话建立 4s 后模拟一次传输断开（ssh/session/state
// disconnected，单次不复发），供重连横幅/倒计时/立即重连/恢复提示的全流程 UI 验证。
// 断开定时器同时挂在 open 与 attach 两条启动路径上：默认启动走
// sessions/list → reattach，只挂 open 会让该参数在首屏完全失效（P2-1）。
export const disconnectAfterMs = fixtureParams.get("err") === "disconnect" ? 4000 : 0;
// ?err=authfail 让 ssh/session/open 抛出真实 sidecar 风格的认证失败错误串，
// 供连接失败错误提示友好化（connectError.*）的浏览器 UI 验证。
export const failSessionOpen = fixtureParams.get("err") === "authfail";
// ?slow=N 把 mock ssh/session/open 延迟 N 秒才返回，供连接中卡片（旋转弧 /
// 流光动画、Cancel、Show logs 面板）的浏览器视觉验证（模拟慢拨号）。
export const slowSessionOpenMs = Math.max(0, Number(fixtureParams.get("slow")) || 0) * 1000;
// ?fresh=1 让 sessions/list 返回空——首屏不走 reattach 而走 ssh/session/open，
// 供连接成功过渡动画（success 卡片）等 open 路径的浏览器视觉验证。
export const freshSessionOpen = fixtureParams.get("fresh") === "1";
// 与 DBX globals.css 的 :root（pearl 浅色）和 .dark 规范块保持一致。
const light = fixtureParams.get("theme") === "light";

// ?noanim=1 关闭全部 CSS 动画/过渡：截图型可视 e2e 在动画被节流的宿主里会
// 拍到停在半透明入场帧的弹层（reka fade-in 冻结在 opacity 0，弹层呈"鬼影"），
// 该参数锁定稳定终态帧，让逐弹层截图可复现。
if (fixtureParams.get("noanim") === "1") {
  const style = document.createElement("style");
  style.textContent = "*,*::before,*::after{animation:none!important;transition:none!important}";
  document.head.appendChild(style);
}

// ?local=1 simulates a host opening a connectionless local-terminal tab via the command context (plugin.mode="local-terminal");
// Connectionless local-terminal tab (HOST_PLUGIN_UI_SPEC §4/§7.1 passthrough): no
// connectionId/connection. workbenchId/restored/surface are injected by the host (mock).
export const localOnlyContext = fixtureParams.get("local") === "1";
// ?proto=rdp 模拟宿主连接表单 protocol=rdp：RDP 连接级门控走查（工具条 SSH-only
// 控件整类隐藏）。mock 的 rdp/start 夹具（见 mock/host/rdp.ts）会推桌面帧，可直连完成。
export const protoFixture = fixtureParams.get("proto");
// ?local=1&restored=1 simulates a host-restored local-terminal tab: restore semantics (spec §7.6/§8.4)
// requires restore-without-replay — no automatic shell, just the exit shell until the user explicitly starts one.
export const restoredFixture = fixtureParams.get("restored") === "1";
// ?ai=off 模拟旧宿主（无 AI 面板桥，capabilities.ai 缺省）：AI 助手全链路
// 降级走查（# 放行为 shell 注释、热键给可见提示）；缺省 ai 在位——
// host.ai.openConversation 会话请求记录进 __dbxMockAiConversations 供走查断言。
export const aiPanelOn = fixtureParams.get("ai") !== "off";
// ?aipanel=1 模拟 web 运行时（openConversation 在、直连生成/推荐位缺）：
// AI 修复走面板回退路径的走查入口。
export const aiCompletionOn = aiPanelOn && fixtureParams.get("aipanel") !== "1";
// ?aifix=1：ssh/terminal 回显改为「E/C → 失败输出（含敏感值）→ D exit 2 →
// 提示符」的 633 全链路，驱动 AI 修复条（脱敏 + 预览确认）走查。
export const aiFixFixture = fixtureParams.get("aifix") === "1";
// ?surface=panel simulates the host opening this webview as the bottom dock panel
// (host §8.3 surface=panel)：dock 走查（紧凑工具条/配色快切）与跨表面外观同步
// 的截图/联动验证入口；缺省仍为 tab。bridge openWorkbench 只开 tab，不受影响。
export const surfaceFixture = fixtureParams.get("surface") === "panel" ? "panel" : "tab";

// ?auth=auto 模拟 Auto 认证链（M13-A，M18 mock 补齐）：ssh/session/open 按后端
// AUTO_AUTH_ORDER（password → private-key → keyboard-interactive → agent，见
// backend/src/ssh.rs）逐方式发 ssh/auth/auto 进度事件后拨号成功（agent 兜底
// 成功，成功方式不发事件——与真实 sidecar「只报非成功阶段」一致）；
// ?auth=autofail 四个方式全败并抛聚合错误串（auto_auth_failure_message 同构）。
// 缺省不发任何认证进度事件（显式方式无回退叙事）。
export const authFixture = fixtureParams.get("auth") || "";
export const autoAuthActive = authFixture === "auto" || authFixture === "autofail";

export const context = localOnlyContext
  ? {
      plugin: { mode: "local-terminal" },
      workbenchId: "visual-workbench",
      restored: restoredFixture,
      surface: surfaceFixture,
    }
  : {
  connectionId: "visual-connection",
  workbenchId: "visual-workbench",
  restored: false,
  surface: surfaceFixture,
  workbenchState: { sftpPath: "/home/demo", splitRatio: 58, paneOrder: "terminal-left", visibleColumns: ["size", "modified", "owner", "group", "permissions"] },
  // ?proto=rdp 模拟一条 RDP 连接（RDP 连接级 UI 门控走查：SSH/SFTP 专属工具
  // 条整类隐藏，连接协议由 openSession 路由到 RDP 会话）。协议形态对齐真实
  // 宿主：宿主推给 webview 的 connection 上下文【不带】external_config.protocol
  // （该字段只随 lifecycle connection/connect 走向 sidecar），前端经
  // connection/protocol 向 sidecar 查询权威协议——mock 在 invoke 分发里按
  // protoFixture 应答。此前把 protocol 塞进 external_config 的形态在真实宿主
  // 不存在，走查会漏掉「查询结果只用于当次路由」的门控缺口。
  connection: {
    name: protoFixture === "rdp" ? "Production RDP" : "Production SSH",
    host: protoFixture === "rdp" ? "rdp.demo.internal" : "192.168.1.64",
    port: protoFixture === "rdp" ? 3389 : 22,
    username: "user",
    color: "#3b82f6",
    readOnly: !writable,
    ...(autoAuthActive ? { authentication: "auto" } : {}),
  },
};
// ?tunnels=1 and ?tunnels=quick model a connection right-click workbench.
if (fixtureParams.has("tunnels")) {
  delete (context as Record<string, unknown>).connection;
  Object.assign(context, { id: "visual-connection", dbType: "ssh", name: "Production SSH" });
}

export const appearance: DbxPluginAppearance = {
  colorScheme: light ? "light" : "dark",
  colors: light
    ? { background: "rgb(255 255 255)", foreground: "rgb(10 10 10)", muted: "rgb(245 245 245)", mutedForeground: "rgb(115 115 115)", accent: "rgb(245 245 245)", accentForeground: "rgb(23 23 23)", border: "rgb(229 229 229)", destructive: "rgb(231 0 11)" }
    : { background: "rgb(19 20 22)", foreground: "rgb(215 215 219)", muted: "rgb(42 42 45)", mutedForeground: "rgb(151 152 157)", accent: "rgb(46 47 51)", accentForeground: "rgb(221 221 226)", border: "rgb(110 110 114 / 0.28)", destructive: "rgb(243 98 95)" },
  terminal: { fontFamily: "'JetBrains Mono', 'Cascadia Mono', Consolas, monospace", fontSize: 13 },
};

// 镜像宿主 1.1 theme 通道形状（colors 反查 --color-* 令牌），与真实宿主一致。
export const theme: DbxPluginTheme = {
  appearance: appearance.colorScheme,
  tokens: Object.fromEntries(
    Object.entries(appearance.colors).map(([key, value]) => [`--color-${key.replace(/([A-Z])/g, (c) => `-${c.toLowerCase()}`)}`, value]),
  ) as Record<string, string>,
};
theme.tokens["--color-primary"] = light ? "rgb(23 23 23)" : "rgb(59 130 246)";
theme.tokens["--color-primary-foreground"] = "rgb(255 255 255)";
// The real host installs these resolved tokens on <html> before loading the
// iframe. Mirror that precedence so the plugin theme bridge does not read the
// Tailwind fallback aliases back into themselves in the visual fixture.
for (const [key, value] of Object.entries(theme.tokens)) document.documentElement.style.setProperty(key, value);
document.documentElement.dataset.dbxTheme = theme.appearance;

// 初始 locale 支持 ?locale= 覆盖（镜像真实桥 api.locale）；运行时经
// __dbxMockSetLocale 切换——镜像真桥 updateLocale 的"改字段 + onEvent 投递
// {type:"env", locale}"语义（真桥无 onLocaleChange 订阅，X-P2/P5 对齐），
// 供 i18n 切换链的浏览器与单测验证。
let currentLocale = fixtureParams.get("locale") || "en";

export function getMockLocale(): string {
  return currentLocale;
}

/** 镜像真桥 updateLocale：改字段并经 onEvent 投递 {type:"env", locale}。 */
export function setMockLocale(next: string): void {
  currentLocale = next || "en";
  emitHostEvent({ type: "env", locale: currentLocale });
}
