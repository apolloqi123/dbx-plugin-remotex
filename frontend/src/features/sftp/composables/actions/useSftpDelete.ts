import type { Ref } from "vue";
import type { SftpEntryKind } from "../../lib/entries/sftpEntries";
import { advanceBatchProgress, createBatchProgress, type BatchProgressState } from "../../../transfers/lib/sftpBatchProgress";

// 列表条目的最小结构（App.vue 的 SftpEntry 为局部接口，按消费字段收敛）。
type DeleteEntry = { uri: string; name: string; kind: SftpEntryKind };
// 批量删除一次 RPC 的返回：逐项归因，未删除项进 failed。
type BatchDeleteResult = { deleted: string[]; failed: Array<{ path: string; error: string }> };
/** 整批删除走一次远端 rm -rf：大目录树可能较久，给 10 分钟预算。 */
const DELETE_BATCH_TIMEOUT_MS = 10 * 60 * 1000;
// 批次进度状态（App.vue 的 batchProgress 使用的结构，lib/sftpBatchProgress

/** SFTP 删除与批量操作：单条删除（sudo 通道分流）、多选批量删除、批量归档
 * （sftp/archive 逐项 + 批次进度）。batchProgress 批次进度为外部编辑/文件夹
 * 上传共用，仍由 App.vue 持有；clearRowSelection/archiveBusy 分别来自
 * useSftpSelectionClipboard/useSftpArchive（晚于本域调用点），以惰性注入。 */
