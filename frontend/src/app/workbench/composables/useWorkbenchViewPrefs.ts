import { computed, ref } from "vue";
import { pluginStore } from "../../../shared/lib/pluginStore";
import {
  sanitizeDirectoryFollowPref,
  sanitizeSftpAutoConnectPref,
  sanitizeSftpPaneDefaultOpen,
} from "../../lib/workbenchLayout";
import {
  loadTerminalBehavior,
  persistTerminalBehavior,
  sanitizeTerminalBehavior,
  terminalBehaviorOptionPatch,
  type TerminalBehaviorSettings,
} from "../../../features/terminal/lib/terminalBehavior";
import {
  loadTerminalHotkeys,
  persistTerminalHotkeys,
  sanitizeTerminalHotkeys,
  type TerminalHotkeyBindings,
} from "../../../features/terminal/lib/terminalHotkeys";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import type { Terminal } from "@xterm/xterm";

// 偏好键（pluginStore 全局持久化）。
const SFTP_PANE_OPEN_KEY = "ssh-sftp-pane-open";
const SFTP_AUTO_CONNECT_KEY = "ssh-sftp-auto-connect";
const FOLLOW_DIRECTORY_KEY = "ssh-follow-directory";

/**
 * 工作台视图/终端行为偏好（pluginStore 单键持久化，宿主 host.storage →
 * localStorage 降级）：
 * - paneDirection（布局轴向）/ SFTP 面板默认打开 / SFTP 自动连接 / 目录跟随；
 * - 终端行为（右键语义、选中复制、响铃、渲染细项）与快捷键绑定。
 *
 * 这些偏好只影响新建/当前工作台的初始与即时行为；per-tab 的 workbenchState
 * 仍由 App 的持久化层写穿（恢复逻辑经返回的 ref 回填）。
 */
