import { computed, ref } from "vue";
import { pluginStore } from "../../shared/lib/pluginStore";
import { BUILTIN_TERMINAL_SCHEMES, schemeTone } from "../../features/terminal/lib/terminalScheme";
import type { TerminalAppearanceSettings, TerminalAppearanceState } from "../../features/terminal/lib/terminalAppearance";

/**
 * dock 面板表面底部工具条的本地 UI 态：工具条隐藏偏好（pluginStore 单键，
 * 跨会话记忆）与「配色快切」弹层（与当前宿主明暗档同调性的方案列表，
 * 单击写入对应槽位并启用 custom 来源）。配色写入本体仍由
 * useTerminalAppearance 承载，这里只做弹层状态与槽位派生。
 */
export function useWorkbenchToolbar(options: {
  appearanceColorScheme: () => string;
  terminalAppearance: () => TerminalAppearanceState;
  updateAppearance: (patch: Partial<TerminalAppearanceSettings>) => void;
}) {
  const { appearanceColorScheme, terminalAppearance, updateAppearance } = options;

  // Dock 面板表面的配色快切弹层（底部栏精简工具条）：列出与当前宿主明暗档
  // 同调性的方案，单击即写入对应槽位并启用 custom 来源；「跟随宿主」恢复 host。
  const panelPaletteOpen = ref(false);
  // dock 工具条隐藏偏好（pluginStore 单键，跨会话记忆）：隐藏后由右上角低透明度
  // 微钮还原（ChevronDown），避免「藏了找不回」。
  const panelToolbarHidden = ref(pluginStore.getItem("ssh-panel-toolbar-hidden") === "1");
  function togglePanelToolbar() {
    panelToolbarHidden.value = !panelToolbarHidden.value;
    try {
      if (panelToolbarHidden.value) pluginStore.setItem("ssh-panel-toolbar-hidden", "1");
      else pluginStore.removeItem("ssh-panel-toolbar-hidden");
    } catch {
      // 存储不可用时仅本次会话生效。
    }
  }
  const panelPaletteSlot = computed<"darkSchemeId" | "lightSchemeId">(() =>
    appearanceColorScheme() === "light" ? "lightSchemeId" : "darkSchemeId",
  );
  const panelSchemeChoices = computed(() => {
    const tone = panelPaletteSlot.value === "lightSchemeId" ? "light" : "dark";
    return [...BUILTIN_TERMINAL_SCHEMES, ...terminalAppearance().customSchemes].filter((scheme) => schemeTone(scheme) === tone);
  });
  const panelActiveSchemeId = computed(() => terminalAppearance().settings.schemeSource === "custom" ? terminalAppearance().settings[panelPaletteSlot.value] : null);
  /** 配色快切：null = 跟随宿主（关闭 custom 来源）；非空写入当前明暗档槽位。 */
  function pickPanelScheme(id: string | null) {
    panelPaletteOpen.value = false;
    updateAppearance(id == null
      ? { schemeSource: "host" }
      : { schemeSource: "custom", [panelPaletteSlot.value]: id });
  }

  return {
    panelPaletteOpen,
    panelToolbarHidden,
    togglePanelToolbar,
    panelPaletteSlot,
    panelSchemeChoices,
    panelActiveSchemeId,
    pickPanelScheme,
  };
}