export function useSftpDelete(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  sudoMode: Ref<boolean>;
  currentPath: Ref<string>;
  deleteTarget: Ref<DeleteEntry | undefined>;
  deleteSubmitting: Ref<boolean>;
  batchDeleteOpen: Ref<boolean>;
  batchDeleteSubmitting: Ref<boolean>;
  batchProgress: Ref<BatchProgressState | null>;
  selectedEntries: Ref<DeleteEntry[]>;
  pathFromUri: (uri: string) => string;
  joinRemote: (parent: string, name: string) => string;
  loadDirectory: (path?: string) => Promise<void>;
  clearRowSelection: () => void;
  archiveBusy: Ref<boolean>;
  /** 已确认删除后回调实际删掉的远端路径（失败批次只含已成功项），App 可据
   *  此关闭对应编辑器页签。 */
  onDeleted?: (paths: string[]) => void;
  /** 删除/批量删除成功或失败后的统一刷新入口（App 可借此刷新所有展开的树
   *  节点）；deletedPaths 为后端确认删除的路径（本地对账用），失败/异常时
   *  缺省，调用方回退全量刷新。缺省回退 loadDirectory() 保持旧调用方行为。 */
  onMutated?: (deletedPaths?: string[]) => void | Promise<void>;
}) {
  const { t, showNotice, showError, session, sudoMode, currentPath, deleteTarget, deleteSubmitting, batchDeleteOpen, batchDeleteSubmitting, batchProgress, selectedEntries, pathFromUri, joinRemote, loadDirectory, clearRowSelection, archiveBusy, onDeleted, onMutated } = options;

/** 变更后刷新：优先外部 onMutated（转发已删除路径供本地对账；失败/异常无
 *  可信清单时零参数调用，走全量重列），否则回退 loadDirectory()；刷新失败
 *  只报错，不把已完成的删除当作失败。 */
async function refreshAfterMutation(deletedPaths?: string[]) {
  try {
    if (!onMutated) {
      await loadDirectory();
      return;
    }
    if (deletedPaths?.length) await onMutated(deletedPaths);
    else await onMutated();
  } catch (cause) {
    showError(cause);
  }
}

async function confirmDelete() {
  if (!session.value || !deleteTarget.value) return;
  deleteSubmitting.value = true;
  let deletedPath = "";
  try {
    try {
      const path = pathFromUri(deleteTarget.value.uri);
      if (sudoMode.value) {
        await window.dbxPlugin.invoke(deleteTarget.value.kind === "directory" ? "sudo/removeAll" : "sudo/remove", {
          sessionId: session.value.sessionId,
          path,
        });
      } else {
        await window.dbxPlugin.invoke("sftp/delete", {
          sessionId: session.value.sessionId,
          path,
          recursive: deleteTarget.value.kind === "directory",
        });
      }
      deletedPath = path;
      deleteTarget.value = undefined;
      showNotice(t("deleted"));
    } catch (cause) {
      showError(cause);
    }
    // 单条删除失败时传空数组（未删任何路径），调用方按空列表无操作；刷新
    // 不带路径（失败没有可信删除清单），走全量重列。
    onDeleted?.(deletedPath ? [deletedPath] : []);
    await refreshAfterMutation(deletedPath ? [deletedPath] : undefined);
  } finally {
    deleteSubmitting.value = false;
  }
}

async function confirmBatchDelete() {
  const sessionId = session.value?.sessionId;
  const targets = selectedEntries.value;
  if (!sessionId || !targets.length || batchDeleteSubmitting.value) return;
  batchDeleteSubmitting.value = true;
  batchProgress.value = createBatchProgress(targets.length);
  // 可信删除清单（后端返回）：仅成功/部分失败时本地对账，RPC 异常走全量刷新。
  let deletedPaths: string[] | undefined;
  try {
    try {
      // 一次 RPC 完成整批（远端按 128 项分片 `rm -rf`），不再逐项
      // invoke + 逐项 SFTP 往返；sudo 车道同样一次认证跑完整批。
      const items = targets.map((entry) => ({
        path: pathFromUri(entry.uri),
        recursive: entry.kind === "directory",
      }));
      const result = sudoMode.value
        ? await window.dbxPlugin.invoke<BatchDeleteResult>(
            "sudo/removeMany",
            { sessionId, paths: items.map((item) => item.path) },
            { timeoutMs: DELETE_BATCH_TIMEOUT_MS },
          )
        : await window.dbxPlugin.invoke<BatchDeleteResult>(
            "sftp/deleteMany",
            { sessionId, items },
            { timeoutMs: DELETE_BATCH_TIMEOUT_MS },
          );
      batchProgress.value = {
        total: targets.length,
        done: result.deleted.length,
        failed: result.failed.length,
        current: "",
      };
      if (result.failed.length > 0) {
        // 部分失败：保留对话框（未删除项仍在列表里），报第一项「路径: 错误」。
        showError(`${result.failed[0].path}: ${result.failed[0].error}`);
      } else {
        batchDeleteOpen.value = false;
        clearRowSelection();
        showNotice(t("deleted"));
      }
      // 只关闭确实删掉的编辑器页签：失败项不传。
      onDeleted?.(result.deleted);
      deletedPaths = result.deleted.length ? result.deleted : undefined;
    } catch (cause) {
      showError(cause);
      onDeleted?.([]);
    }
    await refreshAfterMutation(deletedPaths);
  } finally {
    batchDeleteSubmitting.value = false;
    batchProgress.value = null;
  }
}

async function batchArchive() {
  const sessionId = session.value?.sessionId;
  const targets = selectedEntries.value;
  if (!sessionId || !targets.length || archiveBusy.value) return;
  archiveBusy.value = true;
  let progress = createBatchProgress(targets.length);
  batchProgress.value = progress;
  try {
    let done = 0;
    for (const entry of targets) {
      const archiveName = `${entry.name}.tar.gz`;
      try {
        await window.dbxPlugin.invoke("sftp/archive", {
          sessionId,
          sourcePaths: [pathFromUri(entry.uri)],
          archivePath: joinRemote(currentPath.value, archiveName),
        }, { timeoutMs: 30 * 60 * 1000 });
        progress = advanceBatchProgress(progress, { name: archiveName, ok: true });
      } catch (cause) {
        progress = advanceBatchProgress(progress, { name: archiveName, ok: false });
        throw cause;
      }
      batchProgress.value = progress;
      done += 1;
    }
    showNotice(t("sftpBatch.archiveDone", { count: done }));
    await loadDirectory();
  } catch (cause) {
    showError(cause);
    await loadDirectory();
  } finally {
    archiveBusy.value = false;
    batchProgress.value = null;
  }
}


  return {
    confirmDelete,
    confirmBatchDelete,
    batchArchive,
  };
}
