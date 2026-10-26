import { computed, reactive, ref, shallowRef } from "vue";
import {
  DEFAULT_COLUMN_WIDTHS,
  DEFAULT_VISIBLE_COLUMNS,
  type SftpColumn,
} from "../../../features/sftp/lib/entries/sftpEntries";
import type { SftpTypeFilter } from "../../../features/sftp/lib/entries/sftpFileFilters";
import type { BatchProgressState } from "../../../features/transfers/lib/sftpBatchProgress";

type RightPanelView = "sftp" | "metrics" | "docker";

// 列类型移到 lib/sftpEntries（issue #34：owner/group 属主/属组列，默认关）。
type SftpSortColumn = "name" | "size" | "modified";

/** 单栏文件树实例 API（SftpWorkspacePanel 经回调回填）。 */
export interface SftpTreePaneApi {
  scrollPathToTop(path: string): Promise<boolean>;
  markDropTargetAt(point: { x: number; y: number }): string;
  resetHostDragCalibration(): void;
}

// 路径弹层快捷路径（侧栏移除后仍保留在路径历史 popover 内）。
const SFTP_QUICK_PATHS = ["/", "/home", "/tmp", "/etc", "/var", "/root"];

/**
 * SFTP 域状态（Task 21b）：右侧栏视图开关、列表/过滤/列布局偏好、树实例句柄、
 * 下载落盘与批处理态。
 *
 * 纯本地工厂：列表本体/选中态/导航（entries/currentPath/selectedUris…）由
 * composables/listing/useSftpWorkspace 持有并经 App 解构，这里只保留声明。
 */
export function useSftpState(options: { initialPaneOpen: boolean }) {
  /** 右侧栏根 <section> 位于 WorkbenchSidePanel；该组件经 defineExpose 回传，
   *  WorkbenchWorkspace 用函数 ref 写入本 ref（模板 ref 不能跨组件自动绑定）。 */
  const sftpPane = shallowRef<HTMLElement>();
  // 目录跟随的"目标目录置顶"需要命令式滚动到树内指定路径。
  const sftpTreePane = ref<SftpTreePaneApi | null>(null);

  // 右侧栏可见性：SFTP / 指标 / 容器三视图互斥，sftpPaneOpen 表示右侧栏
  // 整体开合（沿用既有 workbenchState 持久化），rightPanelView 决定当前内容。
  // 指标/容器视图不跨重启恢复，避免启动即触发轮询。
  const sftpPaneOpen = ref(options.initialPaneOpen);
  const rightPanelView = ref<RightPanelView>("sftp");
  // 容器视图：关闭/切换后即卸载停轮询。Docker 打开期间分栏固定为上下。
  const dockerPanelOpen = computed(() => sftpPaneOpen.value && rightPanelView.value === "docker");
  const sftpViewActive = computed(() => rightPanelView.value === "sftp");
  // 终端拖放的“面板抢落点”门控：只有 SFTP 树真的可见时才把拖入交给面板；
  // 指标/容器视图不应阻止终端接收拖拽。
  const sftpViewVisible = computed(() => sftpPaneOpen.value && sftpViewActive.value);

  const visibleColumns = ref<SftpColumn[]>([...DEFAULT_VISIBLE_COLUMNS]);
  /** 每列当前宽度（px）。 */
  const sftpColumnWidths = reactive<Record<SftpColumn, number>>({ ...DEFAULT_COLUMN_WIDTHS });
  /** 名称列宽度（px）；null = 未拖过，弹性填充剩余空间。 */
  const sftpNameWidth = ref<number | null>(null);
  const sort = ref<{ column: SftpSortColumn; direction: "asc" | "desc" }>({ column: "name", direction: "asc" });

  const sftpSearch = ref("");
  const sftpTypeFilter = ref<SftpTypeFilter>("all");
  /** 默认隐藏以 "." 开头的 Unix 隐藏文件、__pycache__ 等系统缓存条目；
   *  用户可通过过滤栏眼睛图标切换。 */
  const sftpShowHidden = ref(false);

  // Aggregated progress for multi-item batch operations (delete / archive);
  // null while no batch is in flight.
  const batchProgress = ref<BatchProgressState | null>(null);

  // Terminal-local drag overlay: true only while files are dragged over the
  // terminal pane and the drop can actually be accepted (writable session).
  const terminalDragActive = ref(false);

  const localDownloadDir = ref("");
  const localCanSave = ref(false);

  return {
    sftpPane,
    sftpTreePane,
    sftpPaneOpen,
    rightPanelView,
    dockerPanelOpen,
    sftpViewActive,
    sftpViewVisible,
    visibleColumns,
    sftpColumnWidths,
    sftpNameWidth,
    sort,
    sftpSearch,
    sftpTypeFilter,
    sftpShowHidden,
    batchProgress,
    terminalDragActive,
    localDownloadDir,
    localCanSave,
    SFTP_QUICK_PATHS,
  };
}

export type SftpState = ReturnType<typeof useSftpState>;
