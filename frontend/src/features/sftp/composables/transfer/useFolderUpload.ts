import { ref, type Ref } from "vue";
import { confirmDialog } from "../../../../shared/lib/confirmDialog";
import { type FolderUploadProgress, advanceFolderUploadDirectories, buildFolderUploadPlan, createFolderUploadProgress, folderUploadOutcome, settleFolderUploadFile } from "../../lib/transfer/folderUpload";
import { folderOverwriteTarget, folderUploadRootPath, folderUploadRootPaths } from "../../lib/transfer/folderOverwrite";
import { createTarChunkSource, folderArchiveName, shouldFolderArchive } from "../../lib/transfer/folderArchive";
import { runTransfers } from "../../../transfers/lib/transferQueue";
import { rpc } from "../../../../shared/lib/rpc";

interface FolderUploadEntry {
  name: string;
  relativePath: string;
  size: number;
  readChunk: (offset: number, length: number) => Promise<Uint8Array>;
}

/** 上传源形状：与本包 useUploadChain 选项的 uploadSource 同构（此处按消费
 * 字段收敛，避免 composable 之间循环引用）。 */
type UploadSource = (
  name: string,
  size: number,
  readChunk: (offset: number, length: number) => Promise<Uint8Array>,
  resume?: { taskId: string; remotePath: string },
  targetDir?: string,
  options?: { duplicatePreCheckedAbsent?: boolean; batchId?: string; displayName?: string; displayFolder?: boolean },
) => Promise<boolean>;

/** 文件夹上传（issue #78）：webkitdirectory 选择器 + 纯前端编排，复用既有
 * sftp/upload 管线。重复裁决（resolveUploadDuplicateName）与批次决策槽由
 * useUploadDuplicates 单实例提供。 */
export function useFolderUpload(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  connected: Ref<boolean>;
  canWrite: Ref<boolean>;
  session: Ref<{ sessionId?: string } | undefined>;
  currentPath: Ref<string>;
  sftpNameEncodingState: Ref<string>;
  openTransferPanel: () => void;
  uploadSource: UploadSource;
  joinRemote: (parent: string, name: string) => string;
  sudoUpload: () => boolean;
  loadTransferDuplicatePolicy: () => "rename" | "ask" | "overwrite";
  batchTransferLimit: () => number;
  resolveUploadDuplicateName: (name: string, targetDir: string, options?: { forceAskDuplicates?: boolean }) => Promise<{ name: string; proceed: boolean; prechecked?: boolean }>;
  resetUploadDuplicateBatchDecision: () => void;
  refreshUploadedTarget: (targetDir?: string) => Promise<void>;
}) {
  const { t, showNotice, showError, connected, canWrite, session, currentPath, sftpNameEncodingState, openTransferPanel, uploadSource, joinRemote, sudoUpload, loadTransferDuplicatePolicy, batchTransferLimit, resolveUploadDuplicateName, resetUploadDuplicateBatchDecision, refreshUploadedTarget } = options;

// 文件夹上传（issue #78）：webkitdirectory 选择器 + 能力探测（缺失时入口隐藏）。
const folderUploadInput = ref<HTMLInputElement>();
const folderUploadSupported = ref(false);
const pendingFolderUploadTargetDir = ref<string>();
// 文件夹批量上传的聚合进度（复用传输面板展示；目录 X/Y · 文件 N/M）。
const folderUploadProgress = ref<FolderUploadProgress>();

// —— 文件夹上传（issue #78）：纯前端编排，复用既有 sftp/upload 管线 ——
// 选目录 → buildFolderUploadPlan 生成远端目录集 + 文件清单 → 逐目录
// sftp/createDirectory（逐个 ensure，父先于子）→ 逐文件 uploadSource
// （冲突策略：rename/overwrite 交给既有 resolveUploadDuplicateName；ask 在
// 批量下降级为「已存在即跳过」+ 完成提示计数，避免上百次弹窗交互）。
// 能力探测：浏览器无 webkitdirectory 支持时入口隐藏（onMounted 一次性探测）。

function chooseFolderUpload(targetDir?: string) {
  if (!connected.value || !canWrite.value || !folderUploadSupported.value) return;
  openTransferPanel();
  pendingFolderUploadTargetDir.value = targetDir;
  folderUploadInput.value?.click();
}

function onFolderUploadInput(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files || []);
  input.value = "";
  const targetDir = pendingFolderUploadTargetDir.value;
  pendingFolderUploadTargetDir.value = undefined;
  if (!files.length) return;
  void uploadFolderFiles(files.map((file) => ({
    name: file.name,
    relativePath: (file as File & { webkitRelativePath?: string }).webkitRelativePath || "",
    size: file.size,
    readChunk: (offset: number, length: number) => file.slice(offset, offset + length).arrayBuffer().then((buffer) => new Uint8Array(buffer)),
  })), targetDir).catch(showError);
}

