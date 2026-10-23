// 工作台内编辑器标签页：终端上方按文件开页签，支持多文件、直接编辑与显式保存。
//
// 打开前检查沿用 useFilePreview 的既有语义（图片/已知二进制/大文件确认/内容
// 嗅探/截断只读），保存沿用同一条写入分流（sudo/writeFile vs sftp/write）。
// 与预览弹窗不同，状态按标签页隔离：每个页签独立持有正文、baseline、脏标记
// 与加载/错误/保存状态，互不影响。
import { computed, ref, watch, type ComputedRef, type Ref } from "vue";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { formatBytes } from "../../../shared/lib/format";
import { rpc } from "../../../shared/lib/rpc";
import { randomUUID } from "../../../shared/lib/uuid";
import { MIB } from "../../settings/lib/settingsModel";
import type { SftpEntryKind } from "../lib/sftpEntries";
import { looksBinary } from "../lib/textSniff";

/** SFTP 列表条目在编辑器侧的最小结构（按消费字段收敛，App 的 SftpEntry 可直接传入）。 */
export interface EditorEntry {
  uri: string;
  name: string;
  kind: SftpEntryKind;
  size?: number;
}

/** 一个编辑器页签的完整状态（正文/基线/脏标记/只读/截断/加载/保存/错误）。 */
export interface EditorTab {
  id: string;
  path: string;
  name: string;
  kind: "text" | "image";
  text: string;
  baseline: string;
  dirty: boolean;
  editable: boolean;
  truncated: boolean;
  loading: boolean;
  saving: boolean;
  size: number;
  imageUrl?: string;
  error?: string;
}

export const IMAGE_MIME_BY_EXTENSION: Record<string, string> = {
  png: "png",
  jpg: "jpeg",
  jpeg: "jpeg",
  gif: "gif",
  webp: "webp",
  svg: "svg+xml",
  bmp: "bmp",
  ico: "x-icon",
};
// 已知二进制扩展名直接拒绝打开；无后缀/改名文件由内容嗅探兜底。
export const BINARY_PREVIEW_EXTENSIONS = new Set([
  "7z", "bin", "bz2", "class", "dll", "dmg", "dylib", "exe", "gz", "iso",
  "jar", "lz4", "o", "obj", "otf", "pdf", "pyc", "rar", "so", "tar", "tif",
  "tiff", "ttf", "war", "woff", "woff2", "xz", "zip", "zst",
]);
// 打开前先读该字节数做二进制嗅探（looksBinary），避免向编辑器灌入乱码。
export const SNIFF_CHUNK_BYTES = 8 * 1024;
export const MAX_INLINE_PREVIEW_BYTES = 1024 * 1024;
export const MAX_DIRECT_WRITE_BYTES = 4 * 1024 * 1024;
export const MAX_IMAGE_PREVIEW_BYTES = 20 * MIB;

interface RemoteReadResult {
  dataBase64: string;
  truncated?: boolean;
}

interface EditorTabsOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  sudoMode: Ref<boolean>;
  canWrite: Ref<boolean>;
  pathFromUri: (uri: string) => string;
  onSaved?: (tab: EditorTab) => void | Promise<void>;
}

function fileExtension(name: string) {
  return name.includes(".") ? name.split(".").pop()?.toLowerCase() || "" : "";
}

function imagePreviewExtension(name: string) {
  const extension = fileExtension(name);
  return extension in IMAGE_MIME_BY_EXTENSION ? extension : "";
}

function hasBinaryExtension(name: string) {
  return BINARY_PREVIEW_EXTENSIONS.has(fileExtension(name));
}

function errorMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}

/** 去掉末尾斜杠（根路径保留 "/"），空串返回空串用于调用方短路。 */
function stripTrailingSlash(path: string) {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed || (path.startsWith("/") ? "/" : "");
}

function pathBasename(path: string) {
  const trimmed = stripTrailingSlash(path);
  const index = trimmed.lastIndexOf("/");
  return index < 0 ? trimmed : trimmed.slice(index + 1) || "/";
}

