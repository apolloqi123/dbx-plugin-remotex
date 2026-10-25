import { computed, ref, type Ref } from "vue";
import { defaultPatternFor, newCustomEditorId, resolveEditorForFile, sanitizeEditorConfig, type CustomEditor, type EditorConfig, type KnownEditor } from "../../lib/editor/editorRules";

/** 打开方式子菜单的条目类型（App.vue 的局部接口，按消费字段收敛）。 */
interface EntryLike {
  name: string;
  uri: string;
}

/**
 * 「打开方式」子菜单与自定义编辑器命令（编辑器映射增强）：子菜单 = 系统
 * 默认 + sidecar 探测到的可用编辑器 + 用户自定义命令，选中即开（一次性，
 * 不写关联）；持久化关联走设置弹窗或自定义命令弹窗的「记住为默认」。
 * 命令串由 sidecar argv 化执行（不过 shell），pattern 取文件扩展名掩码。
 */
export function useEditorOpenWith<TEntry extends EntryLike>(options: {
  editorConfig: Ref<EditorConfig>;
  externalEditors: Ref<KnownEditor[]>;
  updateEditorConfig: (next: EditorConfig) => void;
  openInExternalEditor: (entry: TEntry, target: { kind: "system" } | { kind: "known"; editor: KnownEditor } | { kind: "custom"; editor: CustomEditor }) => Promise<unknown> | unknown;
}) {
  const { editorConfig, externalEditors, updateEditorConfig, openInExternalEditor } = options;

  const availableExternalEditors = computed(() => externalEditors.value.filter((editor) => editor.available));

  /** 当前文件按关联/默认解析到的编辑器 id（子菜单打勾标识；system 返回空串）。 */
  function mappedEditorIdFor(fileName: string): string {
    const resolved = resolveEditorForFile(fileName, editorConfig.value, externalEditors.value);
    return resolved.kind === "system" ? "" : resolved.editor.id;
  }
  function openWithSystemEntry(entry: TEntry) {
    void openInExternalEditor(entry, { kind: "system" });
  }
  function openWithKnownEntry(entry: TEntry, editor: KnownEditor) {
    void openInExternalEditor(entry, { kind: "known", editor });
  }
  function openWithCustomEntry(entry: TEntry, editor: CustomEditor) {
    void openInExternalEditor(entry, { kind: "custom", editor });
  }

  // 自定义编辑器命令弹窗状态。
  const customEditorOpen = ref(false);
  const customEditorEntry = ref<TEntry>();
  const customEditorName = ref("");
  const customEditorCommand = ref("");
  const customEditorRemember = ref(true);
  const customEditorPattern = computed(() =>
    customEditorEntry.value ? defaultPatternFor(customEditorEntry.value.name) : "*",
  );
  function beginCustomEditorCommand(entry: TEntry) {
    customEditorEntry.value = entry;
    customEditorName.value = "";
    customEditorCommand.value = "";
    customEditorRemember.value = true;
    customEditorOpen.value = true;
  }
  async function commitCustomEditor() {
    const entry = customEditorEntry.value;
    const command = customEditorCommand.value.trim();
    if (!entry || !command) return;
    const id = newCustomEditorId();
    const name = customEditorName.value.trim() || command.split(/\s+/)[0] || id;
    const next = sanitizeEditorConfig(editorConfig.value);
    next.customEditors = [...next.customEditors, { id, name, command }];
    if (customEditorRemember.value) {
      const pattern = defaultPatternFor(entry.name);
      next.associations = [
        ...next.associations.filter((assoc) => assoc.pattern.toLowerCase() !== pattern.toLowerCase()),
        { pattern, customId: id },
      ];
    }
    updateEditorConfig(next);
    customEditorOpen.value = false;
    await openInExternalEditor(entry, { kind: "custom", editor: { id, name, command } });
  }

  return {
    availableExternalEditors,
    mappedEditorIdFor,
    openWithSystemEntry,
    openWithKnownEntry,
    openWithCustomEntry,
    customEditorOpen,
    customEditorEntry,
    customEditorName,
    customEditorCommand,
    customEditorRemember,
    customEditorPattern,
    beginCustomEditorCommand,
    commitCustomEditor,
  };
}
