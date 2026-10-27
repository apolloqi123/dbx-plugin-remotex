<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { basicSetup } from "codemirror";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { tags } from "@lezer/highlight";
// 语法高亮调色板来自 shared 公共层（唯一实现点），暗色为提亮后的 GitHub Dark 系。
import { dbxSyntaxHighlight } from "../../../../../shared/frontend/editorTheme";
import EditorFindBar from "./EditorFindBar.vue";

const props = defineProps<{
  text: string;
  fileName: string;
  appearance: DbxPluginAppearance;
  editable?: boolean;
  /** Optional localizer: preview dialogs may mount without one. */
  t?: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  change: [text: string];
}>();

const tr = (key: string, values?: Record<string, string | number>): string =>
  props.t ? props.t(key, values) : key;

const host = ref<HTMLElement>();
const findBar = ref<InstanceType<typeof EditorFindBar>>();
let view: EditorView | undefined;
let generation = 0;

// —— 查找/替换（VS Code 风格浮层）—————————————————————————————————————
// basicSetup 自带的底部搜索面板保持禁用：Mod-F/Mod-H 在编辑器容器上以捕获
// 阶段拦截，改为驱动这个右上角浮层；匹配由文档文本 + RegExp 计算，避免依赖
// @codemirror/search 的 UI 和额外依赖。
const MAX_FIND_MATCHES = 10_000;
const findOpen = ref(false);
const replaceOpen = ref(false);
const query = ref("");
const replacement = ref("");
const caseSensitive = ref(false);
const wholeWord = ref(false);
const useRegex = ref(false);
const invalidRegex = ref(false);
const matches = ref<Array<{ from: number; to: number }>>([]);
const activeMatch = ref(0);
let findTimer: ReturnType<typeof setTimeout> | undefined;

const countLabel = computed(() => {
  if (!findOpen.value || !query.value) return "";
  if (invalidRegex.value) return tr("terminalSearch.noMatch");
  if (!matches.value.length) return tr("terminalSearch.noMatch");
  return `${activeMatch.value + 1}/${matches.value.length}`;
});

