import { computed, nextTick, ref, type Ref } from "vue";
import { supportsBinaryInput } from "../lib/serialTerminalFrames";
import type { SerialConnectOptions } from "../components/SerialConnectDialog.vue";

/** 串口帧流状态（useProtocolFrameBuffers 持有，在 App.vue 中装配于本组合式
 * 函数之后）：会话启停时复位 sequence/pending/未知流标签计数。 */
export interface SerialFrameStreams {
  serialLastSequence: Ref<number>;
  serialPendingFrames: Map<number, { stream: number; data: Uint8Array }>;
  serialUnknownStreamFrames: Ref<number>;
}

/** 连接上下文（设备参数直启串口用）。 */
export interface SerialConnectionContext {
  serialPort?: string;
  serialBaud?: number;
  serialDataBits?: "7" | "8";
  serialParity?: "none" | "even" | "odd";
  serialStopBits?: "1" | "2";
  serialBackspace?: "del" | "ctrl_h";
}

export interface SerialWorkbenchOptions {
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  isDisposed: () => boolean;
  getTerminal: () => { focus(): void } | undefined;
  // A4 恢复外壳态（useLocalShellSession 持有）：从恢复外壳 tab 直起串口时清掉。
  localShellRestored: Ref<boolean>;
  // 装配依赖（声明/装配在本组合式函数之后，或在其他组合式函数内）：一律
  // getter/回调惰性接入；函数内按原名解构，迁出代码保持逐行原样。
  getWorkbenchId: () => Ref<string>;
  getConnection: () => Ref<SerialConnectionContext>;
  getSerialFrameStreams: () => SerialFrameStreams;
  getSerialInputQueue: () => { reset(): void };
  cancelSerialUploadForSessionClose: (sessionId: string | undefined) => void;
  scheduleFit: () => void;
}

/** 串口会话生命周期（P3 + B1 增强，与 Telnet 同款互斥展示；无 replay，掉帧仅
 * 按 pending 上限清空兜底）：会话状态机/启停/二进制输入能力探测/连接驱动直启
 * 收口在本组合式函数。
 *
 * App.vue 只做装配：模板同名绑定经解构透出。帧流/输入队列/上传复位/xterm 实例
 * 均在本组合式函数之后声明或装配，故经 getter 惰性接入——各函数内按原名解构
 * 后，迁出代码（含注释、语句与顺序）保持逐行原样。 */
