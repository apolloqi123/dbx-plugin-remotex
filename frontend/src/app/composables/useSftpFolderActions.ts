import { type Ref } from "vue";
import { findFileTreeNode, applyFileTreeChildren, markCollapsedTreeNodesStale, type SftpFileTreeNode } from "../../features/sftp/lib/sftpFileTree";
import type { SftpSanitizedEntry } from "../../features/sftp/lib/sftpEntries";
import { normalizeRemotePath, pathFromUri } from "./sftpRemotePaths";

type FolderMenuEntry = { uri: string };

/**
 * SFTP 文件树右键菜单与上传后的目录刷新：
 * - 行右键的新建/上传落点 = 被右键的目录，而不是面板当前目录；
 * - 文件夹右键刷新：当前目录走整页 loadDirectory，树内其他目录重列子项并
 *   保持原展开态；上传完成后同样按目标目录增量刷新并清掉折叠缓存。
 *
 * 导航/树工作区仍由各自 composable 承载，这里经回调接线。
 */
export function useSftpFolderActions(options: {
  terminalMenuOpen: Ref<boolean>;
  fileMenu: Ref<{ entry: { uri: string }; selection: string[] } | undefined>;
  session: () => { sessionId: string } | undefined;
  currentPath: Ref<string>;
  sftpTreeRoot: Ref<SftpFileTreeNode>;
  sftpShowHidden: Ref<boolean>;
  mkdirTargetDir: Ref<string | undefined>;
  openMkdirDialog: (targetDir: string) => void;
  beginSymlinkCreate: (targetDir: string) => void;
  chooseUpload: (targetDir?: string) => void;
  chooseFolderUpload: (targetDir?: string) => void;
  openNewFileDialog: (targetDir?: string) => void;
  loadDirectory: () => Promise<void>;
  revealSftpPath: (path: string, opts?: { quiet?: boolean }) => Promise<void>;
  listSftpDirectory: (path: string) => Promise<SftpSanitizedEntry[]>;
  refreshExpandedSftpTree: () => Promise<void>;
  showError: (cause: unknown) => void;
}) {
  const {
    terminalMenuOpen, fileMenu, session, currentPath, sftpTreeRoot, sftpShowHidden,
    mkdirTargetDir, openMkdirDialog, beginSymlinkCreate,
    chooseUpload, chooseFolderUpload, openNewFileDialog, loadDirectory,
    revealSftpPath, listSftpDirectory, refreshExpandedSftpTree, showError,
  } = options;

  function onFileAreaContextMenu(event: MouseEvent) {
    if ((event.target as HTMLElement).closest(".sftp-tree-row")) return;
    event.preventDefault();
    terminalMenuOpen.value = false;
    fileMenu.value = undefined;
  }

  /** 文件夹行右键的新建/上传：落点就是被右键的目录，而不是面板当前目录。
   *  菜单已在动作触发前关闭，这里直接进入各对话框/文件选择器。 */
  function folderMenuAction(
    action: "mkdir" | "newFile" | "upload" | "uploadFolder" | "symlink",
    entry: FolderMenuEntry,
  ) {
    const targetDir = pathFromUri(entry.uri);
    fileMenu.value = undefined;
    if (action === "mkdir") {
      mkdirTargetDir.value = targetDir;
      openMkdirDialog(targetDir);
    } else if (action === "symlink") {
      beginSymlinkCreate(targetDir);
    } else if (action === "upload") {
      chooseUpload(targetDir);
    } else if (action === "uploadFolder") {
      chooseFolderUpload(targetDir);
    } else {
      openNewFileDialog(targetDir);
    }
  }

  /** 文件夹右键刷新：当前目录走整页 loadDirectory；树内其他目录重列子项并
   *  保持原展开态（折叠目录刷新缓存，展开目录原地更新，不强制展开）。 */
  async function refreshSftpFolder(entry: FolderMenuEntry) {
    fileMenu.value = undefined;
    if (!session()) return;
    const path = pathFromUri(entry.uri);
    if (path === currentPath.value) {
      await loadDirectory();
      return;
    }
    const node = findFileTreeNode(sftpTreeRoot.value, path);
    if (!node || node.kind !== "directory") {
      await revealSftpPath(path, { quiet: true });
      return;
    }
    const wasExpanded = node.expanded;
    node.loading = true;
    try {
      const listing = await listSftpDirectory(path);
      const applied = applyFileTreeChildren(sftpTreeRoot.value, path, listing, sftpShowHidden.value);
      if (applied) applied.expanded = wasExpanded;
    } catch (cause) {
      showError(cause);
    } finally {
      node.loading = false;
    }
  }

  /** 上传完成后的目标目录刷新：当前目录走整页列表，树内目录重列子项并保留
   *  原展开态；随后刷新所有已展开分支与折叠缓存，父目录立即可见新结果。 */
  async function refreshSftpAfterUpload(targetDir?: string): Promise<void> {
    if (!session()) return;
    const target = normalizeRemotePath(targetDir || currentPath.value);
    if (target === currentPath.value) {
      await loadDirectory();
    } else {
      const node = findFileTreeNode(sftpTreeRoot.value, target);
      if (node && node.kind === "directory") {
        const wasExpanded = node.expanded;
        node.loading = true;
        try {
          const listing = await listSftpDirectory(target);
          const applied = applyFileTreeChildren(sftpTreeRoot.value, target, listing, sftpShowHidden.value);
          if (applied) applied.expanded = wasExpanded;
        } catch {
          // 目标目录暂时不可读时保留旧列表，全局错误条已由上传链给出。
        } finally {
          node.loading = false;
        }
      } else {
        await revealSftpPath(target, { quiet: true });
      }
    }
    markCollapsedTreeNodesStale(sftpTreeRoot.value);
    await refreshExpandedSftpTree();
  }

  return {
    onFileAreaContextMenu,
    folderMenuAction,
    refreshSftpFolder,
    refreshSftpAfterUpload,
  };
}
