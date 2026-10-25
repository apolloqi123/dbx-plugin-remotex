import { ref, type Ref } from "vue";
import type { SftpSanitizedEntry } from "../../lib/entries/sftpEntries";

/** 服务器内复制/剪切/粘贴的剪贴板：仅当前连接内有效。 */
export interface SftpClipboard {
  mode: "copy" | "cut";
  paths: string[];
  connectionId: string;
}

/** SFTP 面板对话框态：删除/批量删除、新建目录/新建文件落点、面板内剪贴板
 *  与粘贴忙碌、错误横幅 key。删除/批量删除/新建目录的动作本体仍在
 *  useSftpDelete/useSftpCreateDirectory（App 侧接线），本模块只持有状态。 */
export function useSftpDialogs(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  /** 错误横幅 key 由 useNotices 持有：经本模块聚合透出，App 解构口不变。 */
  sftpErrorKey: Ref<number>;
  getConnected: () => boolean;
  getCanWrite: () => boolean;
  getSudoMode: () => boolean;
  getCurrentPath: () => string;
  joinRemote: (parent: string, name: string) => string;
  /** 新建文件后的目标目录刷新（树内目录需重列子项）。 */
  refreshAfterUpload: (targetDir?: string) => Promise<void>;
}) {
  const { t, showNotice, showError, session, joinRemote } = options;

  const deleteTarget = ref<SftpSanitizedEntry>();
  const deleteSubmitting = ref(false);
  // 文件夹行右键菜单里的新建/上传落点：目录右键命中的路径，操作完成后清空。
  const mkdirTargetDir = ref<string>();
  const newFileTargetDir = ref<string>();
  const newFileDialog = ref(false);
  const newFileDraft = ref("");
  const newFileSubmitting = ref(false);
  const batchDeleteOpen = ref(false);
  const batchDeleteSubmitting = ref(false);
  const sftpClipboard = ref<SftpClipboard>();
  const pasteBusy = ref(false);

  function openNewFileDialog(targetDir?: string) {
    if (!options.getConnected() || !options.getCanWrite()) return;
    newFileTargetDir.value = targetDir;
    newFileDraft.value = "";
    newFileDialog.value = true;
  }

  async function createNewFile() {
    const sessionId = session.value?.sessionId;
    const name = newFileDraft.value.trim();
    if (!sessionId || !name || newFileSubmitting.value) return;
    const base = newFileTargetDir.value ?? options.getCurrentPath();
    newFileSubmitting.value = true;
    try {
      await window.dbxPlugin.invoke(options.getSudoMode() ? "sudo/touch" : "sftp/touch", {
        sessionId,
        path: joinRemote(base, name),
      });
      newFileDialog.value = false;
      newFileTargetDir.value = undefined;
      showNotice(t("sftpNewFile.done", { name }));
      await options.refreshAfterUpload(base);
    } catch (cause) {
      showError(cause);
    } finally {
      newFileSubmitting.value = false;
    }
  }

  return {
    deleteTarget,
    deleteSubmitting,
    batchDeleteOpen,
    batchDeleteSubmitting,
    mkdirTargetDir,
    newFileTargetDir,
    newFileDialog,
    newFileDraft,
    newFileSubmitting,
    sftpClipboard,
    pasteBusy,
    sftpErrorKey: options.sftpErrorKey,
    openNewFileDialog,
    createNewFile,
  };
}