export function useWorkbenchViewPrefs(options: {
  applePlatform: boolean;
  terminal: () => Terminal | undefined;
  showNotice: (message: string) => void;
  t: (key: string, values?: Record<string, string | number>) => string;
  scheduleFit: () => void;
}) {
  const { applePlatform, terminal, showNotice, t, scheduleFit } = options;

  // 布局方向偏好（issue #187）：auto 沿用 CSS 断点（窄屏上下/宽屏左右）；
  // vertical 始终上下（终端在上、SFTP 在下），horizontal 始终左右。持久化走
  // WORKBENCH_LAYOUT_KEY 布局偏好档（与 paneOrder 同层，writeWorkbenchState 写穿）。
  const paneDirection = ref<PaneDirectionPref>("auto");
  const sftpPaneDefaultOpen = ref(loadSftpPaneDefaultOpen());
  const sftpAutoConnect = ref(loadSftpAutoConnect());
  // 终端行为偏好（对标 Tabby「Terminal」页）：右键语义、剪贴板、响铃、渲染细项，
  // 单键 pluginStore 持久化；选中复制是其一个字段。
  const terminalBehavior = ref<TerminalBehaviorSettings>(loadTerminalBehavior());
  /** 选中复制（既有消费点：选区变更钩子与设置页开关）。 */
  const termSelectCopy = computed(() => terminalBehavior.value.copyOnSelect);
  // 终端快捷键绑定（对标 Tabby「Hotkeys」页）：平台默认 + 用户改写，单键持久化。
  const terminalHotkeys = ref<TerminalHotkeyBindings>(loadTerminalHotkeys(applePlatform));
  // 目录跟随全局偏好：开关切换时同步写入（与 per-tab workbenchState 并行，
  // 后者为恢复的 tab 提供更精确的状态；这里是新 tab 的缺省来源）。
  const followDirectory = ref(loadDirectoryFollowPref());

  // 全局偏好只影响新工作台的初始面板状态；当前工作台不被连带切换。
  function toggleSftpPaneDefaultOpen() {
    sftpPaneDefaultOpen.value = !sftpPaneDefaultOpen.value;
    try {
      pluginStore.setItem(SFTP_PANE_OPEN_KEY, sftpPaneDefaultOpen.value ? "true" : "false");
    } catch {
      // localStorage 不可用时偏好仅对当前会话生效。
    }
  }

  function loadSftpPaneDefaultOpen(): boolean {
    try {
      return sanitizeSftpPaneDefaultOpen(pluginStore.getItem(SFTP_PANE_OPEN_KEY));
    } catch {
      return false;
    }
  }

  // SFTP 自动连接全局偏好：关闭后连接成功不再自动加载 SFTP 目录（#179）。
  function toggleSftpAutoConnect() {
    sftpAutoConnect.value = !sftpAutoConnect.value;
    try {
      pluginStore.setItem(SFTP_AUTO_CONNECT_KEY, sftpAutoConnect.value ? "true" : "false");
    } catch {
      // localStorage 不可用时偏好仅对当前会话生效。
    }
  }

  function loadSftpAutoConnect(): boolean {
    try {
      return sanitizeSftpAutoConnectPref(pluginStore.getItem(SFTP_AUTO_CONNECT_KEY));
    } catch {
      return true;
    }
  }

  function loadDirectoryFollowPref(): boolean {
    try {
      return sanitizeDirectoryFollowPref(pluginStore.getItem(FOLLOW_DIRECTORY_KEY));
    } catch {
      return false;
    }
  }

  function persistDirectoryFollowPref(enabled: boolean) {
    try {
      pluginStore.setItem(FOLLOW_DIRECTORY_KEY, enabled ? "true" : "false");
    } catch {
      // pluginStore 不可用时全局偏好仅对当前会话生效，per-tab 恢复不受影响。
    }
  }

  /**
   * 终端行为落地：把行为偏好写进 xterm 选项。字体/行高/字间距等外观项不在此处
   * （见 applyTerminalAppearance），这里只管行为类选项。
   */
  function applyTerminalBehavior() {
    const term = terminal();
    if (!term) return;
    const patch = terminalBehaviorOptionPatch(terminalBehavior.value);
    term.options.scrollback = patch.scrollback;
    term.options.scrollOnUserInput = patch.scrollOnUserInput;
    term.options.wordSeparator = patch.wordSeparator;
    term.options.ignoreBracketedPasteMode = patch.ignoreBracketedPasteMode;
    term.options.macOptionIsMeta = patch.macOptionIsMeta;
  }

  // 布局方向偏好改动（issue #187）：轴向切换后终端/SFTP 尺寸变化，重排 xterm。
  // 持久化由 WORKBENCH_LAYOUT watch 统一写穿。
  function setPaneDirection(value: PaneDirectionPref) {
    if (paneDirection.value === value) return;
    paneDirection.value = value;
    scheduleFit();
  }

  /** 行为设置局部更新（设置页控件）：归一化 → 持久化 → 即时生效。 */
  function updateTerminalBehavior(patch: Partial<TerminalBehaviorSettings>) {
    const next = sanitizeTerminalBehavior({ ...terminalBehavior.value, ...patch });
    const copyChanged = next.copyOnSelect !== terminalBehavior.value.copyOnSelect;
    terminalBehavior.value = next;
    persistTerminalBehavior(next);
    applyTerminalBehavior();
    // 选中复制沿用既有即时反馈文案。
    if (copyChanged) {
      showNotice(t(next.copyOnSelect ? "terminalSelectCopy.enabledNotice" : "terminalSelectCopy.disabledNotice"));
    }
  }

  /** 快捷键绑定更新：归一化 → 持久化。派发每次按键实时读表，无需重挂钩子。 */
  function updateTerminalHotkeys(bindings: TerminalHotkeyBindings) {
    const next = sanitizeTerminalHotkeys(bindings, applePlatform);
    terminalHotkeys.value = next;
    persistTerminalHotkeys(next);
  }

  return {
    paneDirection,
    sftpPaneDefaultOpen,
    sftpAutoConnect,
    terminalBehavior,
    termSelectCopy,
    terminalHotkeys,
    followDirectory,
    toggleSftpPaneDefaultOpen,
    loadSftpPaneDefaultOpen,
    toggleSftpAutoConnect,
    loadSftpAutoConnect,
    loadDirectoryFollowPref,
    persistDirectoryFollowPref,
    applyTerminalBehavior,
    setPaneDirection,
    updateTerminalBehavior,
    updateTerminalHotkeys,
  };
}
