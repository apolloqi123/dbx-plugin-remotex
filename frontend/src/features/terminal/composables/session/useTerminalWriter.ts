/**
 * 终端输出管线：大输出背压门与合帧写节流（IMPL_PLAN Task P2-7 / WT-1 / WT-2）、
 * OSC 7 目录跟踪与 OSC 633 命令标记解析、AI 修复输出采集（Warp AI 对齐批）、
 * 提示符换行修复（t8y2/dbx#10750）、zmodem/trzsz 流占用分派、SSH 帧序号重组与
 * 空洞回放，以及宿主二进制事件的总分派（设计稿「terminal/composables/session」
 * 行：输出 gate、写节流、帧排水、二进制分派）。
 *
 * 迁移约束（Task 16）：函数体逐字迁移，只把跨模块依赖改为 options——终端实例
 * （useTerminalInstance 持有）、装饰引擎（后段 composable 才装配）、传输占用与
 * 断线梯子一律经 getter 惰性读取。SSH 帧序号 / pending 缓存仍归 App.vue
 * （useSshConnection、useWorkbenchPersistence、useTerminalTransportWatch 同源
 * 读写同一组 `let`），经 frameStream 的 getter/setter 存取，状态本体不外迁。
 */
import type { Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import { createOutputGate } from "../../lib/emulator/terminalBackpressure";
import { createTerminalOutputColorizer } from "../../lib/emulator/terminalOutputColor";
import { createTerminalWriteThrottle, type TerminalWriteThrottle } from "../../lib/emulator/terminalWriteThrottle";
import type { TerminalSyncOutput } from "../../lib/emulator/terminalModeQueries";
import type { Osc7DirectoryParser } from "../../lib/emulator/terminalDirectoryTracking";
import type { PromptNewlineTransformer } from "../../lib/emulator/terminalPromptNewline";
import { shouldOsc7FollowOverrideUserVarCwd } from "../../lib/emulator/terminalOscChannels";
import type { Osc633CommandParser, Osc633StreamUpdates } from "../../lib/panels/terminalCommandMarkers";
import { readU64 } from "../../lib/emulator/terminalFrameSequence";
import { isSessionGoneError } from "../../lib/behavior/terminalReconnect";
import { detectTrzszAnnounceFromBytes, type TrzszAnnounce } from "../../lib/transfer/terminalTrzsz";
import { isKnownStreamTag } from "../../../serial/lib/serialTerminalFrames";
import type { ReplayResult, SessionInfo } from "../../../connections/lib/sshConnectionTypes";
import type { RdpSessionStateView } from "../../../remote-desktop/lib/rdpFrame";
import { bridgeBinaryBytes } from "../../../../../../shared/frontend/binaryEvent";

/** Upper bound for out-of-order terminal frames held while waiting for the
 * missing sequence; the replay path re-delivers anything dropped beyond it. */
const TERMINAL_PENDING_FRAME_LIMIT = 1024;

/** 主终端会话态（App 的 terminalState 取值域）。 */
export type TerminalSessionState = "connecting" | "connected" | "disconnected" | "error";

/** 乱序帧缓存条目：流标签 + TerminalFrame 9 字节前缀之后的负载。 */
export interface PendingTerminalFrame {
  stream: number;
  data: Uint8Array;
}

/** OSC 633 命令标记态里输出采集读取的最小面。 */
export interface CommandMarkerState {
  readonly active: boolean;
}

/**
 * SSH 帧流状态（App.vue 拥有）：序号游标与回放计数是跨模块共享的 `let`，
 * 这里以 getter/setter 存取，pending Map 按引用共享。
 */
export interface TerminalFrameStream {
  /** 乱序帧缓存（useSshConnection 在会话切换时 clear 同一引用）。 */
  pendingFrames: Map<number, PendingTerminalFrame>;
  /** 已连续消费到的 SSH 帧序号（持久化与 transport watch 同样读写）。 */
  lastSequence: number;
  /** 连续无进展回放计数（useSshConnection 在会话切换时清零）。 */
  replayNoProgress: number;
  /** 当前 SSH 会话 id（open/attach/close 时由 useSshConnection 写入）。 */
  activeSessionId: string;
}

/** 三路同构协议帧流（local/telnet/serial）：状态由 useProtocolFrameBuffers 持有。 */
export interface BinaryFrameStream {
  session: Readonly<Ref<{ sessionId: string } | null | undefined>>;
  lastSequence: Readonly<Ref<number>>;
  pendingFrames: Map<number, PendingTerminalFrame>;
  drain: () => void;
}

export interface SerialBinaryFrameStream extends BinaryFrameStream {
  /** B1：未知流标签帧计数（静默丢帧，不断连）。 */
  unknownStreamFrames: Ref<number>;
}

export interface VncFrameChannel {
  session: Readonly<Ref<{ sessionId: string } | null>>;
  handleFrame: (sessionId: string, payload: Uint8Array) => void;
}

export interface RdpFrameChannel {
  session: Readonly<Ref<{ sessionId: string } | null>>;
  state: Ref<RdpSessionStateView>;
  surface: Readonly<Ref<{ acceptFrame(frame: Uint8Array): boolean } | null>>;
}

/** SFTP 下载分块等待者（8 字节 u64 offset 头 + 负载）。 */
export interface DownloadChunkWaiter {
  offset: number;
  resolve: (bytes: Uint8Array) => void;
  reject: (error: Error) => void;
  timer: number;
}

export interface TerminalWriterOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp", retry?: () => void) => void;

  /** xterm 实例（useTerminalInstance 持有，重建后仍经 getter 取当前实例）。 */
  terminal: () => Terminal | undefined;
  /** zmodem/trzsz 任一占用终端流（备用屏与传输期间不插提示符换行）。 */
  isTransferBusy: () => boolean;

  /** SSH 帧流状态（App 拥有，见 TerminalFrameStream）。 */
  frameStream: TerminalFrameStream;
  session: Ref<SessionInfo | undefined>;
  terminalState: Ref<TerminalSessionState>;
  terminalError: Ref<string>;
  terminalCwd: Ref<string>;
  followDirectory: Ref<boolean>;
  directoryTrackingSupported: Ref<boolean | undefined>;

  /** 会话级 OSC 解析器/变换器（App 与 useCommandMarker 共用同一实例）。 */
  directoryParser: Osc7DirectoryParser;
  promptNewline: PromptNewlineTransformer;
  commandMarkerParser: Osc633CommandParser;
  /** SetUserVar cwd 优先通道的最新上报（用户变量事件，null = 通道未启用）。 */
  getUserVarCwdEvent: () => { path: string; at: number } | null;

  /** 命令标记态与 AI 采集链（useCommandMarker / useAiAssistant / usePromptHints）。 */
  commandMarker: () => CommandMarkerState;
  applyCommandMarker: (updates: Osc633StreamUpdates) => void;
  captureOutputChunk: (data: Uint8Array, activeOrFinishing: boolean) => void;
  maybeOfferAiFix: (updates: Osc633StreamUpdates) => void;
  markTerminalOutputActive: () => void;

  /** 装饰引擎（useTerminalHighlight / ActionLinks / Gutter，后段装配）。 */
  actionLinksEnabled: () => boolean;
  stampGutterWrittenRows: () => void;
  syncSuggestionAnchorsOnSettle: () => void;
  syncGhostAnchorOnSettle: () => void;
  rescanHighlightViewport: () => void;
  scheduleActionLinkScan: (startRow: number, endRow: number) => void;
  scheduleGutterRecompute: () => void;

  /** 传输占用与断线梯子（useZmodem / useTrzsz / useSessionReconnect / transport watch）。 */
  zmodemBusy: () => boolean;
  handleTrzszAnnounce: (announce: TrzszAnnounce) => void;
  ensureTrzszFilter: () => { processServerOutput(data: Uint8Array): void };
  consumeFrameViaZmodem: (data: Uint8Array) => void;
  terminalTransportWatch: () => { reportUnavailable(sessionId: string): void };
  isReconnectPending: () => boolean;
  scheduleSessionReconnect: () => void;
  isDisposed: () => boolean;
  persistState: () => void;

  /** 宿主二进制事件的其余通道（local/telnet/serial/vnc/rdp/sftp 下载）。 */
  localFrames: () => BinaryFrameStream;
  telnetFrames: () => BinaryFrameStream;
  serialFrames: () => SerialBinaryFrameStream;
  vncFrames: () => VncFrameChannel;
  rdpFrames: () => RdpFrameChannel;
  getDownloadChunkWaiters: () => Map<string, DownloadChunkWaiter>;
}

