import { ref, type Ref } from "vue";
import { useFilePreview } from "./useFilePreview";

/** 预览入口条目的最小结构（与 useFilePreview 的 openEntry 入参一致）。 */
type PreviewEntry = Parameters<ReturnType<typeof useFilePreview>["openEntry"]>[0];

/** SFTP 文件预览与保存：状态/打开前确认/编辑草稿/保存语义全部收口在
 *  composables/editor/useFilePreview，本模块只补「当前预览条目」锚点，
 *  供预览弹窗与后续动作读取（关闭即清空）。 */
export function useSftpPreview(options: Parameters<typeof useFilePreview>[0]) {
  const preview = useFilePreview(options);
  // 最近一次从面板打开的条目：预览内容与保存目标由 useFilePreview 持有，
  // 这里只保留入口条目本身（弹窗元数据/后续动作定位用）。
  const previewEntry = ref<PreviewEntry>() as Ref<PreviewEntry | undefined>;

  async function openEntry(entry: PreviewEntry) {
    previewEntry.value = entry;
    await preview.openEntry(entry);
  }

  async function closePreview() {
    previewEntry.value = undefined;
    return preview.closePreview();
  }

  return {
    ...preview,
    previewEntry,
    openEntry,
    closePreview,
  };
}