function docText(): string {
  return view?.state.doc.toString() ?? "";
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Pattern source with whole-word wrapping; null when the regex is invalid. */
function patternSource(): string | null {
  if (!query.value) return null;
  const source = useRegex.value ? query.value : escapeRegExp(query.value);
  return wholeWord.value ? `\\b(?:${source})\\b` : source;
}

function buildPattern(): RegExp | null {
  const source = patternSource();
  if (source === null) return null;
  try {
    return new RegExp(source, caseSensitive.value ? "gu" : "giu");
  } catch {
    return null;
  }
}

function collectMatches(): Array<{ from: number; to: number }> {
  const pattern = buildPattern();
  invalidRegex.value = pattern === null && !!query.value;
  if (!pattern) return [];
  const text = docText();
  const found: Array<{ from: number; to: number }> = [];
  for (const match of text.matchAll(pattern)) {
    const from = match.index ?? 0;
    const length = match[0].length;
    if (length === 0) continue;
    found.push({ from, to: from + length });
    if (found.length >= MAX_FIND_MATCHES) break;
  }
  return found;
}

function selectionMain(): { from: number; to: number; head: number } | undefined {
  return view?.state.selection?.main;
}

/** Recomputes matches and keeps the active index near the editor selection. */
function refreshMatches(): void {
  if (!findOpen.value) {
    matches.value = [];
    return;
  }
  const found = collectMatches();
  matches.value = found;
  if (!found.length) {
    activeMatch.value = 0;
    return;
  }
  const selection = selectionMain();
  const head = selection?.head ?? 0;
  let index = selection
    ? found.findIndex((match) => match.from === selection.from && match.to === selection.to)
    : -1;
  if (index < 0) index = found.findIndex((match) => match.from >= head);
  if (index < 0) index = 0;
  activeMatch.value = index;
}

function scheduleFind(immediate = false): void {
  if (findTimer) clearTimeout(findTimer);
  findTimer = setTimeout(() => {
    findTimer = undefined;
    refreshMatches();
  }, immediate ? 0 : 90);
}

function selectMatch(index: number): void {
  const list = matches.value;
  if (!view || !list.length) return;
  const next = ((index % list.length) + list.length) % list.length;
  activeMatch.value = next;
  const { from, to } = list[next];
  view.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
}

function selectionIsMatch(): boolean {
  const selection = selectionMain();
  const match = matches.value[activeMatch.value];
  return !!selection && !!match && selection.from === match.from && selection.to === match.to;
}

function findNext(): void {
  if (!matches.value.length) return;
  selectMatch(selectionIsMatch() ? activeMatch.value + 1 : activeMatch.value);
}

function findPrev(): void {
  if (!matches.value.length) return;
  selectMatch(selectionIsMatch() ? activeMatch.value - 1 : activeMatch.value);
}

function replacementFor(matchedText: string): string {
  if (!useRegex.value) return replacement.value;
  const source = patternSource();
  if (!source) return replacement.value;
  try {
    return matchedText.replace(
      new RegExp(source, caseSensitive.value ? "u" : "iu"),
      replacement.value,
    );
  } catch {
    return replacement.value;
  }
}

function replaceCurrent(): void {
  if (!view) return;
  const selection = selectionMain();
  const target =
    selection && selection.from !== selection.to
      ? { from: selection.from, to: selection.to }
      : matches.value[activeMatch.value];
  if (!target) return;
  const text = docText().slice(target.from, target.to);
  view.dispatch({ changes: { from: target.from, to: target.to, insert: replacementFor(text) } });
  refreshMatches();
  findNext();
}

function replaceAll(): void {
  if (!view || !matches.value.length) return;
  const changes = matches.value
    .map((match) => ({
      from: match.from,
      to: match.to,
      insert: replacementFor(docText().slice(match.from, match.to)),
    }))
    .reverse();
  view.dispatch({ changes });
  refreshMatches();
}

function openFind(withReplace: boolean): void {
  if (!view) return;
  const selection = selectionMain();
  if (selection && selection.from !== selection.to) {
    const selected = docText().slice(selection.from, selection.to);
    if (selected && !selected.includes("\n")) query.value = selected;
  }
  findOpen.value = true;
  replaceOpen.value = withReplace || replaceOpen.value;
  refreshMatches();
  void nextTick(() => findBar.value?.focusFind());
}

function closeFind(): void {
  findOpen.value = false;
  matches.value = [];
  invalidRegex.value = false;
  void nextTick(() => view?.focus?.());
}

function toggleReplace(): void {
  replaceOpen.value = !replaceOpen.value;
  if (replaceOpen.value) {
    void nextTick(() => findBar.value?.focusReplace());
  }
}

/** Captures Mod-F/Mod-H/F3/Escape before CodeMirror's own search keymap. */
function onHostKeydown(event: KeyboardEvent): void {
  const target = event.target as HTMLElement | null;
  if (target?.closest(".editor-find")) return;
  const mod = event.metaKey || event.ctrlKey;
  const key = event.key.toLowerCase();
  if (mod && !event.altKey && key === "f") {
    event.preventDefault();
    event.stopPropagation();
    openFind(false);
    return;
  }
  if (mod && !event.altKey && key === "h") {
    event.preventDefault();
    event.stopPropagation();
    openFind(true);
    return;
  }
  if (findOpen.value && event.key === "Escape") {
    event.preventDefault();
    closeFind();
    return;
  }
  if (findOpen.value && event.key === "F3") {
    event.preventDefault();
    if (event.shiftKey) findPrev();
    else findNext();
  }
}

watch([query, caseSensitive, wholeWord, useRegex], () => scheduleFind());
watch(findOpen, (open) => {
  if (open) scheduleFind(true);
});

onBeforeUnmount(() => {
  if (findTimer) clearTimeout(findTimer);
});

function previewTheme() {
  const colors = props.appearance.colors;
  return EditorView.theme({
    "&": { height: "100%", backgroundColor: colors.background, color: colors.foreground },
    ".cm-scroller": {
      overflow: "auto",
      fontFamily: props.appearance.terminal.fontFamily,
      fontSize: `${props.appearance.terminal.fontSize}px`,
    },
    ".cm-gutters": { backgroundColor: colors.muted, color: colors.mutedForeground, borderRightColor: colors.border },
    ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: colors.accent },
    // 选区色随明暗切换，与终端 xterm selectionBackground 保持同一观感。
    // 透明度 0x88 的旧值在亮色白底上混合后对比度仅 ~1.2:1（issue #155：
    // 选区与背景几乎一样），提高到 0xad 并加深色相，选中区域清晰可辨。
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
      backgroundColor: props.appearance.colorScheme === "dark" ? "#6481b8ad" : "#7cabecad",
    },
  }, { dark: props.appearance.colorScheme === "dark" });
}

