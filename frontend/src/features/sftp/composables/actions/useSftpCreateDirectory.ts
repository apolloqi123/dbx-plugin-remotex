import { ref, type Ref } from "vue";

/** 新建目录对话框（mkdir；sudo 会话走 sudo/mkdir 通道）。 */
export function useSftpCreateDirectory(options: {
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  sudoMode: Ref<boolean>;
  currentPath: Ref<string>;
  /** 文件夹行右键入口的落点；未设置时使用面板当前目录。成功后清空。 */
  targetDir?: Ref<string | undefined>;
  joinRemote: (parent: string, name: string) => string;
  loadDirectory: (path?: string) => Promise<void>;
  /** 目标目录刷新入口（树内目录需重列子项）；缺省回退 loadDirectory()。 */
  refreshAfterCreate?: (targetDir?: string) => Promise<void>;
}) {
  const { showError, session, sudoMode, currentPath, targetDir, joinRemote, loadDirectory, refreshAfterCreate } = options;

const operationDialog = ref<"mkdir" | null>(null);
const operationDraft = ref("");

async function createDirectory() {
  const name = operationDraft.value.trim();
  if (!session.value || !name) return;
  const base = targetDir?.value ?? currentPath.value;
  const path = joinRemote(base, name);
  try {
    if (sudoMode.value) {
      await window.dbxPlugin.invoke("sudo/mkdir", { sessionId: session.value.sessionId, path });
    } else {
      await window.dbxPlugin.invoke("sftp/createDirectory", { sessionId: session.value.sessionId, path });
    }
    operationDialog.value = null;
    if (targetDir) targetDir.value = undefined;
    if (refreshAfterCreate) await refreshAfterCreate(base);
    else await loadDirectory();
  } catch (cause) {
    showError(cause);
  }
}


  return {
    operationDialog,
    operationDraft,
    createDirectory,
  };
}
