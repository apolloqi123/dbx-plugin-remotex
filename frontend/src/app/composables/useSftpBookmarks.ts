import { ref, watch, type Ref } from "vue";
import {
  defaultBookmarkLabel,
  listBookmarks,
  loadSftpBookmarksFromStore,
  persistSftpBookmarks,
  removeBookmark as removeBookmarkFromList,
  SFTP_BOOKMARKS_LIMIT,
  upsertBookmark,
  validateBookmarkInput,
  type SftpBookmark,
} from "../../features/sftp/lib/sftpBookmarks";

/**
 * SFTP 路径书签（存储迁移批 1）：权威在 pluginStore（宿主 ui-storage.json，
 * 随 DBX secrets 同步加密上云）；sidecar sftp/bookmarks/list 仅作首次拉取的
 * 一次性搬迁种子。星标收藏 + 路径弹层跳转/删除。
 */
export function useSftpBookmarks(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  connected: Ref<boolean>;
  currentPath: Ref<string>;
  pathHistoryOpen: Ref<boolean>;
  closeToolbarPopovers: () => void;
}) {
  const { t, showNotice, showError, connected, currentPath, pathHistoryOpen, closeToolbarPopovers } = options;

  const sftpBookmarks = ref<SftpBookmark[]>([]);
  const bookmarkSaveOpen = ref(false);
  const bookmarkSaving = ref(false);
  const bookmarkLabelDraft = ref("");

  async function refreshBookmarks() {
    const stored = loadSftpBookmarksFromStore();
    if (stored !== null) {
      sftpBookmarks.value = stored;
      return;
    }
    try {
      // 键缺失 = 未迁移：sidecar 清单（含空表）一次性搬入并落键。
      sftpBookmarks.value = persistSftpBookmarks(await listBookmarks());
    } catch {
      // 后端未升级/读取失败时保留既有列表（optional 特性静默降级，不阻塞路径栏）。
    }
  }

  function toggleBookmarkSave() {
    if (!connected.value) return;
    if (bookmarkSaveOpen.value) {
      bookmarkSaveOpen.value = false;
      return;
    }
    closeToolbarPopovers();
    bookmarkLabelDraft.value = defaultBookmarkLabel(currentPath.value);
    bookmarkSaveOpen.value = true;
  }

  async function confirmBookmarkSave() {
    if (!connected.value || bookmarkSaving.value) return;
    const input = { label: bookmarkLabelDraft.value, path: currentPath.value };
    // 前端先行校验（与后端同规则）：label 空/超长/重复、path 空/超长、超上限。
    const localError = validateBookmarkInput(input, sftpBookmarks.value);
    if (localError) {
      showNotice(t(`sftpBookmark.error.${localError}`, localError === "limitReached" ? { limit: SFTP_BOOKMARKS_LIMIT } : {}));
      return;
    }
    bookmarkSaving.value = true;
    try {
      const { bookmarks, bookmark } = upsertBookmark(sftpBookmarks.value, input);
      sftpBookmarks.value = persistSftpBookmarks(bookmarks);
      bookmarkSaveOpen.value = false;
      showNotice(t("sftpBookmark.saved", { label: bookmark.label }));
    } catch (cause) {
      showError(cause);
    } finally {
      bookmarkSaving.value = false;
    }
  }

  async function removeBookmark(bookmark: SftpBookmark) {
    sftpBookmarks.value = persistSftpBookmarks(removeBookmarkFromList(sftpBookmarks.value, bookmark.id));
    showNotice(t("sftpBookmark.deleted"));
  }

  // 连接建立后拉取书签（全局共享，不随会话清空）；打开路径弹层时刷新兜底。
  watch(connected, (value) => {
    if (value) void refreshBookmarks();
  });
  watch(pathHistoryOpen, (open) => {
    if (open) void refreshBookmarks();
  });

  return {
    sftpBookmarks,
    bookmarkSaveOpen,
    bookmarkSaving,
    bookmarkLabelDraft,
    refreshBookmarks,
    toggleBookmarkSave,
    confirmBookmarkSave,
    removeBookmark,
  };
}