/** 批次内每个顶层根目录的存在性预检 + 整目录覆盖确认：存在的根问一次，
 * 接受则递归删除后继续；拒绝、根不合法或删除失败（showError 后）把整根标记
 * 为 blocked，其下条目全部跳过。返回被确认覆盖并已删除的根集合，逐文件车道
 * 据此跳过第二次预检/弹窗。归档与逐文件回退两条车道共用本判定。 */
async function confirmFolderRootOverwrites(roots: readonly string[], base: string, sessionId: string): Promise<{ blocked: Set<string>; overwritten: Set<string> }> {
  const blocked = new Set<string>();
  const overwritten = new Set<string>();
  for (const root of roots) {
    const targetPath = folderOverwriteTarget(base, root, joinRemote);
    if (!targetPath) {
      // 非法根（空/dot/dotdot/绝对/多段）绝不拼路径删除；整根跳过。
      blocked.add(root);
      continue;
    }
    let exists = false;
    try {
      // sudo 车道预检换 sudo/exists（目标目录对登录用户不可读时 sftp/exists 恒 false）。
      const existsMethod = sudoUpload() ? "sudo/exists" : "sftp/exists";
      const probe = await rpc<{ exists: boolean }>(existsMethod, { sessionId, path: targetPath });
      exists = probe.exists === true;
    } catch (cause) {
      // 无法确认目标是否存在时不能静默按「不存在」继续——归档解包恒覆盖，
      // 会把未知的既有目录内容盖掉。失败关闭：该根跳过并明确报错。
      showError(cause, "sftp");
      blocked.add(root);
      continue;
    }
    if (!exists) continue;
    const accepted = await confirmDialog(t("folderUpload.overwriteFolderConfirm", { name: root, path: targetPath }), { danger: true });
    if (!accepted) {
      blocked.add(root);
      continue;
    }
    try {
      // 覆盖 = 整目录删除后再上传：保留旧目录里的残留文件会让「覆盖文件夹」
      // 名不副实（解包/逐文件只覆盖同名项）。
      if (sudoUpload()) await rpc("sudo/removeAll", { sessionId, path: targetPath });
      else await rpc("sftp/delete", { sessionId, path: targetPath, recursive: true });
      overwritten.add(root);
    } catch (cause) {
      showError(cause, "sftp");
      blocked.add(root);
    }
  }
  return { blocked, overwritten };
}

