import { computed, ref } from "vue";
import {
  initialSerialUploadState,
  serialUploadActive,
  serialUploadPercent,
  streamSerialUploadFile,
  type SerialUploadProtocol,
  type SerialUploadUiState,
} from "../lib/serialUpload";

/**
 * 串口文件上传（NyaTerm 对齐 P0-3）：弹窗 + overlay 状态机。起点是
 * SerialUploadDialog 选好文件/协议；文件字节经 File API 分块（≤64KiB）送入
 * sidecar，协议时序完全由 sidecar 引擎驱动；进度由 serial/upload/progress
 * 事件经 usePluginEvents 归并到本 ref。running 期间吞掉键入（协议控制字符
 * 窗口）且禁止并发第二次 upload。
 *
 * 会话关闭时由 App 调 cancelForSessionClose（取消语义与 overlay 按钮不同：
 * 静默复位而不展示 failed）。
 */
export function useSerialUpload(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  getSessionId: () => string | undefined;
}) {
  const { t, showError } = options;

  const serialUpload = ref<SerialUploadUiState>(initialSerialUploadState());
  const serialUploadDialogOpen = ref(false);
  let serialUploadAbortRequested = false;
  const serialUploadBusy = computed(() => serialUploadActive(serialUpload.value));
  const serialUploadOverlayVisible = computed(() => serialUpload.value.phase !== "idle");
  const serialUploadPercentValue = computed(() => serialUploadPercent(serialUpload.value));
  const serialUploadStatusLabel = computed(() => {
    const upload = serialUpload.value;
    if (upload.phase === "complete") return t("serial.upload.complete", { name: upload.fileName });
    if (upload.phase === "failed") return t("serial.upload.failed", { reason: upload.reason });
    return t("serial.upload.running", { name: upload.fileName, percent: serialUploadPercentValue.value });
  });

  async function startSerialUpload(request: { file: File; protocol: SerialUploadProtocol }) {
    const sessionId = options.getSessionId();
    if (!sessionId || serialUploadBusy.value) return;
    serialUploadAbortRequested = false;
    serialUpload.value = {
      phase: "running",
      fileName: request.file.name,
      protocol: request.protocol,
      sent: 0,
      total: request.file.size,
      reason: "",
    };
    try {
      await streamSerialUploadFile(request.file, {
        sessionId,
        protocol: request.protocol,
        fileName: request.file.name,
        bridge: window.dbxPlugin,
        readChunk: async (start, end) => new Uint8Array(await request.file.slice(start, end).arrayBuffer()),
        shouldAbort: () => serialUploadAbortRequested,
      });
    } catch (cause) {
      if ((cause as Error)?.message === "tooLarge") {
        showError(t("serial.upload.tooLarge"), "terminal");
      } else {
        showError(cause, "terminal");
      }
      serialUpload.value = { ...initialSerialUploadState(), phase: "failed", reason: String((cause as Error)?.message ?? cause) };
    }
  }

  // overlay 上的取消：中断本地送数并让 sidecar 发协议取消序列（X/Y: CAN×8，
  // Z: ZDLE×5+BS×5），引擎落 Failed 事件后由 progress 归并到 overlay。
  function cancelSerialUpload() {
    if (!serialUploadBusy.value) return;
    const sessionId = options.getSessionId();
    serialUploadAbortRequested = true;
    if (sessionId) void window.dbxPlugin.invoke("serial/upload/cancel", { sessionId }).catch(() => undefined);
    serialUpload.value = { ...serialUpload.value, phase: "failed", reason: "cancelled" };
  }

  /** 会话关闭连带终止：上传挂在会话上，sidecar cancel 幂等；静默复位，
   *  不把 failed 状态带进下一个会话。sessionId 由调用方在清空会话前捕获。 */
  function cancelForSessionClose(sessionId: string | undefined) {
    if (serialUpload.value.phase === "idle") return;
    serialUploadAbortRequested = true;
    if (sessionId) void window.dbxPlugin.invoke("serial/upload/cancel", { sessionId }).catch(() => undefined);
    serialUpload.value = initialSerialUploadState();
  }

  return {
    serialUpload,
    serialUploadDialogOpen,
    serialUploadBusy,
    serialUploadOverlayVisible,
    serialUploadPercentValue,
    serialUploadStatusLabel,
    startSerialUpload,
    cancelSerialUpload,
    cancelForSessionClose,
  };
}
