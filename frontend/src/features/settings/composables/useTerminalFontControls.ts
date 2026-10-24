// 终端字体控件态（issue #31）：预设等宽字体 + 跟随宿主 + 自定义；哨兵值只作
// 选择器键使用，不会作为字体串写入（写入前映射回 null/真实字体串）。状态由
// 设置弹窗持有（初始化一次，应用/恢复默认后刷新），外观分栏只负责渲染。
import { reactive } from "vue";
import { clampFontSize } from "../../terminal/lib/terminalZoom";
import { loadTerminalFontOverride } from "../../terminal/lib/terminalFont";

const FOLLOW_HOST = "__follow-host__";
const CUSTOM = "__custom__";
const FONT_PRESETS: Array<{ value: string; label: string }> = [
  { value: "'JetBrains Mono', Consolas, monospace", label: "JetBrains Mono" },
  { value: "'Cascadia Code', 'Cascadia Mono', monospace", label: "Cascadia Code" },
  { value: "'Fira Code', monospace", label: "Fira Code" },
  { value: "'Source Code Pro', monospace", label: "Source Code Pro" },
  { value: "Menlo, Monaco, monospace", label: "Menlo" },
  { value: "Consolas, 'Courier New', monospace", label: "Consolas" },
  { value: "'DejaVu Sans Mono', monospace", label: "DejaVu Sans Mono" },
  { value: "monospace", label: "monospace" },
];

export function useTerminalFontControls(options: {
  terminalFontSize: () => number;
  hostFontSize: () => number;
  emitApplyFont: (payload: { family: string | null; size: number }) => void;
}) {
  const state = reactive({
    familyChoice: FOLLOW_HOST as string,
    customDraft: "",
    sizeDraft: String(options.terminalFontSize()),
  });

  // 字体控件回显当前覆盖态：初始化与应用/恢复默认后都会刷新，避免草稿漂移。
  function syncControls(family: string | null, size: number) {
    if (!family) {
      state.familyChoice = FOLLOW_HOST;
      state.customDraft = "";
    } else if (FONT_PRESETS.some((preset) => preset.value === family)) {
      state.familyChoice = family;
      state.customDraft = "";
    } else {
      state.familyChoice = CUSTOM;
      state.customDraft = family;
    }
    state.sizeDraft = String(size);
  }
  syncControls(loadTerminalFontOverride().fontFamily, options.terminalFontSize());

  /// 应用用户字体设置：应用侧（App）负责落到 xterm、持久化与 toast。
  function applyTerminalFont(family: string | null, size: number) {
    options.emitApplyFont({ family, size });
    syncControls(family, size);
  }

  // 设置页字体族下拉：跟随宿主/预设即时应用并持久化；选「自定义」时仅展开
  // 输入框，待输入 @change 再应用（applyTerminalFontFromControls）。
  function onTerminalFontFamilyChoice(choice: string) {
    state.familyChoice = choice;
    if (choice === CUSTOM) return;
    applyTerminalFont(choice === FOLLOW_HOST ? null : choice, options.terminalFontSize());
  }

  function applyTerminalFontFromControls() {
    let family: string | null;
    if (state.familyChoice === CUSTOM) {
      const custom = state.customDraft.trim();
      family = custom.length > 0 ? custom : null; // 空自定义视作跟随宿主
    } else if (state.familyChoice === FOLLOW_HOST) {
      family = null;
    } else {
      family = state.familyChoice;
    }
    const parsed = Number(state.sizeDraft);
    const size = Number.isFinite(parsed) && parsed > 0 ? clampFontSize(parsed, 0) : options.hostFontSize();
    applyTerminalFont(family, size);
  }

  return {
    state,
    presets: FONT_PRESETS,
    followHost: FOLLOW_HOST,
    custom: CUSTOM,
    syncControls,
    applyTerminalFont,
    onTerminalFontFamilyChoice,
    applyTerminalFontFromControls,
  };
}

export type TerminalFontControls = ReturnType<typeof useTerminalFontControls>;
