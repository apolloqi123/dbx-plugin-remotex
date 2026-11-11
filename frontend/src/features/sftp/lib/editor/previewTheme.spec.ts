// 预览编辑器主题的层级约定（回归夹具）：
// 1. 选区底色必须挂在 CodeMirror 基础主题同形状的选择器上，否则聚焦时会被
//    基础主题的深色兜底覆盖（就是「双击选中看不出高亮」的根因）。
// 2. 当前行底色必须半透明，否则会盖住画在正文下方的选区层。
// 3. 同词匹配（cm-selectionMatch）必须弱于选区，且查找命中的琥珀色优先级最高。
import { describe, expect, it } from "vitest";
import {
  previewHighlightColors,
  previewThemeSpec,
  SELECTION_BACKGROUND_SELECTOR,
} from "./previewTheme";

const colors: DbxPluginAppearance["colors"] = {
  background: "rgb(19 20 22)",
  foreground: "rgb(215 215 219)",
  muted: "rgb(42 42 45)",
  mutedForeground: "rgb(151 152 157)",
  accent: "rgb(46 47 51)",
  accentForeground: "rgb(221 221 226)",
  border: "rgb(110 110 114 / 0.28)",
  destructive: "rgb(243 98 95)",
};

function spec(options: Partial<Parameters<typeof previewThemeSpec>[0]> = {}) {
  return previewThemeSpec({
    dark: true,
    colors,
    fontFamily: "monospace",
    fontSize: 12,
    ...options,
  });
}

/** 从 `rgb(r g b / a)` 里取 alpha（0..1），用于比较高亮强弱。 */
function alphaOf(value: string): number {
  const match = /\/\s*([\d.]+)(%?)\)/.exec(value);
  if (!match) return 1;
  const numeric = Number(match[1]);
  return match[2] ? numeric / 100 : numeric;
}

describe("previewThemeSpec", () => {
  it("puts the selection background on the same selector shape as the base theme", () => {
    // 基础主题：`&dark.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground`
    // 只写 `.cm-selectionBackground` 特异性不足，聚焦时会被它覆盖。
    expect(SELECTION_BACKGROUND_SELECTOR).toContain("&.cm-focused > .cm-scroller > .cm-selectionLayer");
    expect(SELECTION_BACKGROUND_SELECTOR).toContain("& > .cm-scroller > .cm-selectionLayer");
    expect(spec()[SELECTION_BACKGROUND_SELECTOR]?.backgroundColor).toBe(previewHighlightColors(true).selection);
  });

  it("keeps the active line translucent so it never hides the selection layer", () => {
    // 不透明底色会盖住正文下方的选区层：当前行选中的词看着像没选中。
    expect(spec()[".cm-activeLine"].backgroundColor).toContain("transparent");
    expect(spec({ dark: false })[".cm-activeLine"].backgroundColor).toContain("transparent");
    // 行号栏不叠在正文上，保持实色即可。
    expect(spec()[".cm-activeLineGutter"].backgroundColor).toBe(colors.accent);
  });

  it("renders same-word matches as a weaker secondary highlight than the selection", () => {
    for (const dark of [true, false]) {
      const theme = spec({ dark });
      const selection = alphaOf(theme[SELECTION_BACKGROUND_SELECTOR].backgroundColor);
      const match = alphaOf(theme[".cm-selectionMatch, .cm-selectionMatch-main"].backgroundColor);
      expect(selection).toBeGreaterThan(match);
      expect(match).toBeGreaterThan(0);
    }
  });

  it("strengthens the highlights in dark mode, where translucent fills wash out", () => {
    // 暗色背景吸收半透明色：同一透明度在暗色下发灰、在亮色下仍清晰，
    // 因此暗色的主/次高亮都必须比亮色更实（实测暗色 72% 选区不够直观）。
    const dark = previewHighlightColors(true);
    const light = previewHighlightColors(false);
    expect(alphaOf(dark.selection)).toBeGreaterThanOrEqual(0.9);
    expect(alphaOf(dark.selection)).toBeGreaterThan(alphaOf(light.selection));
    expect(alphaOf(dark.selectionMatch)).toBeGreaterThan(alphaOf(light.selectionMatch));
    expect(alphaOf(dark.selectionMatchOutline)).toBeGreaterThan(alphaOf(light.selectionMatchOutline));
  });

  it("keeps find hits amber and declared after the same-word highlight", () => {
    const theme = spec();
    const matchIndex = Object.keys(theme).indexOf(".cm-selectionMatch, .cm-selectionMatch-main");
    const findIndex = Object.keys(theme).indexOf(".cm-find-match");
    expect(matchIndex).toBeGreaterThanOrEqual(0);
    // 查找打开时同词匹配也是查找命中，后声明的琥珀色规则胜出。
    expect(findIndex).toBeGreaterThan(matchIndex);
    expect(theme[".cm-find-match"].backgroundColor).toBe("#fbbf2433");
    // 当前命中比其它命中更醒目（描边 2px）。
    expect(theme[".cm-find-match-active"].outline).toContain("2px");
  });
});