/** 路径等于前缀或位于其目录下（前缀 "/" 覆盖全部绝对路径）。 */
function isPathUnder(path: string, prefix: string) {
  return path === prefix || path.startsWith(prefix === "/" ? "/" : `${prefix}/`);
}

/** 开启编辑器页签后，SFTP 文件打开/编辑/保存的状态机。 */
export function useEditorTabs(options: EditorTabsOptions) {
  const { t, showNotice, showError, session, sudoMode, canWrite, pathFromUri, onSaved } = options;

  const tabs = ref<EditorTab[]>([]);
  const activeId = ref("");
  const activeTab = computed(() => tabs.value.find((tab) => tab.id === activeId.value));
  const hasTabs = computed(() => tabs.value.length > 0);

  function activate(id: string) {
    if (tabs.value.some((tab) => tab.id === id)) activeId.value = id;
  }

  function removeTab(id: string) {
    const index = tabs.value.findIndex((tab) => tab.id === id);
    if (index < 0) return;
    tabs.value.splice(index, 1);
    // 关闭当前页签后优先落到右侧相邻页签，没有则落到左侧；全部关闭则清空 active。
    if (activeId.value === id) {
      const next = tabs.value[index] ?? tabs.value[index - 1];
      activeId.value = next?.id ?? "";
    }
  }

  // 返回数组里的响应式代理：直接改 push 进去的裸对象不触发依赖更新，页签会
  // 永远停在 loading（浏览器实测）。调用方必须持有返回值做后续异步写入。
  function pushTab(tab: EditorTab): EditorTab {
    tabs.value.push(tab);
    const stored = tabs.value[tabs.value.length - 1]!;
    activeId.value = stored.id;
    return stored;
  }

  /** 远端重命名成功后同步页签路径：命中 oldPath 本身或其子路径的页签改路径与
   *  文件名（正文/脏标记保留）；重命名失败不会调用。 */
  function handleRename(oldPath: string, newPath: string): void {
    const source = stripTrailingSlash(oldPath);
    const target = stripTrailingSlash(newPath);
    if (!source || !target || source === target) return;
    for (const tab of tabs.value) {
      if (!isPathUnder(tab.path, source)) continue;
      const suffix = tab.path.slice(source.length);
      tab.path = suffix ? `${target === "/" ? "" : target}${suffix}` : target;
      tab.name = pathBasename(tab.path);
    }
  }

  /** 删除已确认后关闭相关页签：不弹脏确认（调用方已确认删除），当前页签被
   *  移除时落到删除区间右侧/左侧的幸存页签。 */
  function closeTabsUnder(path: string): void {
    const target = stripTrailingSlash(path);
    if (!target) return;
    const firstIndex = tabs.value.findIndex((tab) => isPathUnder(tab.path, target));
    if (firstIndex < 0) return;
    const activeRemoved = tabs.value.some((tab) => tab.id === activeId.value && isPathUnder(tab.path, target));
    tabs.value = tabs.value.filter((tab) => !isPathUnder(tab.path, target));
    if (activeRemoved) {
      const next = tabs.value[firstIndex] ?? tabs.value[firstIndex - 1];
      activeId.value = next?.id ?? "";
    }
  }

  function createTab(entry: EditorEntry, kind: "text" | "image", overrides: Partial<EditorTab> = {}): EditorTab {
    return {
      id: randomUUID(),
      path: pathFromUri(entry.uri),
      name: entry.name,
      kind,
      text: "",
      baseline: "",
      dirty: false,
      editable: false,
      truncated: false,
      loading: true,
      saving: false,
      size: entry.size || 0,
      ...overrides,
    };
  }

  function isImagePreviewable(entry: EditorEntry) {
    const size = entry.size || 0;
    return !!imagePreviewExtension(entry.name) && size > 0 && size <= MAX_IMAGE_PREVIEW_BYTES;
  }

  // 嗅探失败不拦截：交给正式读取的 sudo 分流/错误态处理。
  async function remoteFileLooksBinary(entry: EditorEntry) {
    const sessionId = session.value?.sessionId;
    if (!sessionId) return false;
    const path = pathFromUri(entry.uri);
    try {
      const result = sudoMode.value
        ? await rpc<RemoteReadResult>("sudo/readFile", {
            sessionId,
            path,
            offset: 0,
            length: SNIFF_CHUNK_BYTES,
          })
        : await rpc<RemoteReadResult>("sftp/read", {
            sessionId,
            path,
            maxBytes: SNIFF_CHUNK_BYTES,
          });
      return looksBinary(window.dbxPlugin.decodeBase64(result.dataBase64));
    } catch {
      return false;
    }
  }

  async function openImageTab(entry: EditorEntry, mime: string) {
    const path = pathFromUri(entry.uri);
    const tab = pushTab(createTab(entry, "image", { editable: false }));
    try {
      const result = sudoMode.value
        ? await rpc<RemoteReadResult>("sudo/readFile", {
            sessionId: session.value?.sessionId,
            path,
            offset: 0,
            length: MAX_IMAGE_PREVIEW_BYTES,
          })
        : await rpc<RemoteReadResult>("sftp/read", {
            sessionId: session.value?.sessionId,
            path,
            maxBytes: MAX_IMAGE_PREVIEW_BYTES,
          });
      if (result.truncated) {
        // 图片超过内联上限：不开只显示上半截的页签，给出说明后返回。
        removeTab(tab.id);
        showNotice(t("previewDialog.truncated", {
          limit: formatBytes(MAX_IMAGE_PREVIEW_BYTES),
          size: formatBytes(tab.size),
        }));
        return;
      }
      tab.imageUrl = `data:image/${mime};base64,${result.dataBase64}`;
    } catch (cause) {
      // 保留页签展示错误，用户可直接关闭；不额外弹全局横幅。
      tab.error = errorMessage(cause);
    } finally {
      tab.loading = false;
    }
  }

  async function openEntry(entry: EditorEntry) {
    const path = pathFromUri(entry.uri);
    const existing = tabs.value.find((tab) => tab.path === path);
    if (existing) {
      activeId.value = existing.id;
      return;
    }
    // 目录由 App 负责导航；symbolic link / other 与预览一致不进入编辑器。
    if (entry.kind === "directory") return;
    if (entry.kind !== "file") return;

    const size = entry.size || 0;
    if (isImagePreviewable(entry)) {
      await openImageTab(entry, IMAGE_MIME_BY_EXTENSION[imagePreviewExtension(entry.name)]);
      return;
    }
    if (hasBinaryExtension(entry.name)) {
      showNotice(t("binaryFile.notOpen", { name: entry.name }));
      return;
    }

    // 大文件先确认：确认后只加载头部且只读（保存会整文件覆写，丢未加载部分）。
    let truncated = false;
    if (size > MAX_INLINE_PREVIEW_BYTES) {
      const limit = formatBytes(MAX_INLINE_PREVIEW_BYTES);
      const accepted = await confirmDialog(
        t("previewDialog.tooLargeConfirm", { name: entry.name, size: formatBytes(size), limit }),
        { danger: false },
      );
      if (!accepted) return;
      truncated = true;
    }

    // 内容嗅探兜底：无后缀或改名的二进制文件（含 size 未知的条目）在开页签前拦下。
    if (await remoteFileLooksBinary(entry)) {
      showNotice(t("binaryFile.notOpen", { name: entry.name }));
      return;
    }

    // 先挂页签再读：编辑器正文区能展示 loading，失败时把错误落在该页签上。
    const tab = pushTab(createTab(entry, "text", {
      truncated,
      editable: canWrite.value && !truncated && size <= MAX_DIRECT_WRITE_BYTES,
    }));
    try {
      const result = sudoMode.value
        ? await rpc<RemoteReadResult>("sudo/readFile", {
            sessionId: session.value?.sessionId,
            path: tab.path,
            offset: 0,
            length: MAX_INLINE_PREVIEW_BYTES,
          })
        : await rpc<RemoteReadResult>("sftp/read", {
            sessionId: session.value?.sessionId,
            path: tab.path,
            maxBytes: MAX_INLINE_PREVIEW_BYTES,
          });
      tab.truncated = truncated || result.truncated === true;
      tab.text = new TextDecoder("utf-8", { fatal: false }).decode(window.dbxPlugin.decodeBase64(result.dataBase64));
      tab.baseline = tab.text;
      tab.editable = canWrite.value && !tab.truncated && tab.size <= MAX_DIRECT_WRITE_BYTES;
    } catch (cause) {
      // 可预测行为：页签保留并进入错误态（只读），不创建/移除来回跳动。
      tab.error = errorMessage(cause);
      tab.editable = false;
    } finally {
      tab.loading = false;
    }
  }

  function updateText(id: string, text: string) {
    const tab = tabs.value.find((candidate) => candidate.id === id);
    if (!tab || tab.kind !== "text") return;
    tab.text = text;
    tab.dirty = text !== tab.baseline;
  }

  async function saveTab(id: string): Promise<void> {
    const tab = tabs.value.find((candidate) => candidate.id === id);
    const sessionId = session.value?.sessionId;
    if (!tab || tab.kind !== "text" || !tab.editable || !sessionId || !canWrite.value || tab.saving) return;
    tab.saving = true;
    let written: { text: string; bytes: number } | undefined;
    try {
      // 捕获保存时的正文快照：保存期间继续输入时，baseline 只覆盖已写出的内容，
      // 脏标记按当前正文与快照比较而不是无条件清除。
      const text = tab.text;
      const bytes = new TextEncoder().encode(text);
      if (bytes.byteLength > MAX_DIRECT_WRITE_BYTES) {
        showError(new Error(t("sftpAttrs.sizeLimit")));
        return;
      }
      const dataBase64 = window.dbxPlugin.encodeBase64(bytes);
      if (sudoMode.value) {
        await rpc("sudo/writeFile", { sessionId, path: tab.path, dataBase64 });
      } else {
        await rpc("sftp/write", { sessionId, remotePath: tab.path, dataBase64 });
      }
      written = { text, bytes: bytes.byteLength };
    } catch (cause) {
      // 保存失败：保留脏标记，错误交全局错误条，页签可继续编辑/重试。
      showError(cause);
    } finally {
      tab.saving = false;
    }
    if (!written) return;
    tab.baseline = written.text;
    tab.dirty = tab.text !== written.text;
    tab.size = written.bytes;
    tab.error = undefined;
    showNotice(t("editSave.saved", { name: tab.name }));
    try {
      await onSaved?.(tab);
    } catch (cause) {
      // 保存已成功；回调（如刷新目录）失败单独报错，不回滚脏标记。
      showError(cause);
    }
  }

  async function saveActive() {
    if (activeId.value) await saveTab(activeId.value);
  }

  async function closeTab(id: string): Promise<void> {
    const tab = tabs.value.find((candidate) => candidate.id === id);
    if (!tab) return;
    if (tab.dirty && !(await confirmDialog(t("editSave.closeConfirm")))) return;
    removeTab(id);
  }

  async function closeActive() {
    if (activeId.value) await closeTab(activeId.value);
  }

  // 断连后 session 消失：旧页签的路径/正文已无对应连接，静默清空，避免
  // 重新连接后误保存到新会话。仅处理「从有 id 变为无 id」，初始未连接不触发。
  watch(
    () => session.value?.sessionId,
    (sessionId, previous) => {
      if (!sessionId && previous) {
        tabs.value = [];
        activeId.value = "";
      }
    },
  );

  return {
    tabs,
    activeId,
    activeTab: activeTab as ComputedRef<EditorTab | undefined>,
    hasTabs,
    openEntry,
    activate,
    updateText,
    handleRename,
    closeTabsUnder,
    saveTab,
    saveActive,
    closeTab,
    closeActive,
  };
}
