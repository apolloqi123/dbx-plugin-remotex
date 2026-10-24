import { type Ref } from "vue";
import { useTransferDisplay } from "./useTransferDisplay";
import { useTransferLedger } from "./useTransferLedger";
import { useTransferOperations } from "./useTransferOperations";
import { useTransferPanelData } from "./useTransferPanelData";
import { useTransferPreferences } from "./useTransferPreferences";

/** 传输队列核心（上传/下载统一账本）：任务记录（终态清收 ring）、进度事件
 * 归并与速度采样、暂停/取消语义、断点恢复对账、面板数据源与展示助手、
 * 传输偏好（并发/重复策略/限速，权威态在内存 + sidecar preferences 同步）。
 * uploadSource 是唯一上传入口（useTransferHistory 断点续传/粘贴/拖拽/快速
 * 命令条共用）；选择器/确认弹窗/下载落盘入口仍在 App.vue。
 * 分工：账本/对账在 useTransferLedger，面板数据源在 useTransferPanelData，
 * 上传/暂停/取消在 useTransferOperations，偏好读写与展示助手分别收口。 */
export function useTransferQueue(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  session: Ref<{ sessionId?: string } | undefined>;
  currentPath: Ref<string>;
  panelSurface: Ref<boolean>;
  resolveUploadDuplicateName: (name: string, targetDir: string) => Promise<{ proceed: boolean; name: string }>;
  joinRemote: (parent: string, name: string) => string;
  writeU64: (bytes: Uint8Array, offset: number, value: number) => void;
  syncPrefs: () => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  refreshTransferHistory: () => Promise<void>;
  refreshResumableUploads: () => Promise<void>;
  /** UploadSudo：sudo 模式 + 可写连接时上传换 sudo 车道——start/finish 换
   * sudo/upload/*（sidecar 把推送落到登录用户 home 暂存件，提交走 sudo mv），
   * 分块二进制帧、进度事件与取消全复用普通管线。断点续传按恢复当刻的开关
   * 状态选道（spool 是本地文件，车道只影响 finish 的远端提交方式）。 */
  sudoUpload: () => boolean;
}) {
  const { t, showError, session, currentPath, panelSurface, resolveUploadDuplicateName, joinRemote, writeU64, syncPrefs, refreshTransferHistory, refreshResumableUploads, sudoUpload } = options;

  const preferences = useTransferPreferences({ syncPrefs });
  const {
    TRANSFER_CONCURRENCY_KEY,
    TRANSFER_MAX_ACTIVE_KEY,
    TRANSFER_DOWNLOAD_LIMIT_KEY,
    TRANSFER_DUPLICATE_KEY,
    transferConcurrencyState,
    transferMaxActiveState,
    transferDownloadLimitState,
    transferDuplicateState,
    loadTransferConcurrency,
    persistTransferConcurrency,
    loadTransferDuplicatePolicy,
    persistTransferDuplicatePolicy,
    loadTransferMaxActive,
    persistTransferMaxActive,
    loadTransferDownloadLimit,
    persistTransferDownloadLimit,
  } = preferences;

  // 账本先建（进度事件/对账的挂载期清理在同一注册点）；面板开合态由
  // useTransferPanelData 持有，新任务到达时的自动弹开用惰性 getter/setter
  // （updateTransfer 只在运行期被事件触发，无 TDZ）。
  const ledger = useTransferLedger({
    t,
    session,
    panelSurface,
    isPanelOpen: () => transferPanelOpen.value,
    openPanel: () => { transferPanelOpen.value = true; },
  });
  const { transferTasks, transferSpeeds, updateTransfer, restoreTransfers, reconcileActiveTransfers, waitForTransferCompletion } = ledger;

  const panelData = useTransferPanelData({
    transferTasks,
    refreshTransferHistory,
    refreshResumableUploads,
    restoreTransfers,
    reconcileActiveTransfers,
  });
  const { transferPanelOpen, transferHistoryMenu, transferList, activeTransfers, refreshTransferPanel } = panelData;

  const operations = useTransferOperations({
    t,
    session,
    currentPath,
    resolveUploadDuplicateName,
    joinRemote,
    writeU64,
    showError,
    transferTasks,
    waitForTransferCompletion,
    sudoUpload,
  });
  const { pausedTaskIds, cancelledTransferTasks, uploadAckWaiters, downloadChunkWaiters, uploadSource, waitWhilePaused, toggleTransferPause, cancelTransfer } = operations;

  const { transferPercent, transferBarValue, transferShownBytes, transferBytesTitle } = useTransferDisplay();

  return {
    TRANSFER_CONCURRENCY_KEY,
    TRANSFER_MAX_ACTIVE_KEY,
    TRANSFER_DOWNLOAD_LIMIT_KEY,
    TRANSFER_DUPLICATE_KEY,
    transferTasks,
    pausedTaskIds,
    transferPanelOpen,
    transferSpeeds,
    transferHistoryMenu,
    transferConcurrencyState,
    transferMaxActiveState,
    transferDownloadLimitState,
    transferDuplicateState,
    cancelledTransferTasks,
    uploadAckWaiters,
    downloadChunkWaiters,
    transferList,
    activeTransfers,
    updateTransfer,
    restoreTransfers,
    reconcileActiveTransfers,
    refreshTransferPanel,
    loadTransferConcurrency,
    persistTransferConcurrency,
    loadTransferDuplicatePolicy,
    persistTransferDuplicatePolicy,
    loadTransferMaxActive,
    persistTransferMaxActive,
    loadTransferDownloadLimit,
    persistTransferDownloadLimit,
    uploadSource,
    waitWhilePaused,
    toggleTransferPause,
    cancelTransfer,
    transferPercent,
    transferBarValue,
    transferShownBytes,
    transferBytesTitle,
  };
}
