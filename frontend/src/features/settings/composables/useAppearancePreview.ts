// 终端外观预览的纯计算（配色方案 / 外观两栏共用）：预览与真实终端共用同一套
// 纯函数合成——预览呈现的就是 xterm 实际拿到的主题，不会出现「设置页好看、
// 终端不对」的偏差。输入从调用方组件 props 惰性读取。
import { computed } from "vue";
import {
  allAppearanceProfiles,
  applySchemeToTerminalTheme,
  terminalOptionPatch,
  type TerminalAppearanceProfile,
  type TerminalAppearanceState,
} from "../../terminal/lib/terminalAppearance";
import { contrastRatio, parseHexColor, type TerminalThemeLike } from "../../terminal/lib/terminalScheme";

export function useAppearancePreview(options: {
  appearance: () => TerminalAppearanceState;
  customThemes: () => TerminalAppearanceProfile[];
  hostTheme: () => TerminalThemeLike;
  hostColorScheme: () => "light" | "dark";
  hostFontFamily: () => string;
}) {
  const appearanceSettings = computed(() => options.appearance().settings);
  const appearanceProfiles = computed(() => allAppearanceProfiles({ ...options.appearance(), customThemes: options.customThemes() }));

  const previewTheme = computed(() => applySchemeToTerminalTheme(
    options.hostTheme(),
    appearanceSettings.value,
    options.appearance().customSchemes,
    options.hostColorScheme(),
  ));
  const previewOptions = computed(() => terminalOptionPatch(appearanceSettings.value));
  const previewFontFamily = computed(() => options.appearance().font.family ?? options.hostFontFamily());
  const previewContrast = computed(() => {
    const foreground = parseHexColor(previewTheme.value.foreground);
    const background = parseHexColor(previewTheme.value.background);
    return foreground && background ? contrastRatio(foreground, background) : 21;
  });

  return {
    appearanceSettings,
    appearanceProfiles,
    previewTheme,
    previewOptions,
    previewFontFamily,
    previewContrast,
  };
}
