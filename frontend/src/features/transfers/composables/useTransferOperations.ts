import { type TransferTask } from "../lib/transferQueue";
import { reactive, type Ref } from "vue";
import { transferCancelReason } from "../lib/transferProgress";
import { transferPausable } from "../lib/transferResume";
import { rpc } from "../../../shared/lib/rpc";
import {
  UPLOAD_SPOOL_WINDOW,
  enqueueUploadAck,
  rejectUploadAcks,
  removeUploadAck,
  type UploadAckWaiter,
  type UploadAckWaiters,
} from "../lib/uploadAck";

/** 传输数据面（队列执行侧）：唯一上传入口、分片循环与 ack 等待、暂停/取消
 *  语义（等待器即时释放），以及下载分块/取消登记的共享表。 */
export function useTransferOperations(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  session: Ref<{ sessionId?: string } | undefined>;
  currentPath: Ref<string>;
  resolveUploadDuplicateName: (name: string, targetDir: string) => Promise<{ proceed: boolean; name: string }>;
  joinRemote: (parent: string, name: string) => string;
  writeU64: (bytes: Uint8Array, offset: number, value: number) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  /** 账本任务表（同一 reactive 对象，上传入口建卡直写）。 */
  transferTasks: Record<string, TransferTask>;
  waitForTransferCompletion: (taskId: string) => Promise<void>;
  /** UploadSudo：sudo 模式 + 可写连接时上传换 sudo 车道——start/finish 换
   * sudo/upload/*（sidecar 把推送落到登录用户 home 暂存件，提交走 sudo mv），
   * 分块二进制帧、进度事件与取消全复用普通管线。断点续传按恢复当刻的开关
   * 状态选道（spool 是本地文件，车道只影响 finish 的远端提交方式）。 */
  sudoUpload: () => boolean;
}) {
  const { t, session, currentPath, resolveUploadDuplicateName, joinRemote, writeU64, showError, transferTasks, waitForTransferCompletion, sudoUpload } = options;

  // 断点续传（F1）：暂停中的任务（两分片之间生效）；等待恢复的回调登记表。
  const pausedTaskIds = reactive(new Set<string>());
  const pauseWaiters = new Map<string, Array<() => void>>();

  const uploadAckWaiters: UploadAckWaiters = new Map();
  const downloadChunkWaiters = new Map<string, { offset: number; resolve: (bytes: Uint8Array) => void; reject: (error: Error) => void; timer: number }>();
  // Download task ids the user cancelled from the transfer panel; lets the download
  // loop distinguish a user cancel (notice) from a real failure (error banner).
  const cancelledTransferTasks = new Set<string>();

  /** 单文件上传入口。返回 boolean 供批次调用方区分成功与放弃：true=已交付
   * sidecar（后台推送中也算），false=重复目标弹窗里放弃跳过（无会话同理）——
   * 抛错仍是失败。成功提示的计数只该包含 true 的那份。 */
  async function uploadSource(name: string, size: number, readChunk: (offset: number, length: number) => Promise<Uint8Array>, resume?: { taskId: string; remotePath: string }, targetDir?: string, options?: { duplicatePreCheckedAbsent?: boolean; batchId?: string; displayName?: string; displayFolder?: boolean }): Promise<boolean> {
    if (!session.value) return false;
    // resume 携带原 taskId/remotePath：后端校验 spool meta 后从已传前缀续接。
    // targetDir 仅新上传生效（终端拖入的自定义目标目录）；缺省仍是 SFTP 当前目录。
    const dir = targetDir ?? currentPath.value;
    // 重复目标预检（P1-5）：仅新上传生效；rename 可能改写最终远端文件名，
    // 后续 remotePath 与传输面板展示名都用解析后的名字。duplicatePreChecked-
    // Absent（评审 M-4）：文件夹批量 ask 模式已逐文件预检过不存在，跳过二次
    // sftp/exists（千文件目录少一半往返）。
    let uploadName = name;
    if (!resume && !options?.duplicatePreCheckedAbsent) {
      const resolved = await resolveUploadDuplicateName(name, dir);
      if (!resolved.proceed) return false;
      uploadName = resolved.name;
    }
    // UploadSudo：车道在任务启动当刻定死，start 与 finish 必须同道（sidecar
    // 按注册表里的 sudo 标记分派提交方式，finish 方法名只是前端契约一致性）。
    const sudoLane = sudoUpload();
    const startMethod = sudoLane ? "sudo/upload/start" : "sftp/upload/start";
    const finishMethod = sudoLane ? "sudo/upload/finish" : "sftp/upload/finish";
    const info = await rpc<{ taskId: string; chunkSize: number; resumeOffset?: number; compression?: string }>(startMethod, resume
      ? { sessionId: session.value.sessionId, remotePath: resume.remotePath, size, resumeTaskId: resume.taskId }
      : { sessionId: session.value.sessionId, remotePath: joinRemote(dir, uploadName), size });
    const startOffset = info.resumeOffset ?? 0;
    // 压缩通道标记（M33）：start 响应即带决策结果，任务卡建卡时就点徽标；
    // 后续 progress 事件（恒带 compression）接手维护，回退时以 none 清除。
    // 容器上传的暂存文件名由服务端消毒，展示名保留用户选中的原文件名；
    // 仅影响任务卡文案，远端写入路径仍是 uploadName。
    transferTasks[info.taskId] = { taskId: info.taskId, sessionId: session.value.sessionId, direction: "upload", fileName: options?.displayName ?? uploadName, displayName: options?.displayName, displayFolder: options?.displayFolder, size, transferred: startOffset, compression: info.compression === "gzip" ? "gzip" : undefined, status: startOffset > 0 ? "running" : "queued", joinedAt: Date.now(), batchId: options?.batchId };
    // spool 窗口：readChunk（宿主桥 base64 读盘）与 sidecar 落盘确认相互
    // 重叠，最多 UPLOAD_SPOOL_WINDOW 片在途；二进制通道保序，sidecar 的
    // 顺序 offset 校验因此不受影响，断点续传前缀语义也不变。
    const pendingAcks: Array<Promise<void>> = [];
    try {
      let offset = startOffset;
      while (offset < size) {
        await waitWhilePaused(info.taskId);
        // 暂停中被取消：releasePause 唤醒后循环会继续 sendBinary 打到已取消的
        // taskId，后端报非取消错误码会把整批剩余任务一起中止（与 H-3 的用户
        // 取消不中止整批语义相悖）。恢复后先查取消集，以 transfer-cancelled
        // 出局，让 runTransfers 继续派发。
        if (cancelledTransferTasks.has(info.taskId)) {
          throw Object.assign(new Error(t("transferStatus.cancelled")), { code: "transfer-cancelled" });
        }
        let chunk: Uint8Array;
        try {
          chunk = await readChunk(offset, info.chunkSize);
        } catch (cause) {
          throw Object.assign(cause instanceof Error ? cause : new Error(String(cause)), { code: "upload-read-failed" });
        }
        if (!chunk.byteLength) throw new Error(t("errors.localFileShortRead"));
        // 读盘期间到达的取消：此时任务可能已被后端摘除，再发一帧只会拿到
        // upload-append-failed（非取消码），把整批剩余项一起中止——按用户
        // 取消出局，让 runTransfers 继续派发。
        if (cancelledTransferTasks.has(info.taskId)) {
          throw Object.assign(new Error(t("transferStatus.cancelled")), { code: "transfer-cancelled" });
        }
        const payload = new Uint8Array(8 + chunk.byteLength);
        writeU64(payload, 0, offset);
        payload.set(chunk, 8);
        const nextOffset = offset + chunk.byteLength;
        const ack = waitForUploadAck(info.taskId, nextOffset);
        // 失败路径会一次性拒绝所有在途 ack；先挂空处理器避免被放弃的
        // promise 以 unhandled rejection 形式冒到全局。
        ack.catch(() => undefined);
        pendingAcks.push(ack);
        await window.dbxPlugin.sendBinary(`sftp/upload/${info.taskId}`, payload);
        offset = nextOffset;
        if (pendingAcks.length >= UPLOAD_SPOOL_WINDOW) await pendingAcks.shift();
      }
      await Promise.all(pendingAcks);
      // finish RPC 只把远端推送交给 sidecar 后台任务就返回（多 GB 文件的推送
      // 可达数十分钟，长持 RPC 会被桥上任何一端的 deadline 判死并"自动取消"，
      // issue #60）；真正的完成/失败由终态 progress 事件回传，这里等它落地。
      try {
        await rpc(finishMethod, { taskId: info.taskId }, { timeoutMs: 60_000 });
      } catch (cause) {
        throw Object.assign(cause instanceof Error ? cause : new Error(String(cause)), { code: "upload-start-failed" });
      }
      await waitForTransferCompletion(info.taskId);
      return true;
    } catch (cause) {
      rejectUploadAcks(
        uploadAckWaiters,
        info.taskId,
        cause instanceof Error ? cause : new Error(String(cause)),
      );
      await rpc("sftp/transfer/cancel", { taskId: info.taskId, reason: transferCancelReason(cause) }).catch(() => undefined);
      throw cause;
    }
  }

  function waitForUploadAck(taskId: string, nextOffset: number) {
    return new Promise<void>((resolve, reject) => {
      const timer = window.setTimeout(async () => {
        // 30s 未确认先查任务真实状态：确实已落盘（ack 事件丢失/桥抖动）就
        // 只释放这一片；任务已死或没推进，则整条上传按 ack-timeout 收口。
        try {
          const status = await rpc<{ transferred: number; status: string }>("sftp/transfer/status", { taskId });
          if (status.transferred >= nextOffset && status.status === "running") {
            removeUploadAck(uploadAckWaiters, taskId, nextOffset);
            resolve();
            return;
          }
          rejectUploadAcks(
            uploadAckWaiters,
            taskId,
            Object.assign(new Error(t("errors.uploadAckTimeout")), { code: "upload-ack-timeout" }),
          );
        } catch (cause) {
          rejectUploadAcks(
            uploadAckWaiters,
            taskId,
            cause instanceof Error ? cause : new Error(String(cause)),
          );
        }
      }, 30_000);
      const waiter: UploadAckWaiter = { nextOffset, resolve, reject, timer };
      enqueueUploadAck(uploadAckWaiters, taskId, waiter);
    });
  }

  // 分片循环在每个分片之间调用；暂停时挂起，恢复后继续。
  function waitWhilePaused(taskId: string): Promise<void> | undefined {
    if (!pausedTaskIds.has(taskId)) return undefined;
    return new Promise((resolve) => {
      const waiters = pauseWaiters.get(taskId) ?? [];
      waiters.push(resolve);
      pauseWaiters.set(taskId, waiters);
    });
  }

  function releasePause(taskId: string) {
    if (pausedTaskIds.delete(taskId)) {
      for (const waiter of pauseWaiters.get(taskId) ?? []) waiter();
    }
    pauseWaiters.delete(taskId);
  }

  function toggleTransferPause(task: TransferTask) {
    if (!transferPausable(task.status)) return;
    if (pausedTaskIds.has(task.taskId)) releasePause(task.taskId);
    else pausedTaskIds.add(task.taskId);
  }

  async function cancelTransfer(task: TransferTask) {
    releasePause(task.taskId);
    if (task.direction === "download") {
      // Reject the pending chunk waiter so the download loop exits immediately
      // instead of waiting for its 30s timeout; the backend cancel follows below.
      const waiter = downloadChunkWaiters.get(task.taskId);
      if (waiter) {
        window.clearTimeout(waiter.timer);
        downloadChunkWaiters.delete(task.taskId);
        waiter.reject(new Error(t("transferStatus.cancelled")));
      }
      cancelledTransferTasks.add(task.taskId);
    } else {
      // 上传取消对称地立即释放 ack 等待器（评审 H-3）：后端取消即摘任务，
      // ack 永不再来——不等 30s 超时。错误码 transfer-cancelled 让
      // runTransfers 把用户取消与真实失败区分开、继续派发剩余项。同时登记
      // 取消集：暂停中的任务没有挂起的 ack 等待器，恢复后的分片循环靠它
      // 识别"恢复即终止"（见 uploadSource 的循环头检查）。
      cancelledTransferTasks.add(task.taskId);
      rejectUploadAcks(
        uploadAckWaiters,
        task.taskId,
        Object.assign(new Error(t("transferStatus.cancelled")), { code: "transfer-cancelled" }),
      );
    }
    // reason=user 让后端账本把"用户主动取消"与异常清理区分开（issue #60）。
    await rpc("sftp/transfer/cancel", { taskId: task.taskId, reason: "user" }).catch((cause) => showError(cause));
  }

  return {
    pausedTaskIds,
    cancelledTransferTasks,
    uploadAckWaiters,
    downloadChunkWaiters,
    uploadSource,
    waitWhilePaused,
    toggleTransferPause,
    cancelTransfer,
  };
}
