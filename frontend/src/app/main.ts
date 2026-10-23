import { createApp } from "vue";
import App from "./App.vue";
import TunnelManager from "../features/forwarding/components/TunnelManager.vue";
import { isTunnelManagerContext } from "../shared/lib/pluginContext";
import { TUNNELS_QUICK_ID } from "../shared/lib/pluginIds";
import "@xterm/xterm/css/xterm.css";
import "./style.css";
import "./styles/tailwind.css";
import { installHostThemeBridge } from "../../../shared/frontend/themeSync";
import { pluginStore } from "../shared/lib/pluginStore";
import { settingsPreferences } from "../features/settings/lib/settingsPreferences";
import { watchPopupReveal } from "../shared/lib/popupReveal";

let openedContributionId = "";
document.addEventListener("dbx-plugin-init", (event) => {
  const detail = (event as CustomEvent<{ contributionId?: unknown }>).detail;
  if (typeof detail?.contributionId === "string") openedContributionId = detail.contributionId;
});

async function waitForHostApi(timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs;
  while (!window.dbxPlugin && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return window.dbxPlugin;
}

// 宿主令牌 → 插件变量桥：首绘即命中宿主主题，主题变化经 SDK 令牌更新自动跟随。
// 字体回退值覆盖为插件规范链（UI 字体补 CJK 回退，与 style.css :root 一致；
// 终端字体保持等宽链，与既有默认一致）。
installHostThemeBridge({
  "--ui-font-family": 'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif',
  "--terminal-font-family": '"JetBrains Mono", "Cascadia Mono", Consolas, monospace',
});

// UI 状态水合（宿主 host.storage → localStorage 降级 + 旧键搬家）先于挂载，
// 保证 App setup 内的同步首读（面板形态 / 字体 / WebGL 等偏好）命中持久化值。
const boot = async () => {
  await pluginStore.ready;
  // settingsPreferences 单例在模块求值期创建（早于上面的水合）：首读只能拿到
  // 默认值。这里在 App setup 消费 ref 之前原地重读宿主 storage，否则持久化的
  // webgl/历史面板关闭态与 250 条历史上限会被默认值覆盖（历史上限还会把环
  // 截到 100 条后回写）。必须在 createApp 之前完成。
  settingsPreferences.reloadFromStorage();
  // 弹层动画冻结保险先于挂载装好：宿主渲染器停摆（动画定格透明首帧）时，
  // 弹层挂载 300ms 后被强制落到可见态（lib/popupReveal.ts）。
  watchPopupReveal();
  // A native connection context-menu dialog carries the host's public
  // {id, dbType, name} summary. Route it before App mounts so no PTY is made.
  const api = await waitForHostApi();
  const readyContext: Record<string, unknown> = api ? await api.ready.catch(() => ({})) : {};
  const requestedContext = api ? await api.request<Record<string, unknown>>("host.getContext").catch(() => ({})) : {};
  const context = { ...readyContext, ...requestedContext, quickStart: readyContext.quickStart === true || openedContributionId === TUNNELS_QUICK_ID };
  if (isTunnelManagerContext(context)) createApp(TunnelManager, { context }).mount("#app");
  else createApp(App).mount("#app");
};
boot();