export function useTerminalWriter(options: TerminalWriterOptions) {
  const {
    t,
    showNotice,
    showError,
    terminal: terminalGet,
    isTransferBusy,
    frameStream,
    session,
    terminalState,
    terminalError,
    terminalCwd,
    followDirectory,
    directoryTrackingSupported,
    directoryParser,
    promptNewline,
    commandMarkerParser,
    getUserVarCwdEvent,
    commandMarker,
    applyCommandMarker,
    captureOutputChunk,
    maybeOfferAiFix,
    markTerminalOutputActive,
    actionLinksEnabled,
    stampGutterWrittenRows,
    syncSuggestionAnchorsOnSettle,
    syncGhostAnchorOnSettle,
    rescanHighlightViewport,
    scheduleActionLinkScan,
    scheduleGutterRecompute,
    zmodemBusy,
    handleTrzszAnnounce,
    ensureTrzszFilter,
    consumeFrameViaZmodem,
    terminalTransportWatch,
    isReconnectPending,
    scheduleSessionReconnect,
    isDisposed,
    persistState,
    localFrames,
    telnetFrames,
    serialFrames,
    vncFrames,
    rdpFrames,
    getDownloadChunkWaiters,
  } = options;

  // Large-output rendering throttle: coalesce consecutive PTY frames into one
  // merged xterm write per animation frame (capped, order preserving). The sink
  // reads `terminal` lazily so it also works across terminal recreation. The
  // write completion callback is the gutter timestamp capture point (P1-3): it
  // stamps the logical rows each merged batch actually produced.
  //
  // 大输出保护（IMPL_PLAN Task P2-7）：积压口径 = 已交给 xterm 但 write 回调尚未
  // 触发（还没解析完）的字节数。≥128KiB 进入 strained：合并批次按 32KiB 分帧写、
  // 挂起 gutter/关键词高亮/动作链接扫描；回落到 64KiB 以下自动恢复并补扫一次。
  const outputGate = createOutputGate();
  // 输出着色（tailspin 风格）：按行给结构化明文加 ANSI，备用屏/ESC/大输出透传。
  // 位置在提示符换行修复之后、写节流之前——只改显示，OSC 解析/AI 采集仍吃原始字节。
  const outputColorizer = createTerminalOutputColorizer({
    isAlternateScreen: () => terminalGet()?.buffer?.active?.type === "alternate",
  });
  let outputInFlightBytes = 0;
  function settleOutputChunk(chunk: Uint8Array) {
    outputInFlightBytes = Math.max(0, outputInFlightBytes - chunk.byteLength);
    // 写入完成回调同时是模式复评点；恢复时挂起的扫描由 onOutputGateRelease 补上。
    if (outputGate.feed(outputInFlightBytes) && outputGate.mode === "normal") onOutputGateRelease();
    stampGutterWrittenRows();
    syncSuggestionAnchorsOnSettle();
    syncGhostAnchorOnSettle();
  }
  function onOutputGateRelease() {
    showNotice(t("backpressure.released"));
    rescanHighlightViewport();
    const terminal = terminalGet();
    if (actionLinksEnabled() && terminal) scheduleActionLinkScan(0, terminal.rows - 1);
    scheduleGutterRecompute();
  }
  const terminalWriteThrottle: TerminalWriteThrottle = createTerminalWriteThrottle({
    sink: (data) => {
      const terminal = terminalGet();
      if (!terminal) return;
      outputInFlightBytes += data.byteLength;
      if (outputGate.feed(outputInFlightBytes) && outputGate.mode === "strained") {
        showNotice(t("backpressure.engaged"));
        scheduleGutterRecompute();
      }
      if (outputGate.mode === "strained") {
        for (const frame of outputGate.write(data)) terminal.write(frame, () => settleOutputChunk(frame));
      } else {
        terminal.write(data, () => settleOutputChunk(data));
      }
    },
  });
  // DECSET 2026 同步渲染（WT-1，对标 WezTerm 帧合并）：`CSI ? 2026 h` 期间挂起
  // 合帧通道的定时提交、写入持续入队，`CSI ? 2026 l` 把积压整批合并一次提交——
  // vim 重绘 / cat 大文件在同步窗内的中间帧不再逐帧上屏，撕裂闪烁随之收敛。
  // 积压受合帧通道 1MiB 字节上限约束；hold 另有 100ms 截止时限：PTY 读边界把
  // 同步窗切成两帧时（codex 30fps 小帧流下必然发生），release 自身被 hold 挡在
  // 队列里，不定时放行会一直积到字节帽才渲染——表现为终端长时间"卡死"。
  // 防卡死优先于帧完整，异常驻留同步态时按 deadline 强制放行。
  const terminalSyncOutput: TerminalSyncOutput = {
    begin: () => terminalWriteThrottle.setHold(true),
    end: () => terminalWriteThrottle.setHold(false),
  };

  function writeTerminalOutput(data: Uint8Array) {
    const now = Date.now();
    const userVarCwdEvent = getUserVarCwdEvent();
    for (const path of directoryParser.push(data)) {
      // WT-2：SetUserVar cwd 优先通道在位且刚更新时，提示符 OSC 7 的旧值不回踩
      // （裁决见 shouldOsc7FollowOverrideUserVarCwd；无该通道时行为与从前一致）。
      if (!shouldOsc7FollowOverrideUserVarCwd(userVarCwdEvent, path, now)) continue;
      terminalCwd.value = path;
    }
    // AI 修复输出采集（Warp AI 对齐批）：解码与 OSC 解析同流，命令执行期
    // （active）的输出文本进尾部环形缓冲；快进快出的命令（E/C/D 同一 chunk）
    // 由 D 帧判定一并采集——wasActive 覆盖长输出的中间 chunk，D 帧 chunk 覆盖
    // 尾段正文。剥 ANSI 后入缓冲——633 帧噪声不进 AI 快照。
    markTerminalOutputActive();
    const aiChunkWasActive = commandMarker().active;
    const aiChunkUpdates = commandMarkerParser.push(data);
    applyCommandMarker(aiChunkUpdates);
    // AI 修复输出采集（Warp AI 对齐批）：解析（E 帧清零缓冲）之后的同 chunk
    // 正文照采——执行期 chunk 靠 active 判定，快进快出命令靠 D 帧 chunk 判定。
    captureOutputChunk(data, aiChunkWasActive || commandMarker().active || aiChunkUpdates.lastExitCode != null);
    maybeOfferAiFix(aiChunkUpdates);
    // 提示符换行修复（t8y2/dbx#10750）：OSC 7 帧前按需补 CRLF。备用屏
    // （vim/top 绝对定位重绘）与 zmodem/trzsz 传输期间只跟踪不插入——二进制
    // 显示流与全屏应用的画面不容忍多余换行。directoryParser/采集链仍吃原始
    // 字节：变换器只做插入，帧本身原样保留。
    const terminal = terminalGet();
    const suppressPromptNewline = isTransferBusy() || terminal?.buffer.active.type === "alternate";
    const framed = promptNewline.push(data, { suppressInsert: suppressPromptNewline });
    // 传输期（zmodem/trzsz）二进制显示流不参与着色，避免改写协议数据。
    terminalWriteThrottle.write(isTransferBusy() ? framed : outputColorizer.transform(framed));
  }

  /**
   * Terminal output dispatch: ZMODEM owns the stream while busy; otherwise the
   * frame feeds the trzsz filter, which passes it through to the terminal and
   * watches for the remote `::TRZSZ:TRANSFER:` announce to take over exactly
   * one transfer. Plain frames thus reach the terminal untouched.
   */
  function dispatchTerminalOutput(data: Uint8Array) {
    if (zmodemBusy()) {
      writeTerminalOutput(data);
      return;
    }
    const announce = detectTrzszAnnounceFromBytes(data);
    if (announce) handleTrzszAnnounce(announce);
    ensureTrzszFilter().processServerOutput(data);
  }

  // 本地终端输出与 SSH 同一帧协议（stream + u64 sequence），复用乱序重组与
  // 空洞补发；State 帧到来即落退出态。补发不完整只 resync 到缓冲尾部——
  // 环形缓冲淘汰不可恢复，但活会话不能被误判成已退出。
  //
  // Payloads below are retained as `subarray` views of the event buffer: the
  // host bridge hands every binary event its own transferred buffer, so the
  // view stays valid while referenced (pending frames, browser-save chunk
  // lists). RDP/VNC frames rely on the same contract.
  function handleBinary(event: DbxPluginBinaryEvent) {
    if (event.channel.startsWith("local/terminal/out/")) {
      const local = localFrames();
      const localId = event.channel.slice("local/terminal/out/".length);
      if (!local.session.value || localId !== local.session.value.sessionId) return;
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (payload.length < 9) return;
      const sequence = readU64(payload, 1);
      if (sequence <= local.lastSequence.value) return;
      local.pendingFrames.set(sequence, { stream: payload[0], data: payload.subarray(9) });
      local.drain();
      return;
    }
    if (event.channel.startsWith("telnet/terminal/out/")) {
      // Telnet 输出帧与 local/SSH 同形（9 字节 TerminalFrame 前缀），独立
      // sequence/pending 状态避免与 SSH/本地流互染。
      const telnet = telnetFrames();
      const telnetId = event.channel.slice("telnet/terminal/out/".length);
      if (!telnet.session.value || telnetId !== telnet.session.value.sessionId) return;
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (payload.length < 9) return;
      const sequence = readU64(payload, 1);
      if (sequence <= telnet.lastSequence.value) return;
      telnet.pendingFrames.set(sequence, { stream: payload[0], data: payload.subarray(9) });
      telnet.drain();
      return;
    }
    if (event.channel.startsWith("serial/terminal/out/")) {
      // 串口输出帧与 local/SSH/Telnet 同形（9 字节 TerminalFrame 前缀，stream
      // 恒为 Stdout），独立 sequence/pending 状态避免与其它会话流互染。
      const serial = serialFrames();
      const serialId = event.channel.slice("serial/terminal/out/".length);
      if (!serial.session.value || serialId !== serial.session.value.sessionId) return;
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (payload.length < 9) return;
      // B1 解码契约：未知流标签（> Stdin=3）一律静默丢帧并计数，不断连。
      if (!isKnownStreamTag(payload[0])) {
        serial.unknownStreamFrames.value += 1;
        return;
      }
      const sequence = readU64(payload, 1);
      if (sequence <= serial.lastSequence.value) return;
      serial.pendingFrames.set(sequence, { stream: payload[0], data: payload.subarray(9) });
      serial.drain();
      return;
    }
    if (event.channel.startsWith("vnc/frame/")) {
      // VNC 帧补丁（44 字节头 + RGBA payload，sidecar 单调递增 sequence）：
      // 会话隔离后交给 useVncSession 的帧入口，由 VncSurface 解码 + rAF 合帧；
      // 乱序帧在组件内丢弃。
      const vnc = vncFrames();
      const vncId = event.channel.slice("vnc/frame/".length);
      if (!vnc.session.value || vncId !== vnc.session.value.sessionId) return;
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      vnc.handleFrame(vncId, payload);
      return;
    }
    if (event.channel.startsWith("rdp/frame/")) {
      // RDP 帧补丁：与 vnc/frame 同一 44 字节 patch 头 + RGBA（解码复用
      // vncFrame），sequence 跨重连单调。首个桌面帧同时把状态推到 running。
      const rdp = rdpFrames();
      const rdpId = event.channel.slice("rdp/frame/".length);
      if (!rdp.session.value || rdpId !== rdp.session.value.sessionId) return;
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (rdp.state.value.state !== "running") rdp.state.value = { ...rdp.state.value, state: "running" };
      rdp.surface.value?.acceptFrame(payload);
      return;
    }
    const sessionId = frameStream.activeSessionId || session.value?.sessionId;
    const sshPrefix = "ssh/terminal/out/";
    // Compare without building `ssh/terminal/out/${sessionId}` per frame:
    // this runs on every binary message.
    if (
      sessionId &&
      event.channel.length === sshPrefix.length + sessionId.length &&
      event.channel.startsWith(sshPrefix) &&
      event.channel.endsWith(sessionId)
    ) {
      const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (payload.length < 9) return;
      const sequence = readU64(payload, 1);
      if (sequence <= frameStream.lastSequence) return;
      frameStream.pendingFrames.set(sequence, { stream: payload[0], data: payload.subarray(9) });
      drainTerminalFrames();
      return;
    }
    const downloadChunkWaiters = getDownloadChunkWaiters();
    // No download in flight: skip the prefix probe/slice for every frame.
    if (downloadChunkWaiters.size === 0) return;
    const taskId = event.channel.startsWith("sftp/download/") ? event.channel.slice("sftp/download/".length) : "";
    const waiter = downloadChunkWaiters.get(taskId);
    if (!waiter) return;
    const payload = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
    if (payload.length < 8 || readU64(payload, 0) !== waiter.offset) return;
    window.clearTimeout(waiter.timer);
    downloadChunkWaiters.delete(taskId);
    waiter.resolve(payload.subarray(8));
  }

  let replayInFlight = false;

  function drainTerminalFrames() {
    let frame = frameStream.pendingFrames.get(frameStream.lastSequence + 1);
    while (frame) {
      frameStream.pendingFrames.delete(frameStream.lastSequence + 1);
      frameStream.lastSequence += 1;
      if (frame.stream === 2) {
        const state = new TextDecoder().decode(frame.data);
        if (state === "directory-tracking-unavailable") {
          followDirectory.value = false;
          directoryTrackingSupported.value = false;
          showNotice(t("directoryTrackingUnavailable"));
        } else {
          terminalState.value = "disconnected";
          terminalError.value = state === "ssh-transport-disconnected" ? t("transportDisconnected") : state || t("disconnected");
          if (state === "ssh-transport-disconnected" && session.value?.sessionId) {
            terminalTransportWatch().reportUnavailable(session.value.sessionId);
          }
        }
      } else {
        consumeFrameViaZmodem(frame.data);
      }
      frame = frameStream.pendingFrames.get(frameStream.lastSequence + 1);
    }
    persistState();
    // A stalled gap must not grow the pending map without bound: once the
    // buffer overshoots, drop it and let the replay re-deliver everything
    // after the last in-order sequence.
    if (frameStream.pendingFrames.size > TERMINAL_PENDING_FRAME_LIMIT) {
      frameStream.pendingFrames.clear();
    }
    const firstPending = frameStream.pendingFrames.size
      ? Math.min(...frameStream.pendingFrames.keys())
      : Number.POSITIVE_INFINITY;
    if (Number.isFinite(firstPending) && firstPending > frameStream.lastSequence + 1 && !replayInFlight && session.value) {
      const holeAt = frameStream.lastSequence;
      replayInFlight = true;
      // A session-gone replay failure must resync and reconnect: the hole can
      // never be filled (the session object is gone server-side), so the
      // ssh-transport-disconnected state frame stuck behind it would otherwise
      // freeze the drain forever and the tab keeps claiming "connected".
      let sessionGone = false;
      void window.dbxPlugin.invoke<ReplayResult>("ssh/terminal/replay", { sessionId: session.value.sessionId, afterSequence: frameStream.lastSequence })
        .then((result) => {
          if (!result.complete) {
            terminalState.value = "error";
            terminalError.value = t("sessionUnrecoverable");
            return;
          }
          // The replay returned healthy but the hole below firstPending is still
          // there: those frames are gone for good (e.g. sequence numbering
          // restarted across a reconnect). Retry once more, then resync the
          // cursor past the hole — dropping the missing prefix beats spinning
          // this replay loop forever and freezing the workbench.
          if (frameStream.lastSequence === holeAt) {
            frameStream.replayNoProgress += 1;
            if (frameStream.replayNoProgress >= 3) {
              frameStream.lastSequence = firstPending - 1;
              frameStream.replayNoProgress = 0;
            }
          } else {
            frameStream.replayNoProgress = 0;
          }
        })
        .catch((cause) => {
          if (isSessionGoneError(cause) && firstPending > frameStream.lastSequence) {
            sessionGone = true;
            frameStream.lastSequence = firstPending - 1;
            frameStream.replayNoProgress = 0;
            return;
          }
          showError(cause, "terminal");
        })
        .finally(() => {
          replayInFlight = false;
          drainTerminalFrames();
          // Run after the resync drain above delivered the buffered disconnect
          // state frame (it flips the state off "connected"); the ladder then
          // picks the reconnect up unless one is already pending.
          if (sessionGone && !isReconnectPending() && !isDisposed()) scheduleSessionReconnect();
        });
    }
  }

  /** 卸载兜底：在途写入回调整体作废，保护态复位（原 onBeforeUnmount 顺序不变）。 */
  function disposeOutputPipeline() {
    terminalWriteThrottle.dispose();
    outputColorizer.reset();
    // 终端重建/会话关闭：在途写入回调整体作废，保护态复位，避免旧积压误判。
    outputInFlightBytes = 0;
    outputGate.reset();
  }

  return {
    outputGate,
    outputInFlightBytes: () => outputInFlightBytes,
    settleOutputChunk,
    onOutputGateRelease,
    terminalWriteThrottle,
    terminalSyncOutput,
    writeTerminalOutput,
    dispatchTerminalOutput,
    handleBinary,
    drainTerminalFrames,
    disposeOutputPipeline,
  };
}
