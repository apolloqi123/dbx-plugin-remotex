import type { Terminal } from "@xterm/xterm";
import { resolveTerminalInputRoute } from "../../lib/transfer/terminalTrzsz";
import {
  createTerminalInputQueue,
  terminalInputChannel,
  TerminalInputTimeoutError,
  type TerminalInputQueue,
} from "../../lib/emulator/terminalInputQueue";
import { SERIAL_STREAM_STDIN } from "../../../serial/lib/serialTerminalFrames";
import { isSessionGoneError } from "../../lib/behavior/terminalReconnect";
import {
  EnterLineTracker,
  acceptCommandWithContinuation,
  canCaptureEnterLine,
  echoConfirmsLine,
  extractDesyncedCommand,
  extractEchoedCommand,
} from "../../lib/behavior/terminalEnterCapture";
import { applyLineEditControlChar } from "../../lib/emulator/terminalLineModel";
import { cursorAbsoluteRow } from "../../lib/emulator/terminalAnchor";
import { normalizeTerminalInputBytes } from "../../lib/emulator/terminalInput";

/** 主终端会话态（App 的 terminalState 取值域）。 */
export type TerminalInputSessionState = "connecting" | "connected" | "disconnected" | "error";

/** #33/#71 快速输入丢字母的分层计数（App 的 `__dbxTerminalDiag` reactive 同引用）。 */
export interface TerminalInputDiag {
  keys: number;
  sends: number;
  acks: number;
  errors: number;
  swallowed: number;
}

/**
 * 终端输入与行跟踪：主终端键入路由（传输占用/`#` AI 搜索/行快照与建议刷新）、
 * 有序输入队列（SSH/本地/Telnet 与串口 Stdin 通道）、行缓冲记账与
 * 「回车行」兜底采集（commandHistory 数据源）。
 *
 * 迁移约束（Task 17）：函数体逐字迁移，只把跨模块依赖改为 options——会话态、
 * 传输占用、建议/ghost 刷新、命令标记、断线梯子一律经 getter 惰性读取
 * （本 composable 在 useTerminalInstance 之前装配，其依赖多在后段声明）。
 * 行缓冲 `pendingTerminalInput` 本体仍归 App.vue（useCommandSuggestions /
 * useGhostSuggest / aiSearch 同源读写），经 getter/setter 存取。
 */
