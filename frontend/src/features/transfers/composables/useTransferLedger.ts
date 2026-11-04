import { type TransferTask } from "../lib/transferQueue";
import { onBeforeUnmount, reactive, type Ref } from "vue";
import { normalizeTransferStatus } from "../lib/transferHistory";
import { isLiveTransferStatus } from "../lib/transferOrder";
import { mergeTransferProgress } from "../lib/transferProgress";
import { sampleTransferSpeed, type TransferSpeedSample } from "../lib/transferSpeed";
import { rpc } from "../../../shared/lib/rpc";

/** 传输账本（上传/下载统一）：任务记录（终态清收 ring）、进度事件归并与
 *  速度采样、finish 收尾等待器（watchdog 兜底）、后端快照对账/恢复。 */
export function useTransferLedger(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  session: Ref<{ sessionId?: string } | undefined>;
  /** Dock panel surface：单一聚焦终端，传输记录入口整体禁用，不自动弹面板。 */
  panelSurface: Ref<boolean>;
  /** 面板开合态（transferPanelOpen 由 useTransferPanelData 持有；新任务到达
   *  时仅在面板未开时自动弹开，惰性 getter 规避构造顺序）。 */
  isPanelOpen: () => boolean;
  openPanel: () => void;
}) {
  const { t, session, panelSurface, isPanelOpen, openPanel } = options;

  const transferTasks = reactive<Record<string, TransferTask>>({});
  const transferSpeeds = reactive<Record<string, number>>({});
  const transferSamples = new Map<string, TransferSpeedSample>();

  // 上传收尾等待器（issue #60）：finish RPC 只负责把远端推送交给 sidecar
  // 后台任务，真正的完成/失败经终态 progress 事件回传，这里据此结算。
  // 评审 H-1（2026-09-30）：等待器自带 watchdog 轮询——终态事件丢失（宿主
  // 桥抖动）或 sidecar 在长推送期间重启时，靠 sftp/transfer/status 兜底
  // 结算，不再让 uploadSource 永久挂起拖死整批队列。
  const COMPLETION_WATCHDOG_MS = 15_000;
  const transferCompletionWaiters = new Map<string, { resolve: () => void; reject: (error: Error) => void; watchdog: number }>();

  function updateTransfer(params: Record<string, unknown>) {
    const taskId = String(params.taskId || "");
    if (!taskId) return;
    const existing = transferTasks[taskId];
    const status = normalizeTransferStatus(params.status, existing?.status);
    const progress = mergeTransferProgress(existing, params);
    // 速度只采样真实网络推送（uploading/fetching/transferring 阶段）：
    // staging 字节走本机内存/磁盘，压缩/解压是本机 CPU 段，计入都会显示
    // 假速度（issue #60；M33 扩展本地阶段集合）。阶段切换时重置采样窗口。
    const phaseChanged = progress.phase !== existing?.phase;
    if (progress.phase === undefined || progress.phase === "uploading" || progress.phase === "fetching" || progress.phase === "transferring") {
      const sample = sampleTransferSpeed(phaseChanged ? undefined : transferSamples.get(taskId), progress.transferred, performance.now());
      transferSamples.set(taskId, sample);
      transferSpeeds[taskId] = sample.speed;
    } else {
      transferSamples.delete(taskId);
      transferSpeeds[taskId] = 0;
    }
    // 目录下载事件附带的树内字段（fileCount/currentFile）有则透传；
    // 文件下载事件不带这些键，保持原有行为。（issue #46）
    const fileCount = params.fileCount !== undefined ? Number(params.fileCount) : existing?.fileCount;
    const currentFile = typeof params.currentFile === "string" ? params.currentFile : existing?.currentFile;
    // 压缩通道标记（M33）：prep 回退事件（compression=none）清掉徽标。
    const compression = params.compression === "gzip" ? "gzip" as const : params.compression === "none" ? undefined : existing?.compression;
    transferTasks[taskId] = {
      taskId,
      sessionId: String(params.sessionId || existing?.sessionId || ""),
      direction: params.direction === "download" ? "download" : existing?.direction || "upload",
      // displayName 是用户可见名（文件夹整包上传的临时 tar 等）：progress 事件
      // 的 fileName 是远端内部名，不得把它覆盖回面板。
      fileName: existing?.displayName ?? String(params.fileName || existing?.fileName || ""),
      displayName: existing?.displayName,
      displayFolder: existing?.displayFolder,
      size: progress.size,
      transferred: progress.transferred,
      staged: progress.staged,
      phase: progress.phase,
      compression,
      status,
      error: typeof params.error === "string" ? params.error : existing?.error,
      joinedAt: existing?.joinedAt ?? Date.now(),
      fileCount: Number.isFinite(fileCount) && fileCount! > 0 ? fileCount : undefined,
      currentFile: currentFile || undefined,
      // 事件从不携带批次键（issue #165）：透传首见记录上的 batchId。
      batchId: existing?.batchId,
    };
    settleTransferCompletion(taskId, status);
    if (!isLiveTransferStatus(status)) scheduleTerminalTaskSweep(taskId);
    if (!existing && isLiveTransferStatus(transferTasks[taskId].status)) {
      // 面板已开时不得重开：openTransferPanel 的"先收口再开"会卸载弹层、
      // 复位滚动位置，用户正往下看历史时会被弹回顶部（issue #18）。互斥族
      // 保证面板开着时没有其他弹层，直接置 open 即可。Dock panel surface
      // 同样不弹：面板是单一聚焦终端，传输记录入口整体禁用。
      if (!isPanelOpen() && !panelSurface.value) openPanel();
    }
  }

  // —— 终态任务清收 ——
  // transferTasks/transferSpeeds 此前只在视图层过滤，记录本身永不清收：长会话
  // 成千上万次小传输会持续累积响应式对象并放大每次进度写入的依赖追踪成本。
  // 终态后保留一小段时间供用户看到结果，随后删除任务与速度/采样记录；记录
  // 总量超 ring 上限时最老终态先删。活跃任务永不清理；记录删除后同一 taskId
  // 再来事件会照常重建（updateTransfer 的 !existing 分支）。
  const TERMINAL_TASK_RETENTION_MS = 30_000;
  const TERMINAL_TASK_RING_LIMIT = 200;
  const terminalTaskSweepTimers = new Map<string, number>();

  function dropTransferRecord(taskId: string) {
    const timer = terminalTaskSweepTimers.get(taskId);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      terminalTaskSweepTimers.delete(taskId);
    }
    delete transferTasks[taskId];
    delete transferSpeeds[taskId];
    transferSamples.delete(taskId);
  }

  function scheduleTerminalTaskSweep(taskId: string) {
    if (terminalTaskSweepTimers.has(taskId)) return;
    terminalTaskSweepTimers.set(
      taskId,
      window.setTimeout(() => {
        terminalTaskSweepTimers.delete(taskId);
        dropTransferRecord(taskId);
      }, TERMINAL_TASK_RETENTION_MS),
    );
    enforceTransferRecordRing();
  }

  function enforceTransferRecordRing() {
    let overflow = Object.keys(transferTasks).length - TERMINAL_TASK_RING_LIMIT;
    if (overflow <= 0) return;
    const sweepable = Object.values(transferTasks)
      .filter((task) => !isLiveTransferStatus(task.status) && terminalTaskSweepTimers.has(task.taskId))
      .sort((a, b) => (a.joinedAt ?? 0) - (b.joinedAt ?? 0));
    for (const task of sweepable) {
      if (overflow <= 0) break;
      dropTransferRecord(task.taskId);
      overflow -= 1;
    }
  }

  onBeforeUnmount(() => {
    for (const timer of terminalTaskSweepTimers.values()) window.clearTimeout(timer);
    terminalTaskSweepTimers.clear();
    // 卸载后 watchdogs 仍会每 15s 发 sftp/transfer/status（sidecar 可能已随之
    // 退出，全部失败也照轮询）：清定时器并把等待中的收尾显式收口。
    for (const [taskId, waiter] of transferCompletionWaiters) {
      window.clearTimeout(waiter.watchdog);
      waiter.reject(Object.assign(new Error(`transfer ledger disposed (${taskId})`), { code: "transfer-terminal" }));
    }
    transferCompletionWaiters.clear();
  });

  /** 终态事件结算 finish 之后的收尾等待器：completed 兑现；cancelled 按用户
   * 取消回传（transfer-cancelled 码，runTransfers 据此继续派发剩余项，评审
   * H-3）；failed 才视为批次失败。 */
  function settleTransferCompletion(taskId: string, status: TransferTask["status"]) {
    const waiter = transferCompletionWaiters.get(taskId);
    if (!waiter || (status !== "completed" && status !== "cancelled" && status !== "failed")) return;
    transferCompletionWaiters.delete(taskId);
    window.clearTimeout(waiter.watchdog);
    if (status === "completed") waiter.resolve();
    else if (status === "cancelled") waiter.reject(Object.assign(new Error(transferTasks[taskId]?.error || t(`transferStatus.${status}`)), { code: "transfer-cancelled" }));
    else waiter.reject(Object.assign(new Error(transferTasks[taskId]?.error || t(`transferStatus.${status}`)), { code: "transfer-terminal" }));
  }

  /** 后端状态快照统一入账（评审 H-1）：面板对账与 watchdog 轮询共用——
   * 终态才生效，且必须穿过 settleTransferCompletion。此前 reconcile 直接改
   * task.status 绕过结算，事件丢失后等待器永不兑现、批次挂死。 */
  function applyTransferStatusSnapshot(taskId: string, status: { transferred?: number; status: string; phase?: string }): boolean {
    const task = transferTasks[taskId];
    if (!task) return true;
    const normalized = normalizeTransferStatus(status.status, task.status);
    if (normalized !== "queued" && normalized !== "running") {
      task.status = normalized;
      if (status.transferred != null) {
        // staging 阶段的 transferred 字段是 spool 字节数，不能覆盖真实推送计数。
        if (status.phase === "staging") task.staged = status.transferred;
        else task.transferred = status.transferred;
      }
      settleTransferCompletion(taskId, normalized);
      if (!isLiveTransferStatus(normalized)) scheduleTerminalTaskSweep(taskId);
      return true;
    }
    return false;
  }

  /** 终态兜底轮询：终态已入账返回 true（等待器已结算）；否则继续等下一轮。
   * 查询失败（sidecar 重启中/任务已被清收）不算终态。 */
  async function pollTransferStatus(taskId: string): Promise<boolean> {
    try {
      const status = await rpc<{ transferred?: number; status: string; phase?: string }>("sftp/transfer/status", { taskId });
      return applyTransferStatusSnapshot(taskId, status);
    } catch {
      return false;
    }
  }

  function armCompletionWatchdog(taskId: string): number {
    return window.setTimeout(async () => {
      if (!transferCompletionWaiters.has(taskId)) return;
      if (await pollTransferStatus(taskId)) return;
      const waiter = transferCompletionWaiters.get(taskId);
      if (waiter) waiter.watchdog = armCompletionWatchdog(taskId);
    }, COMPLETION_WATCHDOG_MS);
  }

  /** 挂起直到该任务收到终态 progress 事件（完成/取消/失败）；注册前已终态则
   * 立即结算，否则挂等待器并武装 watchdog 轮询兜底（评审 H-1）。后台推送可
   * 持续数十分钟（issue #60），轮询只是轻量 RPC，不设次数上限。 */
  function waitForTransferCompletion(taskId: string) {
    const existing = transferTasks[taskId];
    const status = existing?.status;
    if (status === "completed" || status === "cancelled" || status === "failed") {
      return status === "completed" ? Promise.resolve() : Promise.reject(Object.assign(new Error(existing?.error || t(`transferStatus.${status}`)), { code: status === "cancelled" ? "transfer-cancelled" : "transfer-terminal" }));
    }
    return new Promise<void>((resolve, reject) => {
      transferCompletionWaiters.set(taskId, { resolve, reject, watchdog: armCompletionWatchdog(taskId) });
    });
  }

  async function restoreTransfers() {
    if (!session.value) return;
    const result = await rpc<{ tasks: TransferTask[] }>("sftp/transfer/list", { sessionId: session.value.sessionId }).catch(() => ({ tasks: [] }));
    for (const task of result.tasks) {
      // 后端 list 的 staging 行把 spool 字节放在 transferred 里；恢复到本地
      // 状态时归位到 staged，避免重挂后进度条展示阶段计数（issue #60）。
      if (task.phase === "staging") {
        task.staged = task.transferred;
        task.transferred = 0;
      }
      const existing = transferTasks[task.taskId];
      // joinedAt 只在首次见到时落一次：后端返回序（HashMap 迭代序）不再影响
      // 活跃区排序（issue #18）。batchId 同理透传（issue #165）：它是纯前端
      // 键（后端 list 不带），不透传的话批中关开一次面板，restoreTransfers
      // 整行覆盖就会把批次卡打回 0/N · 0%。
      transferTasks[task.taskId] = {
        ...task,
        fileName: existing?.displayName ?? task.fileName,
        displayName: existing?.displayName,
        displayFolder: existing?.displayFolder,
        joinedAt: existing?.joinedAt ?? Date.now(),
        batchId: existing?.batchId ?? task.batchId,
      };
    }
  }

  /**
   * 面板打开时对账活跃任务：逐个向后端查询 `sftp/transfer/status`，后端已不
   * 认识的任务（sidecar 重启、页面重载后错过终态事件的“僵尸行”）标记为失败，
   * 终态以服务端为准。查询失败视为任务已死——存活任务的状态查询总会成功。
   * 入账统一走 applyTransferStatusSnapshot（评审 H-1）：直接改状态会绕过
   * settleTransferCompletion，事件丢失时 uploadSource 的收尾等待永不兑现。
   */
  async function reconcileActiveTransfers() {
    for (const task of Object.values(transferTasks)) {
      if (task.status !== "queued" && task.status !== "running") continue;
      try {
        const status = await rpc<{ transferred?: number; status: string; phase?: string }>("sftp/transfer/status", { taskId: task.taskId });
        applyTransferStatusSnapshot(task.taskId, status);
      } catch {
        task.status = "failed";
        task.error = t("transfersHistory.interrupted");
        settleTransferCompletion(task.taskId, "failed");
      }
    }
  }

  return {
    transferTasks,
    transferSpeeds,
    updateTransfer,
    restoreTransfers,
    reconcileActiveTransfers,
    waitForTransferCompletion,
  };
}
