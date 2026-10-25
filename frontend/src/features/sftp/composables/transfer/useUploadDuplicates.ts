import { ref, type Ref } from "vue";
import { rpc } from "../../../../shared/lib/rpc";

/** 上传源形状：与本包 useUploadChain 选项的 uploadSource 同构（此处按消费
 * 字段收敛，避免 composable 之间循环引用）。 */
type UploadSource = (
  name: string,
  size: number,
  readChunk: (offset: number, length: number) => Promise<Uint8Array>,
  resume?: { taskId: string; remotePath: string },
  targetDir?: string,
  options?: { duplicatePreCheckedAbsent?: boolean; batchId?: string },
) => Promise<boolean>;

interface UploadDuplicatePrompt {
  fileName: string;
  path: string;
  resolve: (choice: "overwrite" | "rename" | undefined) => void;
}

/** 上传重复目标策略（P1-5：预检 + 重命名/覆盖/询问「应用到全部」）。
 * 弹窗槽位与「应用到全部」批次决策槽都是单实例：所有上传车道（宿主句柄/
 * 本地文件/文件夹/拖拽/剪贴板）共用同一份，批间以
 * resetUploadDuplicateBatchDecision 在每批开头清空。 */
export function useUploadDuplicates(options: {
  uploadSource: UploadSource;
  currentPath: Ref<string>;
  sudoUpload: () => boolean;
  joinRemote: (parent: string, name: string) => string;
  session: Ref<{ sessionId?: string } | undefined>;
  transferDuplicateState: Ref<"rename" | "ask" | "overwrite">;
}) {
  const { uploadSource, currentPath, sudoUpload, joinRemote, session, transferDuplicateState } = options;

  let uploadDuplicateBatchDecision: "overwrite" | "rename" | undefined;

  const uploadDuplicatePrompt = ref<UploadDuplicatePrompt | null>(null);
  const uploadDuplicateApplyAll = ref(false);

/** 批次开头清空决策：所有车道共用这一个槽（见 useUploadChain 调用处）。 */
function resetUploadDuplicateBatchDecision() {
  uploadDuplicateBatchDecision = undefined;
}

function resolveUploadDuplicate(choice: "overwrite" | "rename" | undefined) {
  if (choice !== undefined && uploadDuplicateApplyAll.value) uploadDuplicateBatchDecision = choice;
  uploadDuplicatePrompt.value?.resolve(choice);
  uploadDuplicatePrompt.value = null;
  uploadDuplicateApplyAll.value = false;
}

// 单槽弹窗互斥（promise 链）：runTransfers 以批次上限并发跑上传，两个并发
// 任务同时撞名时，后问直接覆盖 uploadDuplicatePrompt 会把先问的 resolve 顶掉
// ——该 worker 永久 await、批次挂死、并发槽泄漏。串成链后，后到者的询问在
// 先到者结算完成后再占槽；弹窗 UI 保持一次只问一个。
let uploadDuplicateAskChain: Promise<unknown> = Promise.resolve();

function askUploadDuplicate(fileName: string, path: string): Promise<"overwrite" | "rename" | undefined> {
  const asked = uploadDuplicateAskChain.then(
    () =>
      new Promise<"overwrite" | "rename" | undefined>((resolve) => {
        uploadDuplicatePrompt.value = { fileName, path, resolve };
      }),
  );
  uploadDuplicateAskChain = asked.catch(() => undefined);
  return asked;
}

/**
 * 解析上传的最终远端文件名：目标已存在时按策略返回 proceed=false（放弃）或
 * 调整后的名字（rename 经后端 sftp/rename-unique 探测 name(1)..name(999)）。
 * 预检/重命名失败不阻断上传，回落现有覆盖语义。
 *
 * options.forceAskDuplicates：本批次无视用户默认策略（rename/overwrite）强制
 * 弹既有询问——拖拽/文件行入口要求「同名必问」。用显式参数而不是全局标志，
 * 并发批次不会互相串扰；批内「应用到全部」决策仍记在
 * uploadDuplicateBatchDecision（每批开头清空）。
 */
async function resolveUploadDuplicateName(name: string, targetDir: string, options?: { forceAskDuplicates?: boolean }): Promise<{ name: string; proceed: boolean; prechecked?: boolean }> {
  const sessionId = session.value?.sessionId;
  if (!sessionId) return { name, proceed: true, prechecked: false };
  const targetPath = joinRemote(targetDir, name);
  let exists = false;
  try {
    // sudo 车道预检换 sudo/exists（目标目录不可读时 sftp/exists 恒 false）。
    const existsMethod = sudoUpload() ? "sudo/exists" : "sftp/exists";
    const probe = await rpc<{ exists: boolean }>(existsMethod, { sessionId, path: targetPath });
    exists = probe.exists === true;
  } catch {
    // 预检不可用时不能声称「已确认不存在」：把预检交回 uploadSource 的
    // 常规策略（ask/rename），避免 duplicatePreCheckedAbsent 静默覆盖。
    return { name, proceed: true, prechecked: false };
  }
  if (!exists) return { name, proceed: true, prechecked: true };
  const policy = transferDuplicateState.value;
  const forceAsk = options?.forceAskDuplicates === true;
  if (policy === "overwrite" && !forceAsk) return { name, proceed: true, prechecked: true };
  // ask（或强制询问）批量决策：首个冲突问一次；只有勾选「应用到全部」时
  // resolveUploadDuplicate 才把决策写进批次，否则本次选择只作用于本文件。
  let decision = uploadDuplicateBatchDecision;
  if ((forceAsk || policy === "ask") && decision === undefined) {
    const choice = await askUploadDuplicate(name, targetPath);
    if (!choice) return { name, proceed: false, prechecked: false };
    decision = uploadDuplicateBatchDecision ?? choice;
  }
  if ((forceAsk || policy === "ask") && decision === "overwrite") return { name, proceed: true, prechecked: true };
  // rename（或 ask/强制询问选了重命名）：后端探测不冲突新名；旧 sidecar 无该方法时
  // 回落原名覆盖。sudo 车道带 sudo 标记（后端改走 sudo test -e 探测）。
  try {
    const result = await rpc<{ name: string }>("sftp/rename-unique", { sessionId, dir: targetDir, name, sudo: sudoUpload() });
    return { name: result.name || name, proceed: true, prechecked: true };
  } catch {
    // 探测新名失败：交回 uploadSource 的常规策略，不再静默按原名覆盖。
    return { name, proceed: true, prechecked: false };
  }
}

/** 单文件上传的统一入口：forceAskDuplicates 时先做强制询问的预检/裁决，再
 * 带 duplicatePreCheckedAbsent 下发（免去 uploadSource 内部第二次 exists 与
 * 第二次弹窗）；否则原样交给 uploadSource，用户配置策略保持不变。 */
async function uploadWithDuplicatePolicy(
  name: string,
  size: number,
  readChunk: (offset: number, length: number) => Promise<Uint8Array>,
  targetDir: string | undefined,
  options: { batchId?: string; forceAskDuplicates?: boolean },
): Promise<boolean> {
  if (!options.forceAskDuplicates) {
    return uploadSource(name, size, readChunk, undefined, targetDir, options.batchId ? { batchId: options.batchId } : undefined);
  }
  const resolved = await resolveUploadDuplicateName(name, targetDir ?? currentPath.value, { forceAskDuplicates: true });
  if (!resolved.proceed) return false;
  return uploadSource(resolved.name, size, readChunk, undefined, targetDir, {
    batchId: options.batchId,
    duplicatePreCheckedAbsent: resolved.prechecked === true,
  });
}

  return {
    uploadDuplicatePrompt,
    uploadDuplicateApplyAll,
    resolveUploadDuplicate,
    resolveUploadDuplicateName,
    uploadWithDuplicatePolicy,
    resetUploadDuplicateBatchDecision,
  };
}