export function useSerialWorkbench(options: SerialWorkbenchOptions) {
  const {
    showError,
    isDisposed,
    getTerminal,
    localShellRestored,
    getWorkbenchId,
    getConnection,
    getSerialFrameStreams,
    getSerialInputQueue,
    cancelSerialUploadForSessionClose,
    scheduleFit,
  } = options;

// 串口会话（P3 + B1 增强）：与 SSH/本地/Telnet 同款互斥展示，并入
// localUiMode。出帧由读线程单线程递增 sequence；键盘输入主路径走
// `serial/terminal/in/{id}` 二进制写通道（B1，Stdin=3 标签），JSON
// `serial/write` 保留为兼容/降级路径；resize 无协议概念（设计稿 §4）。
const serialSession = ref<{ sessionId: string; port: string; baudRate: number } | null>(null);
const serialDialogOpen = ref(false);
const serialState = ref<"idle" | "running" | "closed">("idle");
const serialError = ref("");
// B1 解码契约：输出帧遇到未知流标签（> Stdin=3）一律静默丢弃并计数。
// B1 能力开关：true = 键盘走二进制写通道。初始值取 serial/start 的
// binaryInput 能力字段（未声明 = 旧 sidecar → JSON 兼容路径）；通道报错
// （未知方法/会话消失）时 send 回调一次性降级 JSON。
const serialBinaryInput = ref(true);
const isSerialMode = computed(() => serialSession.value !== null);
const serialTarget = computed(() => (serialSession.value ? `${serialSession.value.port}@${serialSession.value.baudRate}` : ""));

// —— 串口会话生命周期（P3，与 Telnet 同款互斥展示；无 replay，掉帧仅按
// pending 上限清空兜底）——
// 退出态统一入口：error 为 null 表示 sidecar 未带原因（主动关闭），
// 非空时在退出覆盖层展示（读线程 IO 失败/设备拔线）。

async function startSerialSession(options: SerialConnectOptions) {
  const workbenchId = getWorkbenchId();
  const { serialLastSequence, serialPendingFrames, serialUnknownStreamFrames } = getSerialFrameStreams();
  const serialInputQueue = getSerialInputQueue();
  // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
  if (serialSession.value && serialState.value !== "closed") await closeSerialSession();
  try {
    // 线上字段为 snake_case：SerialStartRequest 未启用 camelCase rename；
    // 响应则由 sidecar 手拼 json!，sessionId/port/baudRate 为 camelCase，
    // binaryInput 为 B1 能力字段（旧 sidecar 缺失 → JSON 兼容路径）。
    const info = await window.dbxPlugin.invoke<{ sessionId: string; port: string; baudRate: number; binaryInput?: boolean }>("serial/start", {
      workbenchId: workbenchId.value,
      port_name: options.portName,
      baud_rate: options.baudRate,
      data_bits: options.dataBits,
      parity: options.parity,
      stop_bits: options.stopBits,
      backspace_mode: options.backspaceMode,
    });
    if (isDisposed()) {
      void window.dbxPlugin.invoke("serial/close", { sessionId: info.sessionId }).catch(() => undefined);
      return;
    }
    serialSession.value = { sessionId: info.sessionId, port: info.port, baudRate: info.baudRate };
    serialState.value = "running";
    serialError.value = "";
    serialLastSequence.value = 0;
    serialPendingFrames.clear();
    serialUnknownStreamFrames.value = 0;
    // 能力探测降级（设计稿 §2）：未声明 binaryInput 的 sidecar 走 JSON
    // serial/write；通道报错时再一次性降级（send 回调）。
    serialBinaryInput.value = supportsBinaryInput(info);
    serialInputQueue.reset();
    // 从 A4 恢复外壳 tab 直接起串口时清掉外壳态，退出覆盖层随即让位。
    localShellRestored.value = false;
    await nextTick();
    scheduleFit();
    getTerminal()?.focus();
  } catch (cause) {
    showError(cause, "terminal");
  }
}

async function closeSerialSession() {
  const { serialPendingFrames, serialUnknownStreamFrames } = getSerialFrameStreams();
  const serialInputQueue = getSerialInputQueue();
  const sessionId = serialSession.value?.sessionId;
  serialSession.value = null;
  serialState.value = "idle";
  serialError.value = "";
  serialPendingFrames.clear();
  serialUnknownStreamFrames.value = 0;
  serialBinaryInput.value = true;
  serialInputQueue.reset();
  // 上传挂在会话上：随会话关闭一并终止（sidecar cancel 幂等）。
  cancelSerialUploadForSessionClose(sessionId);
  if (!sessionId) return;
  await window.dbxPlugin.invoke("serial/close", { sessionId }).catch(() => undefined);
  getTerminal()?.focus();
}

// M32-B：串口连接路由——从连接 config 组装 SerialConnectOptions。设备路径
// 是唯一硬前提（其余参数有 sidecar 默认值）；startSerialSession 内部吞错并
// 落 showError，成功与否以 serialSession 挂载为准。
async function startSerialFromConnection(): Promise<boolean> {
  const connection = getConnection();
  const conn = connection.value;
  if (!conn.serialPort) return false;
  await startSerialSession({
    portName: conn.serialPort,
    baudRate: conn.serialBaud ?? 115200,
    dataBits: conn.serialDataBits ?? "8",
    parity: conn.serialParity ?? "none",
    stopBits: conn.serialStopBits ?? "1",
    backspaceMode: conn.serialBackspace ?? "del",
  });
  return serialSession.value !== null;
}

  return {
    serialSession,
    serialDialogOpen,
    serialState,
    serialError,
    serialBinaryInput,
    isSerialMode,
    serialTarget,
    startSerialSession,
    closeSerialSession,
    startSerialFromConnection,
  };
}
