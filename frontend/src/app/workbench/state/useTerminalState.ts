import { computed, reactive, ref, shallowRef, useTemplateRef } from "vue";
import type { Terminal } from "@xterm/xterm";
import { createTerminalCopyCache } from "../../../features/terminal/lib/behavior/terminalInteraction";
import {
  activeProfileId,
  loadTerminalAppearance,
  type TerminalAppearanceState,
} from "../../../features/terminal/lib/appearance/terminalAppearance";
import {
  loadTerminalFontOverride,
  type TerminalFontOverride,
} from "../../../features/terminal/lib/appearance/terminalFont";

/** 工作台会话态词表（连接卡片/状态胶囊/断线梯子共用）。 */
export type TerminalConnectionState = "connecting" | "connected" | "disconnected" | "error";

/** 终端外观来源（宿主 appearance）：字号初值取自宿主下发的终端字号。 */
export interface TerminalAppearanceSource {
  readonly value: { readonly terminal: { readonly fontSize: number } };
}

/**
 * 主终端域状态（Task 21b）：xterm 实例别名/addons 之外的「模板共享态」。
 *
 * 纯本地工厂：每次调用创建一份新的 refs（不是全局单例），由
 * `useWorkbenchContext` 组合进工作台上下文。行为（实例创建、addons、输出管线、
 * 输入路由）仍留在 App.vue 与各 composable，此文件只持有声明与初始值。
 */
export function useTerminalState(options: { appearance: TerminalAppearanceSource }) {
  /** 终端舞台组件实例：xterm 宿主 ref 由 WorkbenchStage 的 defineExpose 回传
   *  （T20）。模板 ref 在子组件挂载后写入，computed 惰性读取，因此 composable
   *  持有的注册表在会话打开时总能拿到真实宿主，且不产生第二个宿主元素。 */
  const stage = useTemplateRef<{ terminalHost?: HTMLElement }>("stage");
  const terminalHost = computed(() => stage.value?.terminalHost);
  // —— xterm 实例：创建/addons/宿主编排/尺寸 fit/WebGL 开关与销毁收口在
  // features/terminal/composables/session/useTerminalInstance。实例由它持有并
  // 在创建时回填下面的别名（既有大量调用点与模板绑定直接读 terminal）；
  // 跨模块依赖一律经 getter/回调注入。 ——
  // 声明类型保持 `Terminal | undefined`（未创建前为空），ref 只在 getter 里读，
  // 与迁移前的 `let` 别名语义一致（模板侧自动解包，`terminal?.focus()` 不变）。
  const terminal = shallowRef<Terminal>();
  const terminalState = ref<TerminalConnectionState>("connecting");
  // 终端 shell 最近一次上报的 cwd（OSC 7 / OSC 633 Cwd，无论跟随开关是否打开
  // 都记录）：终端拖拽上传的「当前目录」落点解析靠它，避免误用 SFTP 面板的
  // 浏览目录（初始值 "/"，拼根路径会被服务器以权限拒绝）。
  const terminalCwd = ref("");
  // 沙箱宿主读不到系统剪贴板：右键粘贴的降级链靠这份插件视图内的复制副本。
  const terminalCopyCache = createTerminalCopyCache();
  const directoryTrackingSupported = ref<boolean | undefined>();
  const terminalFontSize = ref(options.appearance.value.terminal.fontSize);
  // 终端字体单独设置（issue #31）：字体族/字号的用户覆盖，null 字段 = 跟随宿主。
  // setup 期读取安全：loadTerminalFontOverride 经 pluginStore（内部全 guarded，
  // opaque origin 不抛错），见 lib/terminalFont.ts 说明。
  const terminalFontOverride = ref<TerminalFontOverride>(loadTerminalFontOverride());
  // 终端外观偏好（对标 Tabby 的 Settings → Appearance）：配色方案两槽（随宿主
  // 亮暗自动切换）、底色策略、字体间距、光标形态与多套主题快照。默认态为
  // schemeSource: "host"，即不启用任何方案、行为与既有版本完全一致。
  //
  // 字体字段（font）不在此持久化状态里作为真相：权威态是 terminalFontOverride
  // （Ctrl+滚轮缩放也写它）。读取时用 terminalAppearanceState 覆盖合成，避免
  // 「缩放改过字号后主题仍高亮」这类双真相漂移。
  const terminalAppearance = ref<TerminalAppearanceState>(loadTerminalAppearance());
  const terminalAppearanceState = computed<TerminalAppearanceState>(() => ({
    ...terminalAppearance.value,
    font: { family: terminalFontOverride.value.fontFamily, size: terminalFontOverride.value.fontSize },
  }));
  // 当前配置命中的主题 id（null = 已改动，不再等于任何预设/我的主题）。
  const activeAppearanceThemeId = computed(() => activeProfileId(terminalAppearanceState.value));
  // #33/#71 快速输入丢字母的分层计数：keys(onData 实际路由到 PTY 的按键)、
  // sends(提交给宿主桥的帧)、acks(sidecar 确认收到的帧)、errors(桥拒绝)、
  // swallowed(被 zmodem/trzsz 路由吞掉的按键)。宿主开启 localStorage 的
  // dbx-term-diag=1 后每 2s 在控制台输出；始终挂在 window 上便于随时读取。
  const terminalDiag = reactive({ keys: 0, sends: 0, acks: 0, errors: 0, swallowed: 0 });
  const terminalDiagVisible = ref(false);

  return {
    terminal,
    terminalHost,
    terminalState,
    terminalCwd,
    terminalCopyCache,
    directoryTrackingSupported,
    terminalFontSize,
    terminalFontOverride,
    terminalAppearance,
    terminalAppearanceState,
    activeAppearanceThemeId,
    terminalDiag,
    terminalDiagVisible,
  };
}

export type TerminalState = ReturnType<typeof useTerminalState>;