async function uploadFolderFiles(entries: readonly FolderUploadEntry[], baseDir?: string, options?: { forceAskDuplicates?: boolean }) {
  if (!session.value?.sessionId || !canWrite.value) return;
  // 并发 run 闭包里不能用 session.value（TS 无法跨异步闭包收窄）：守卫后
  // 捕获一次。
  const sessionId = session.value.sessionId;
  // 批量决策是「每批次开头清 undefined」的契约：文件夹入口同样要清，否则
  // 上一批的「应用到全部=覆盖」残留会让本批归档撞名时静默放行（解包恒覆盖，
  // 属破坏性结果）。
  resetUploadDuplicateBatchDecision();
  // baseDir：终端/树行/宿主桥拖入的自定义落点；缺省仍是 SFTP 当前目录。
  const base = baseDir ?? currentPath.value;
  const plan = buildFolderUploadPlan(entries);
  if (!plan.files.length) {
    showNotice(t("folderUpload.empty"));
    return;
  }
  // 整文件夹覆盖（多根逐个确认一次）必须在归档/逐文件之前：接受即递归删除
  // 旧根目录，拒绝则整根跳过（多根批次里其余根照常上传）。
  const roots = folderUploadRootPaths(plan.files);
  const { blocked, overwritten } = await confirmFolderRootOverwrites(roots, base, sessionId);
  const uploadFiles = plan.files.filter((file) => {
    const root = folderUploadRootPath(file.relativePath);
    return !root || !blocked.has(root);
  });
  const uploadDirectories = plan.directories.filter((directory) => {
    // 目录本身没有文件名段：补一个探针段走同一套根归一。
    const root = folderUploadRootPath(`${directory}/file`);
    return !root || !blocked.has(root);
  });
  if (!uploadFiles.length) return;
  const uploadPlan = uploadFiles.length === plan.files.length && uploadDirectories.length === plan.directories.length
    ? plan
    : {
      directories: uploadDirectories,
      files: uploadFiles,
      totalBytes: uploadFiles.reduce((sum, file) => sum + file.size, 0),
    };
  // 文件夹整包车道（folderArchive）：目录打成单个流式 tar 走既有上传管线
  // （网络腿 gzip 由 M33 决策、sudo 车道由 UploadSudo 接管），落盘后远端
  // sftp/extract 解包。latin-1 不走该车道：tar 头名字节与解包命令串都是
  // UTF-8 边界（登记边界）。整包超传输上限时回退逐文件。
  if (sftpNameEncodingState.value !== "latin-1" && shouldFolderArchive(uploadPlan.files.length)) {
    const archiveEntries = uploadPlan.files.map((file) => ({
      relativePath: file.relativePath,
      size: file.size,
      // readChunk 经计划保留的 source 回取：plan.files 是 entries 的过滤子集，
      // 按下标配对会在首条被跳过后整体错位（A 的字节传到 B 的名字下）。
      readChunk: file.source.readChunk,
    }));
    // 归档是瞬时传输载体，使用不会与用户文件冲突的临时名：既避免同名
    // .tar 的第二次覆盖询问，也保证任何冲突/取消都不会发生在根目录已删除
    // 之后（覆盖确认的破坏性动作只由根目录提示触发）。
    const archiveDisplayName = folderArchiveName(archiveEntries);
    // 展示名去掉 .tar：用户上传的是文件夹，不是归档文件本身。
    const archiveFolderName = archiveDisplayName.replace(/\.tar$/i, "") || "folder";
    const uniqueSuffix = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    // 临时名带上（消毒后的）文件夹名：任务卡与落盘历史都能还原可读名称，
    // 而不是 `.sshx-upload-<随机串>.tar`。
    const safeStem = archiveFolderName.replace(/[^\p{L}\p{N}._-]+/gu, "-").replace(/^-+|-+$/g, "").slice(0, 48) || "folder";
    const archiveName = `.sshx-upload-${safeStem}-${uniqueSuffix}.tar`;
    const source = createTarChunkSource(archiveEntries, archiveName);
    if (source) {
      openTransferPanel();
      try {
        const archivePath = joinRemote(base, archiveName);
        await uploadSource(archiveName, source.size, source.readChunk, undefined, base, {
          duplicatePreCheckedAbsent: true,
          displayName: archiveFolderName,
          displayFolder: true,
        });
        await rpc("sftp/extract", { sessionId, archivePath, destinationPath: base, overwrite: true, sudo: sudoUpload() }, { timeoutMs: 10 * 60 * 1000 });
        // 归档是传输载体：解包成功即清理（sudo 车道 sudo/remove），失败保留
        // 供手工解包。
        await rpc(sudoUpload() ? "sudo/remove" : "sftp/delete", { sessionId, path: archivePath }).catch(() => undefined);
        await refreshUploadedTarget(base);
        showNotice(t("folderArchive.completed", { count: archiveEntries.length, name: archiveFolderName }));
      } catch (cause) {
        showError(cause, "sftp");
      }
      return;
    }
    showNotice(t("folderArchive.tooLarge"));
  }
  openTransferPanel();
  let progress = createFolderUploadProgress(uploadPlan);
  const publish = (currentFile = "") => {
    folderUploadProgress.value = { ...progress, currentFile };
  };
  publish();
  // 逐目录 ensure：集合已去重且父先于子；单目录失败不阻断（文件上传会
  // 因目录缺失自然失败并计入 failed），创建失败只降级提示。
  for (const relative of uploadPlan.directories) {
    const remotePath = joinRemote(base, relative);
    try {
      // sudo 车道建目录换 sudo/mkdir（参数族同用 path），父目录不可写时
      // SFTP createDirectory 必失败。
      const mkdirMethod = sudoUpload() ? "sudo/mkdir" : "sftp/createDirectory";
      await rpc(mkdirMethod, { sessionId, path: remotePath });
    } catch {
      // 已存在/权限不足等：由后续文件上传结果兜底，这里不中止整批。
    }
    progress = advanceFolderUploadDirectories(progress);
    publish();
  }
  let skipped = 0;
  let failed = 0;
  // 逐文件上传走 runTransfers（评审 M-4）：尊重批次并发偏好，不再纯串行；
  // 聚合进度（文件 N/M · 当前相对路径）在并发 run 里同步推进（单线程事件
  // 循环内 read-modify-write 无交错）。单文件失败计 failed 不中止整批——
  // catch 吞掉不外抛，runTransfers 的失败中止语义不触发；用户取消同理
  // （计 failed，批次继续，与既有串行语义一致）。
  await runTransfers(uploadPlan.files.map((file, index) => ({ file, entry: file.source, index })), batchTransferLimit(), {
    id: (item) => `folder-${item.index}`,
    run: async (item) => {
      const { file, entry } = item;
      progress.currentFile = file.relativePath;
      publish(file.relativePath);
      const segments = file.relativePath.split("/");
      const dirSegments = segments.slice(0, -1);
      const fileName = segments[segments.length - 1];
      const targetDir = dirSegments.length ? joinRemote(base, dirSegments.join("/")) : base;
      const root = folderUploadRootPath(file.relativePath);
      try {
        if (root && overwritten.has(root)) {
          // 根目录已确认覆盖并递归删除：不再做逐文件 exists 预检/弹窗
          //（同一文件夹只在根级问一次）。
          await uploadSource(fileName, file.size, entry.readChunk, undefined, targetDir, { duplicatePreCheckedAbsent: true });
        } else if (options?.forceAskDuplicates) {
          // 拖拽批次的逐文件回退：同名必问，且失败关闭（预检失败交回
          // uploadSource 的常规策略，不静默覆盖）。
          const resolved = await resolveUploadDuplicateName(fileName, targetDir, { forceAskDuplicates: true });
          if (!resolved.proceed) {
            skipped += 1;
            progress = settleFolderUploadFile(progress, { file, ok: false });
            publish();
            return;
          }
          await uploadSource(resolved.name, file.size, entry.readChunk, undefined, targetDir, { duplicatePreCheckedAbsent: resolved.prechecked === true });
        } else if (loadTransferDuplicatePolicy() === "ask") {
          // ask 模式批量降级：预检已存在则跳过；不存在则带 preCheckedAbsent
          // 上传，免去 uploadSource 内的第二次 exists（评审 M-4）。sudo 车道
          // 预检换 sudo/exists（目标目录不可读时 sftp/exists 恒 false）。
          const existsMethod = sudoUpload() ? "sudo/exists" : "sftp/exists";
          const probe = await rpc<{ exists: boolean }>(existsMethod, { sessionId, path: joinRemote(targetDir, fileName) }).catch(() => ({ exists: false }));
          if (probe.exists === true) {
            skipped += 1;
            progress = settleFolderUploadFile(progress, { file, ok: false });
            publish();
            return;
          }
          await uploadSource(fileName, file.size, entry.readChunk, undefined, targetDir, { duplicatePreCheckedAbsent: true });
        } else {
          await uploadSource(fileName, file.size, entry.readChunk, undefined, targetDir);
        }
        progress = settleFolderUploadFile(progress, { file, ok: true });
      } catch {
        failed += 1;
        progress = settleFolderUploadFile(progress, { file, ok: false });
      }
      publish();
    },
  });
  const outcome = folderUploadOutcome(progress, skipped, failed);
  folderUploadProgress.value = undefined;
  if (outcome.uploaded) await refreshUploadedTarget(base);
  let message: string;
  if (outcome.failed) {
    message = t("folderUpload.completedWithFailures", { count: outcome.failed, total: outcome.fileCount });
  } else {
    message = t("folderUpload.completed", { count: outcome.uploaded });
  }
  if (outcome.skipped) message += t("folderUpload.skippedNote", { count: outcome.skipped });
  showNotice(message);
}

  return {
    folderUploadInput,
    folderUploadSupported,
    folderUploadProgress,
    chooseFolderUpload,
    onFolderUploadInput,
    uploadFolderFiles,
  };
}
