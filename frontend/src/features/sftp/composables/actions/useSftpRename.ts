import { nextTick, ref, type Ref } from "vue";
import { confirmDialog } from "../../../../shared/lib/confirmDialog";
import { shouldCommitRename } from "../../lib/actions/sftpRename";

/** 重命名目标路径：父目录取自条目自身的绝对路径（树中可展开多个目录，非当前
 *  目录的文件不能用 currentPath 拼目标）。sourcePath 为 pathFromUri 的结果。 */
export function renameTargetPath(
  sourcePath: string,
  name: string,
  joinRemote: (parent: string, name: string) => string,
): string {
  const normalized = sourcePath.replace(/\/+$/, "");
  const index = normalized.lastIndexOf("/");
  const parent = index <= 0 ? "/" : normalized.slice(0, index);
  return joinRemote(parent, name);
}

/** SFTP 行内重命名：blur 兜底提交（R3-P1-2/R3-P2-1 短路语义）、目标存在性
 * 预检 + 覆盖确认（R3-P2-2）、sudo 通道分流；失败收敛为关闭编辑态并刷新。 */
export function useSftpRename(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  canWrite: Ref<boolean>;
  sudoMode: Ref<boolean>;
  selectedPath: Ref<string>;
  pathFromUri: (uri: string) => string;
  joinRemote: (parent: string, name: string) => string;
  loadDirectory: (path?: string) => Promise<void>;
  /** 成功后同步编辑器页签路径（旧路径 → 新路径），随后触发 onMutated。 */
  onRenamed?: (oldPath: string, newPath: string) => void;
  /** 重命名成功/失败后的统一刷新入口（App 可借此刷新所有展开的树节点）；
   *  缺省回退 loadDirectory() 保持旧调用方行为。 */
  onMutated?: () => void | Promise<void>;
}) {
  const { t, showError, session, canWrite, sudoMode, selectedPath, pathFromUri, joinRemote, loadDirectory, onRenamed, onMutated } = options;

// 目录条目的最小结构（App.vue 的 SftpEntry 为局部接口，按消费字段收敛）。
type RenameEntry = { uri: string; name: string };

const renamingPath = ref("");
const renameDraft = ref("");
const renameSubmitting = ref(false);

function beginRename(entry: RenameEntry) {
  if (!canWrite.value) return;
  selectedPath.value = entry.uri;
  renamingPath.value = entry.uri;
  renameDraft.value = entry.name;
  void nextTick(() => document.querySelector<HTMLInputElement>(".rename-input")?.select());
}

/** 变更后刷新：优先外部 onMutated（刷新所有已展开树节点等），否则回退
 *  loadDirectory()；刷新失败只报错，不把已成功的重命名当作失败。 */
async function refreshAfterMutation() {
  try {
    if (onMutated) await onMutated();
    else await loadDirectory();
  } catch (cause) {
    showError(cause);
  }
}

async function commitRename(entry: RenameEntry) {
  // R3-P1-2 / R3-P2-1：blur 是"卸载/失焦"兜底提交入口。Esc 取消会先清
  // renamingPath 再卸载输入框，Enter 提交成功后也会清空——两种场景下
  // editingPath 已不指向本行，blur 到达时被 shouldCommitRename 短路，
  // 取消语义不再以草稿名逃逸提交、Enter 也不再双发。
  if (!shouldCommitRename({ editingPath: renamingPath.value, entryUri: entry.uri, submitting: renameSubmitting.value })) return;
  const name = renameDraft.value.trim();
  if (!session.value || !name || name === entry.name) {
    renamingPath.value = "";
    return;
  }
  const sourcePath = pathFromUri(entry.uri);
  const targetPath = renameTargetPath(sourcePath, name, joinRemote);
  renameSubmitting.value = true;
  try {
    // R3-P2-2：与粘贴对齐的目标存在性预检。OpenSSH rename 撞名语义依
    // posix-rename 扩展而异，前端先给出明确的覆盖确认；预检失败不阻断，
    // 交由后端执行时报错。
    let targetExists = false;
    try {
      const probe = await window.dbxPlugin.invoke<{ exists: boolean }>("sftp/exists", {
        sessionId: session.value.sessionId,
        path: targetPath,
      });
      targetExists = probe.exists === true;
    } catch {
      // 预检不可用时保持原语义直接下发。
    }
    if (targetExists && !(await confirmDialog(t("sftpRename.overwriteConfirm", { name })))) {

      renamingPath.value = "";
      return;
    }
    if (sudoMode.value) {
      await window.dbxPlugin.invoke("sudo/rename", { sessionId: session.value.sessionId, sourcePath, targetPath });
    } else {
      await window.dbxPlugin.invoke("sftp/rename", { sessionId: session.value.sessionId, sourcePath, targetPath });
    }
    renamingPath.value = "";
    // 先同步编辑器页签路径（旧的 sourcePath → targetPath），再刷新树/列表。
    onRenamed?.(sourcePath, targetPath);
    await refreshAfterMutation();
  } catch (cause) {
    // R3-P2-2：失败路径收敛——关闭行内编辑态并刷新列表，不再滞留打开态。
    renamingPath.value = "";
    showError(cause);
    await refreshAfterMutation();
  } finally {
    renameSubmitting.value = false;
  }
}


  return {
    renamingPath,
    renameDraft,
    renameSubmitting,
    beginRename,
    commitRename,
  };
}
