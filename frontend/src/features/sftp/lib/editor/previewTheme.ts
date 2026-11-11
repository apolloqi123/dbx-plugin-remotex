// 文件预览/编辑器（TextPreview）的 CodeMirror 主题：宿主外观令牌 → EditorView.theme
// 的样式表。单独成模块是为了把「选中文本 = 主高亮、同词匹配 = 次高亮」的层级
// 写成可单测的纯数据，避免以后又被基础主题或随手改色盖回去。

type ThemeColors = DbxPluginAppearance["colors"];

export interface PreviewThemeOptions {
  dark: boolean;
  colors: ThemeColors;
  fontFamily: string;
  fontSize: number;
}

export interface PreviewHighlightColors {
  /** 选中的文本（主高亮）。 */
  selection: string;
  /** 同词其它出现（次高亮底色）。 */
  selectionMatch: string;
  /** 同词其它出现（次高亮描边）。 */
  selectionMatchOutline: string;
}

/**
 * 选区底色选择器必须复刻 CodeMirror 基础主题的层级：基础主题里
 * `&dark.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground`
 * 的特异性更高，只写 `.cm-selectionBackground` 会在编辑器聚焦时被它的深色
 * 兜底（暗色 #223）覆盖，选中的文本几乎看不出高亮。
 */
export const SELECTION_BACKGROUND_SELECTOR =
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, "
  + "& > .cm-scroller > .cm-selectionLayer .cm-selectionBackground";

/** 主/次高亮配色：同色系、两级明度，选区永远强于同词匹配。
 *  暗色下背景吸收大半透明色，必须比亮色更实（否则蓝色发灰、看不出来）。 */
export function previewHighlightColors(dark: boolean): PreviewHighlightColors {
  return dark
    ? {
        selection: "rgb(59 130 246 / 95%)",
        selectionMatch: "rgb(96 165 250 / 34%)",
        selectionMatchOutline: "rgb(147 197 253 / 60%)",
      }
    : {
        selection: "rgb(59 130 246 / 72%)",
        selectionMatch: "rgb(37 99 235 / 12%)",
        selectionMatchOutline: "rgb(37 99 235 / 28%)",
      };
}

/** `EditorView.theme` 的样式表（纯数据，便于断言选择器与配色层级）。 */
export function previewThemeSpec(options: PreviewThemeOptions): Record<string, Record<string, string>> {
  const { dark, colors, fontFamily, fontSize } = options;
  const highlight = previewHighlightColors(dark);
  return {
    "&": { height: "100%", backgroundColor: colors.background, color: colors.foreground },
    ".cm-scroller": {
      overflow: "auto",
      fontFamily,
      fontSize: `${fontSize}px`,
      lineHeight: "1.6",
    },
    ".cm-gutters": {
      backgroundColor: colors.muted,
      color: colors.mutedForeground,
      borderRightColor: colors.border,
      paddingRight: "8px",
    },
    // 当前行底色必须半透明：选区层绘制在正文之下，不透明底色会把当前行的
    // 选区整段盖住（双击选中当前行的词几乎看不见的主因）。暗色沿用 accent
    // 色带但压到 30%，否则叠在选区上会把蓝色压灰；亮色 accent 接近白色，
    // 半透明后会冲淡蓝色选区，改用 foreground 的低透明度灰，既保留行提示
    // 又不盖选区。
    ".cm-activeLine": {
      backgroundColor: dark
        ? `color-mix(in srgb, ${colors.accent} 30%, transparent)`
        : `color-mix(in srgb, ${colors.foreground} 6%, transparent)`,
    },
    ".cm-activeLineGutter": { backgroundColor: colors.accent },
    ".cm-cursor": { borderLeftColor: colors.foreground, borderLeftWidth: "2px" },
    [SELECTION_BACKGROUND_SELECTOR]: { backgroundColor: highlight.selection },
    // 同词次高亮（basicSetup 的 highlightSelectionMatches 注入）：淡底 + 细描边。
    ".cm-selectionMatch, .cm-selectionMatch-main": {
      backgroundColor: highlight.selectionMatch,
      outline: `1px solid ${highlight.selectionMatchOutline}`,
      borderRadius: "2px",
    },
    // 查找命中高亮（applyFindDecorations 注入）：全部命中淡琥珀底，当前命中
    // 加深并加 2px 描边。不改变文字颜色，语法高亮在两种主题下都保持可读。
    // 放在同词高亮之后：查找打开时同词匹配同时也是查找命中，琥珀色优先。
    ".cm-find-match": {
      backgroundColor: dark ? "#fbbf2433" : "#f59e0b2e",
      outline: `1px solid ${dark ? "#fbbf2473" : "#d9770666"}`,
      borderRadius: "2px",
    },
    ".cm-find-match-active": {
      backgroundColor: dark ? "#fbbf2480" : "#f59e0b73",
      outline: `2px solid ${dark ? "#fbbf24" : "#d97706"}`,
      borderRadius: "2px",
    },
  };
}