export interface TerminalInputOptions {
  /** 主 SSH 会话（useSshConnection 持有）。 */
  session: () => { sessionId: string } | undefined;
  localSession: () => { sessionId: string } | null;
  telnetSession: () => { sessionId: string } | null;
  serialSession: () => { sessionId: string } | null;
  /** 串口 B1 二进制写通道可用性（宿主报错后一次性降级 JSON 兼容路径）。 */
  serialBinaryInput: () => boolean;
  setSerialBinaryInput: (enabled: boolean) => void;
  /** 串口文件上传进行中：协议控制字符窗口内吞掉键入。 */
  serialUploadBusy: () => boolean;
  terminalState: () => TerminalInputSessionState;
  zmodemBusy: () => boolean;
  trzszBusy: () => boolean;
  /** 文件传输占用统一语义（zmodem/trzsz 任一持有终端流）。 */
  terminalTransferBusy: () => boolean;
  /** `#` AI 命令搜索：消费的字节不进 PTY。 */
  handleAiSearchData: (data: string) => boolean;
  /** trzsz 持有流时的输入 filter 入口。 */
  feedTrzszTerminalInput: (data: string) => void;
  /** 丢字分层计数（setup 后段才声明，故惰性读取）。 */
  diag: () => TerminalInputDiag;
  getPendingTerminalInput: () => string;
  setPendingTerminalInput: (value: string) => void;
  /** 引导条的行空判定镜像（promptHintsLineEmpty 在 setup 尾部才声明）。 */
  setPromptHintsLineEmpty: (empty: boolean) => void;
  refreshSuggestionsAfterInput: (data: string, lineBeforeInput: string) => void;
  refreshGhostAfterInput: (data: string) => void;
  /** completionController.lineChanged()：revision 前进作废在途结果并调度防抖。 */
  notifyCompletionLineChanged: () => void;
  terminal: () => Terminal | undefined;
  commandMarkerActive: () => boolean;
  pushTerminalCommandHistory: (command: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  isLocalMode: () => boolean;
  isTelnetMode: () => boolean;
  markLocalExited: (code: number | null) => void;
  /** 传输断开的统一入口（useTerminalTransportWatch）。 */
  reportTransportUnavailable: (sessionId: string) => void;
  isReconnectPending: () => boolean;
  scheduleSessionReconnect: () => void;
}

export function useTerminalInput(options: TerminalInputOptions) {
  const {
    session,
    localSession,
    telnetSession,
    serialSession,
    serialBinaryInput,
    setSerialBinaryInput,
    serialUploadBusy,
    terminalState,
    zmodemBusy,
    trzszBusy,
    terminalTransferBusy,
    handleAiSearchData,
    feedTrzszTerminalInput,
    diag,
    getPendingTerminalInput,
    setPendingTerminalInput,
    setPromptHintsLineEmpty,
    refreshSuggestionsAfterInput,
    refreshGhostAfterInput,
    notifyCompletionLineChanged,
    terminal: terminalGet,
    commandMarkerActive,
    pushTerminalCommandHistory,
    showError,
    isLocalMode,
    isTelnetMode,
    markLocalExited,
    reportTransportUnavailable,
    isReconnectPending,
    scheduleSessionReconnect,
  } = options;

const terminalInputQueue = createTerminalInputQueue({
  // Only SSH sends use the reconnect watcher below. A stalled local PTY or
  // Telnet send must not be mislabeled as an exited local process or SSH loss.
  sendTimeoutMs: (sessionId) => (sessionId === session()?.sessionId ? 8_000 : 0),
  send: (sessionId, payload) => {
    diag().sends += 1;
    // 通道按会话身份选择（telnet: 前缀 / 本地会话 / SSH，见
    // terminalInputChannel）：本地会话误发 ssh 通道会被 sidecar 的 SSH
    // 会话表拒收，表现为终端看似在线却打不进字。
    return window.dbxPlugin.sendBinary(terminalInputChannel(sessionId, localSession()?.sessionId), payload);
  },
  onError: (cause) => {
    diag().errors += 1;
    terminalInputQueue.reset();
    // 本地会话的输入错误（进程退出/会话被回收）直接落退出态，不走 SSH 重连梯子。
    if (isLocalMode()) {
      markLocalExited(null);
      return;
    }
    const sshSessionId = session()?.sessionId;
    if (!isTelnetMode() && sshSessionId && (cause instanceof TerminalInputTimeoutError || isSessionGoneError(cause))) {
      reportTransportUnavailable(sshSessionId);
      return;
    }
    showError(cause, "terminal");
    // 会话被外部杀掉（宿主重推连接的 disconnect、sidecar 重启）时本 tab 无
    // 事件感知，终端看似活着实则打不进字。输入撞上死会话时按传输断开的
    // 同款有界梯子自动重连。错误串契约见 backend ssh.rs session()。
    if (terminalState() === "connected" && !isReconnectPending() && isSessionGoneError(cause)) scheduleSessionReconnect();
  },
});

// 串口专用输入队列：与 SSH/Telnet 共用同一有序实现，但负载带 Stdin 流
// 标签（B1 TerminalFrame 形状），序列号独立计数；宿主对未知通道/方法报错
// 时一次性降级 JSON 兼容路径（serialBinaryInput）。
const serialInputQueue = createTerminalInputQueue({
  frameTag: SERIAL_STREAM_STDIN,
  send: (sessionId, payload) => {
    if (!sessionId.startsWith("serial:")) return;
    return window.dbxPlugin
      .sendBinary(`serial/terminal/in/${sessionId.slice("serial:".length)}`, payload)
      .catch((cause: unknown) => {
        setSerialBinaryInput(false);
        throw cause;
      });
  },
  onError: (cause) => showError(cause, "terminal"),
});

// 主终端键入路由（传输占用/`#` AI 搜索/行快照与建议刷新）：实例只做 onData
// 接线，路由规则留在本 composable（Task 17 收口点）。
// Named handler: main's WKWebView input-loss fallback shares this route.
// Gate is local-terminal aware (a4): no SSH session AND no local shell means
// there is no PTY to receive input.
function routeTerminalData(data: string) {
  if (!session() && !localSession()) return;
  // 文件传输占用路由：trzsz 持有流时，传输中的输入进 filter（Ctrl+C 停传输、
  // 其余吞掉），等待协商期直接吞掉（防止杂散键入干扰 trz 握手）；zmodem 持有
  // 流时输入保持阻塞，否则走普通 PTY 键盘写入（8 字节序号前缀已封装）。
  const route = resolveTerminalInputRoute({ zmodemBusy: zmodemBusy(), trzszBusy: trzszBusy() });
  if (route === "trzsz") {
    feedTrzszTerminalInput(data);
    diag().swallowed += 1;
    return;
  }
  if (route === "blocked") {
    diag().swallowed += 1;
    return;
  }
  // `#` AI 命令搜索（Warp AI Command Search 同位）：等待冻结、模式内编辑与
  // 空行激活统一由共享 composable 判定；消费的字节不进 PTY。
  if (handleAiSearchData(data)) {
    diag().swallowed += 1;
    return;
  }
  // 首帧优化：键入字节先发送，输入记录随后同步完成，bookkeeping 不阻塞发送路径。
  // 命令建议（P1-1）：行快照先于 trackPendingInput 取（\r 会清空行缓冲），
  // 之后按输入事件推进抑制门并刷新浮层。快速命令/粘贴/自动应答不走路由，
  // 天然不会触发浮层，也不会进入采集。
  sendTerminalBytes(new TextEncoder().encode(data));
  const lineBeforeInput = getPendingTerminalInput();
  trackPendingInput(data);
  refreshSuggestionsAfterInput(data, lineBeforeInput);
  refreshGhostAfterInput(data);
  diag().keys += 1;
}

// —— 行改写追踪（issue #192 失真回退采集）：键入模型在 Tab 补全 / ↑ 召回 /
// 光标移动等 shell 改写行的事件后与真实行失真（事件本身不进模型），主路径
// 的「模型行 ⊂ 屏幕行」对照从此永远失配 → 漏采。状态机在 EnterLineTracker
// （lib/terminalEnterCapture.ts）：改写前缀 = 首个改写事件时的模型行（提取
// 锚），改写后键入 = 回显活性证据（关回显的凭据输入永不上屏 → 拒采）。
const enterLineTracker = new EnterLineTracker();
// 反斜杠续行草稿（多行命令）：暂存去续行反斜杠的前段，下一行回车拼接。
let pendingContinuation = "";

function trackPendingInput(data: string) {
  if (data.includes("\u001b")) {
    // ESC 序列块（↑ 召回 / Home / End / 方向键）：readline 会改写整行，键入
    // 模型自此失真——整块跳过模型记账（原行为不变），但改写事件必须记账，
    // 否则回退采集失去全部证据（issue #192：↑ 召回改参后回车不入史）。
    enterLineTracker.noteRewrite(getPendingTerminalInput());
    return;
  }
  for (const character of data) {
    if (character === "\r" || character === "\n" || character === "\u0003") {
      // 回车执行点（Ctrl+C 是清行不是执行）：远端未装 shell integration 的
      // SSH 会话没有 OSC 633 E 帧，手动键入的命令只有这里能入 commandHistory
      // （↑ history 面板的数据源）。门过滤 + 回显对照见 lib/terminalEnterCapture.ts。
      if (character !== "\u0003") captureEnterLine(getPendingTerminalInput());
      setPendingTerminalInput("");
      enterLineTracker.reset();
      // Ctrl+C 放弃当前（可能未完成的续行）命令：续行草稿一并作废。
      if (character === "\u0003") pendingContinuation = "";
    }
    else if (character === "\u007f") {
      setPendingTerminalInput(getPendingTerminalInput().slice(0, -1));
      // 退格与远端行编辑同步（该字符模型与屏幕都不再有），证据尾巴同步削。
      enterLineTracker.noteBackspace();
    }
    else if (character === "\t") {
      // Tab 补全：shell 改写行且补全插入段不进模型（#169 只覆盖「补全后立
      // 即回车」——前缀仍是子串；继续键入即失真）。记改写事件走回退采集。
      enterLineTracker.noteRewrite(getPendingTerminalInput());
    }
    else {
      // readline 行编辑控制字节（Ctrl+U 清行 / Ctrl+W 删词）：回显不可见但
      // 真实行已变，模型必须跟上，否则建议/补全对着死字符检索（#138 review
      // P0-B）。未认领的字节按原逻辑放行（可打印追加，其余忽略）——忽略的
      // 控制字节（Ctrl+A/E/L/R 等）同样是 shell 行改写，记改写事件。
      const edited = applyLineEditControlChar(getPendingTerminalInput(), character);
      if (edited !== null) setPendingTerminalInput(edited);
      else if (character >= " ") {
        setPendingTerminalInput(getPendingTerminalInput() + character);
        enterLineTracker.notePrintable(character);
      }
      else enterLineTracker.noteRewrite(getPendingTerminalInput());
    }
  }
  // 行缓冲变更点（FIG wave-1 锚点）：revision 前进作废在途结果 + 防抖调度
  // （输入门未放行时只作废不调度；常规键入路径随后由
  // refreshSuggestionsAfterInput 的 request("typing") 即时冲掉防抖）。
  notifyCompletionLineChanged();
  // 引导条的行空判定与行缓冲同步推进（pendingTerminalInput 是普通变量，
  // 引导条显隐要响应式，就在这个唯一汇点镜像一份）。
  setPromptHintsLineEmpty(getPendingTerminalInput() === "");
}

/** 回车行兜底采集：门过滤 + 回显对照后写入 commandHistory；与 E 帧/命令
 *  弹窗采集共用 pushTerminalCommandHistory，重复命令由环去重置顶，时间戳
 *  覆盖更新（回车先采、E 帧后到，最终值一致）。
 *  issue #192 两段增强：① 模型失真回退——Tab 补全后继续键入 / ↑ 召回后编
 *  辑的行，模型与屏幕对照必然失配，改以行改写状态（改写前缀锚 + 键入尾巴
 *  回显证据）从屏幕直接提取（extractDesyncedCommand），凭据/选择提示形态
 *  命中即拒；② 反斜杠续行合并——尾奇数反斜杠的行暂存，与下一行拼接成一条
 *  入史（与远端历史文件导入 parseShellHistoryText 同语义），Ctrl+C 作废草稿。 */
function captureEnterLine(line: string) {
  if (
    !canCaptureEnterLine(line, {
      alternateActive: terminalGet()?.buffer.active.type === "alternate",
      transferBusy: terminalTransferBusy() || serialUploadBusy(),
      shellCommandActive: commandMarkerActive(),
    })
  ) {
    return;
  }
  const echo = readEchoTextBeforeCursor();
  // 以屏幕回显提取的命令入史（issue #169）：模型行只含键入前缀，Tab 补全/
  // 历史展开后的真实命令（`cd blen` → `cd blender/`）只在回显里；提取失败
  // （屏幕不可测）回落模型行，不劣于旧行为。模型失配时走失真回退提取
  // （issue #192），两级都拒采则放弃（宁可漏采，不错采）。
  const extracted =
    echoConfirmsLine(echo, line)
      ? extractEchoedCommand(echo, line) ?? line
      : extractDesyncedCommand(echo, enterLineTracker.snapshot());
  if (!extracted) return;
  const { entry, pending } = acceptCommandWithContinuation(pendingContinuation, extracted);
  pendingContinuation = pending;
  if (entry) pushTerminalCommandHistory(entry);
}

/**
 * 读光标前（含向上折行链）的屏幕文本，作回显对照源。拼接含提示符前缀
 * （prompt 边界不可知），对照语义因此是「模型行是屏幕文本的子串」而非全等；
 * buffer 不可测（测试/渲染器未就绪）返回 null，由 echoConfirmsLine 放行。
 */
function readEchoTextBeforeCursor(): string | null {
  const terminal = terminalGet();
  if (!terminal) return null;
  try {
    const buffer = terminal.buffer.active;
    if (buffer.type !== "normal") return null;
    const rowY = cursorAbsoluteRow(buffer);
    const cursorRow = buffer.getLine(rowY);
    if (!cursorRow) return null;
    const parts: string[] = [];
    let text = "";
    for (let x = 0; x < buffer.cursorX; x += 1) text += cursorRow.getCell(x)?.getChars() ?? "";
    parts.push(text);
    let row = cursorRow;
    while (row.isWrapped && rowY - parts.length >= 0) {
      const previous = buffer.getLine(rowY - parts.length);
      if (!previous) break;
      row = previous;
      let wrapped = "";
      for (let x = 0; x < terminal.cols; x += 1) wrapped += row.getCell(x)?.getChars() ?? "";
      parts.unshift(wrapped);
    }
    return parts.join("");
  } catch {
    return null;
  }
}

function sendTerminalBytes(data: Uint8Array) {
  // 串口会话优先：B1 主路径走 `serial/terminal/in/{id}` 二进制写通道（专用
  // 有序队列 + Stdin 流标签帧）；宿主报错（未知通道/旧 sidecar）一次性降级
  // serial/write JSON 兼容路径。
  const serial = serialSession();
  if (serial) {
    // 上传进行中吞掉键入：X/Y/ZMODEM 的 ACK/NAK/CAN 控制字符窗口内，
    // 用户字节会污染协议流（进度状态在终端 overlay 上提示"传输中"）。
    // B1 互斥：二进制写帧与 JSON 键入共用这道前端闸门；sidecar 侧另有
    // 二进制帧拒收后盾。
    if (serialUploadBusy()) return;
    if (serialBinaryInput()) {
      serialInputQueue.enqueue(`serial:${serial.sessionId}`, normalizeTerminalInputBytes(data));
      return;
    }
    const dataBase64 = window.dbxPlugin.encodeBase64(normalizeTerminalInputBytes(data));
    void window.dbxPlugin.invoke("serial/write", { sessionId: serial.sessionId, dataBase64 }).catch((cause) => showError(cause, "terminal"));
    return;
  }
  // Telnet 会话优先：同一终端视图同一时刻只挂一个会话（SSH/本地/Telnet 互斥），
  // telnet: 前缀在队列 send 回调里拆成 telnet/terminal/in/{id} 通道。
  const telnet = telnetSession();
  if (telnet) {
    terminalInputQueue.enqueue(`telnet:${telnet.sessionId}`, normalizeTerminalInputBytes(data));
    return;
  }
  // Ignore typing while an SSH session is reconnecting. Keeping bytes in a
  // stale session queue can replay partial commands after the browser returns.
  const local = localSession();
  if (!local && terminalState() !== "connected") return;
  const sessionId = local?.sessionId ?? session()?.sessionId;
  if (!sessionId) return;
  terminalInputQueue.enqueue(sessionId, normalizeTerminalInputBytes(data));
}

  return {
    sendTerminalBytes,
    routeTerminalData,
    trackPendingInput,
    captureEnterLine,
    readEchoTextBeforeCursor,
    enterLineTracker,
    terminalInputQueue,
    serialInputQueue,
  };
}
