import { ref, type Ref } from "vue";
import type { SftpFileTreeNode } from "../../lib/tree/sftpFileTree";
import { useUploadChain } from "./useUploadChain";

/** 上传链选项由 composables/transfer/useUploadChain 定义；本模块只接管拖拽
 *  高亮/宿主拖放落点两项面板状态，其余入口（选择器/剪贴板/文件夹/终端拖拽/
 *  冲突策略）原样透传，不复制实现。 */
type UploadChainOptions = Parameters<typeof useUploadChain>[0];

/** 本机文件拖入 SFTP 面板：dragenter/dragover/dragleave/drop、落点目录解析、
 *  冲突策略提示与上传链调用。树行 drop 的目标目录（文件夹行=该目录，文件行
 *  回落当前目录）也在这里裁决。 */
export function useSftpDropUpload(options: Omit<UploadChainOptions, "dragActive"> & {
  /** 右侧栏当前确实展示 SFTP 视图：树行 drop 只在面板可见时接收。 */
  sftpViewActive: Ref<boolean>;
}) {
  const { sftpViewActive, ...chainOptions } = options;
  const dragActive = ref(false);
  // 宿主桥接管 OS drop 时没有目标路径：SftpTreePane 把当前悬停的树行目录
  // 上抛到这里，作为 host onDrop 的上传落点。
  const sftpDragDropTarget = ref("");

  const chain = useUploadChain({ ...chainOptions, dragActive });

  /** 树行 drop：文件夹行为目标目录；文件行回落当前目录（行本身不是落点）。
   *  事件必须同步交给上传链，webkitGetAsEntry 的 dataTransfer.items 只在
   *  当前 tick 内有效。 */
  function onSftpTreeDrop(node: SftpFileTreeNode, event: DragEvent) {
    if (!sftpViewActive.value) return;
    chain.onDrop(event, node.kind === "directory" ? node.path : options.currentPath.value);
  }

  return {
    dragActive,
    sftpDragDropTarget,
    onSftpTreeDrop,
    ...chain,
  };
}
