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
} from "../../lib/paths/sftpBookmarks";

/**
 * SFTP 路径书签（存储迁移批 1）：权威在 pluginStore（宿主 ui-storage.json，
 * 随 DBX secrets 同步加密上云）；sidecar sftp/bookmarks/list 仅作首次拉取的
 * 一次性搬迁种子。星标收藏 + 书签/快捷路径弹层跳转/删除。
 */
export function useSftpBookmarks(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  connected: Ref<boolean>;
  currentPath: Ref<string>;
  closeToolbarPopovers: () => void;
}) {
  const { t, showNotice, showError, connected, currentPath, closeToolbarPopovers } = options;

  const sftpBookmarks = ref<SftpBookmark[]>([]);
  const bookmarksOpen = ref(false);
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

  /** 单按钮管理弹层（2026-10-29 重做）：打开即进入「收藏当前路径」准备态
   *  ——名称预填路径末段，列表刷新兜底；收藏/删除都在同一弹层内完成，
   *  不再有独立的「收藏此路径」按钮与二次弹层（两个图标语义重复）。 */
  function toggleBookmarksMenu() {
    const next = !bookmarksOpen.value;
    closeToolbarPopovers();
    if (next) bookmarkLabelDraft.value = defaultBookmarkLabel(currentPath.value);
    bookmarksOpen.value = next;
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
      // 保持弹层打开：新条目立刻可见，输入框回到当前路径的默认名称。
      bookmarkLabelDraft.value = defaultBookmarkLabel(currentPath.value);
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

  // 连接建立后拉取书签（全局共享，不随会话清空）；打开书签弹层时刷新兜底。
  watch(connected, (value) => {
    if (value) void refreshBookmarks();
  });
  watch(bookmarksOpen, (open) => {
    if (open) void refreshBookmarks();
  });
  // 弹层打开期间路径变了（面包屑回跳/跟随终端）：名称仍是上一个路径的默认值时
  // 跟着刷新，用户已改过的名称不覆盖。
  watch(currentPath, (next, previous) => {
    if (!bookmarksOpen.value || bookmarkSaving.value) return;
    if (bookmarkLabelDraft.value === defaultBookmarkLabel(previous)) bookmarkLabelDraft.value = defaultBookmarkLabel(next);
  });

  return {
    sftpBookmarks,
    bookmarksOpen,
    toggleBookmarksMenu,
    bookmarkSaving,
    bookmarkLabelDraft,
    refreshBookmarks,
    confirmBookmarkSave,
    removeBookmark,
  };
}