async function extensions() {
  const language = LanguageDescription.matchFilename(languages, props.fileName);
  const support = language ? await language.load().catch(() => undefined) : undefined;
  return [
    basicSetup,
    // basicSetup 内置 defaultHighlightStyle 是浅底配色，暗色下发暗；此处按宿主
    // 明暗注入 shared 调色板高亮（后声明者优先，内置样式退为 fallback），
    // appearance 变化重建编辑器时随 colorScheme 自然跟随。
    dbxSyntaxHighlight(props.appearance.colorScheme, { HighlightStyle, syntaxHighlighting, tags }) as Extension,
    EditorState.readOnly.of(!props.editable),
    EditorView.editable.of(props.editable === true),
    EditorView.lineWrapping,
    previewTheme(),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        emit("change", update.state.doc.toString());
        if (findOpen.value) scheduleFind();
      }
    }),
    ...(support ? [support] : []),
  ];
}

async function createEditor() {
  const current = ++generation;
  const configured = await extensions();
  if (!host.value || current !== generation) return;
  // Seed the new editor with the live document so edits survive theme or
  // editable-mode re-creations; fall back to the incoming text on first mount.
  const doc = view?.state.doc.toString() ?? props.text;
  view?.destroy();
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({ doc, extensions: configured }),
  });
}

onMounted(createEditor);
watch(() => [props.fileName, props.appearance, props.editable] as const, createEditor, { deep: true });
watch(() => props.text, (text) => {
  if (!view || text === view.state.doc.toString()) return;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
});
onBeforeUnmount(() => {
  generation += 1;
  if (findTimer) clearTimeout(findTimer);
  view?.destroy();
});
</script>

<template>
  <div class="preview-editor" @keydown.capture="onHostKeydown">
    <div ref="host" class="preview-editor-cm" />
    <EditorFindBar
      v-if="findOpen"
      ref="findBar"
      :t="tr"
      :query="query"
      :replacement="replacement"
      :count-label="countLabel"
      :invalid-regex="invalidRegex"
      :replace-open="replaceOpen"
      :case-sensitive="caseSensitive"
      :whole-word="wholeWord"
      :use-regex="useRegex"
      @update:query="query = $event"
      @update:replacement="replacement = $event"
      @update:case-sensitive="caseSensitive = $event"
      @update:whole-word="wholeWord = $event"
      @update:use-regex="useRegex = $event"
      @next="findNext"
      @prev="findPrev"
      @close="closeFind"
      @toggle-replace="toggleReplace"
      @replace="replaceCurrent"
      @replace-all="replaceAll"
    />
  </div>
</template>

<style scoped>
.preview-editor {
  position: relative;
  height: 100%;
}

.preview-editor-cm {
  height: 100%;
}

</style>
