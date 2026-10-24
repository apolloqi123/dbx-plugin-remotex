// 录制导出（M14）：GIF 帧抓取与 transcript 落盘都是命令式 xterm/DOM 代码，
// 从 composables/useRecording 拆出，便于单独阅读。状态（导出锁、提示）仍由
// 调用方持有：本模块只接收依赖回调，不做任何全局状态。

import { type Ref } from "vue";
import { encodeGif } from "./gifEncoder";
import { buildTimeline, gifFramePlan, type RecordingSummary, type ReplayEvent } from "./replayScheduler";
import { standaloneArrayBuffer } from "../../transfers/lib/standaloneBuffer";
import { terminalOptionPatch, type TerminalAppearanceState } from "../../terminal/lib/terminalAppearance";
import { resolveTerminalFont, type TerminalFontOverride } from "../../terminal/lib/terminalFont";
import { attachWebglRenderer } from "../../terminal/lib/terminalWebgl";
import { type TerminalThemeLike } from "../../terminal/lib/terminalScheme";
import { buildTranscript, transcriptFileName } from "./transcript";
import { rpc, rpcAvailable } from "../../../shared/lib/rpc";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";

export interface RecordingExportDeps {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  appearance: Ref<{ terminal: { fontSize: number } }>;
  terminalTheme: () => TerminalThemeLike;
  terminalFontOverride: Ref<TerminalFontOverride>;
  terminalAppearance: Ref<TerminalAppearanceState>;
  hostFontFamily: () => string;
  loadDownloadDir: () => string;
  loadDownloadUseDefaultDir: () => boolean;
  askDownloadTarget: (fileName: string) => Promise<{ dir: string; setDefault: boolean } | undefined>;
  applyChosenDirAsDefault: (dir: string) => void;
  resolveDownloadConflictFor: (dir: string, fileName: string) => Promise<"rename" | "overwrite" | undefined>;
  probeLocalCapabilities: () => Promise<{ canSaveLocal: boolean; downloadsDir: string } | undefined>;
  saveHostFile: (chunks: Uint8Array[], fileName: string) => Promise<void>;
  openTransferTarget: (path: string) => Promise<void>;
  revealTransferTarget: (path: string) => Promise<void>;
  loadReplayEvents: (recordingId: string) => Promise<ReplayEvent[]>;
  /** 行内导出的全局锁：多条记录共用，只有发起行显示 Encoding…。 */
  recordingExportingId: Ref<string | null>;
}

// —— M14 录制增强：transcript 导出 ———
// 回放链已把事件分页拉到前端，transcript 在前端拼装（lib/transcript 纯函数）
// 并走既有保存桥（宿主 fileTransfer → sidecar local/saveFile → 浏览器下载），
// 不新增协议面。
export async function exportRecordingTranscript(item: RecordingSummary, deps: RecordingExportDeps) {
  const {
    t, showNotice, showError, loadReplayEvents, recordingExportingId,
    probeLocalCapabilities, saveHostFile, loadDownloadDir, loadDownloadUseDefaultDir,
    askDownloadTarget, applyChosenDirAsDefault, resolveDownloadConflictFor,
    openTransferTarget, revealTransferTarget,
  } = deps;
  if (recordingExportingId.value) return;
  recordingExportingId.value = item.recordingId;
  try {
    const events = await loadReplayEvents(item.recordingId);
    const text = buildTranscript(events);
    if (!text) throw new Error(t("replayExportFailed"));
    const bytes = new TextEncoder().encode(text);
    const fileName = transcriptFileName(item.recordingId);
    const fileTransfer = rpcAvailable() ? window.dbxPlugin.fileTransfer : undefined;
    if (fileTransfer) {
      // 宿主原生保存对话框：用户自选目的地。
      const target = await fileTransfer.beginSave({ name: fileName, contentType: "text/plain", size: bytes.byteLength });
      try {
        // issue #116：同 SFTP/trzsz/GIF 落盘——transfer 列表只收 ArrayBuffer，
        // 交独立 buffer（transcript 导出是 integration 线独有入口，与三处同源）。
        await fileTransfer.write(target.handleId, 0, standaloneArrayBuffer(bytes));
        await fileTransfer.finish(target.handleId);
      } catch (cause) {
        await fileTransfer.cancel(target.handleId).catch(() => undefined);
        throw cause;
      }
      showNotice(t("replayExported"));
      return;
    }
    const local = await probeLocalCapabilities();
    if (local?.canSaveLocal) {
      // sidecar 落盘：默认下载目录（或「每次询问」），冲突走既有协商流。
      let targetDir = "";
      let setDefaultAfter = false;
      if (!loadDownloadUseDefaultDir()) {
        const chosen = await askDownloadTarget(fileName);
        if (chosen === undefined) return;
        targetDir = chosen.dir.trim();
        setDefaultAfter = chosen.setDefault;
      }
      const conflict = await resolveDownloadConflictFor(targetDir, fileName);
      if (conflict === undefined) return;
      const saved = await rpc<{ localPath: string; name: string }>("local/saveFile", {
        name: fileName,
        dataBase64: window.dbxPlugin.encodeBase64(bytes),
        targetDir: targetDir || loadDownloadDir() || undefined,
        conflict: conflict === "overwrite" ? "overwrite" : undefined,
      });
      showNotice(t("downloadedTo", { name: saved.name, path: saved.localPath }), [
        { label: t("openDownloadedFile"), run: () => void openTransferTarget(saved.localPath) },
        { label: t("revealInFolder"), run: () => void revealTransferTarget(saved.localPath) },
      ]);
      if (setDefaultAfter) applyChosenDirAsDefault(targetDir);
      return;
    }
    await saveHostFile([bytes], fileName);
    showNotice(t("replayExported"));
  } catch (cause) {
    showError(cause);
  } finally {
    recordingExportingId.value = null;
  }
}

// GIF 导出管线：离屏 xterm 逐事件重放，按 500ms 事件时间抽帧（封顶 120 帧），
// 每帧从 xterm 画布取像素 → encodeGif。纯前端，无新依赖。回放弹窗与录制
// 列表行内按钮共用；调用方负责 replayExporting 状态与错误呈现。
export async function exportRecordingGif(summary: RecordingSummary, events: readonly ReplayEvent[], deps: RecordingExportDeps) {
  const {
    t, showNotice, appearance, terminalTheme, terminalFontOverride, terminalAppearance, hostFontFamily,
    probeLocalCapabilities, saveHostFile, loadDownloadDir, loadDownloadUseDefaultDir,
    askDownloadTarget, applyChosenDirAsDefault, resolveDownloadConflictFor,
    openTransferTarget, revealTransferTarget,
  } = deps;
  const COLS = 80;
  const ROWS = 24;
  const FRAME_INTERVAL_MS = 500;
  const MAX_FRAMES = 120;
  const fileName = `${summary.recordingId || "session"}.gif`;
  // Open the native save picker before the first await so browsers that require
  // a user gesture keep the permission to choose both directory and filename.
  // DBX hosts without File System Access continue through fileTransfer below.
  let nativeSave: DbxGifSaveFileHandle | undefined;
  if (window.showSaveFilePicker) {
    try {
      nativeSave = await window.showSaveFilePicker({
        suggestedName: fileName,
        types: [{ description: "GIF image", accept: { "image/gif": [".gif"] } }],
      });
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === "AbortError") return;
      throw cause;
    }
  }
  const host = document.createElement("div");
  host.style.cssText = "position:fixed;left:-99999px;top:0;";
  document.body.appendChild(host);
  let term: Terminal | null = null;
  try {
    // 离屏终端与主终端同款外观（配色/字体/字重/行高）：导出的 GIF 必须和用户
    // 屏幕上看到的一致，否则「导出」就失去意义。
    const exportFont = resolveTerminalFont(terminalFontOverride.value, {
      fontFamily: hostFontFamily(),
      fontSize: appearance.value.terminal.fontSize,
    });
    const exportPatch = terminalOptionPatch(terminalAppearance.value.settings);
    term = new Terminal({
      cols: COLS,
      rows: ROWS,
      theme: terminalTheme(),
      fontFamily: exportFont.fontFamily,
      fontSize: exportFont.fontSize,
      fontWeight: exportPatch.fontWeight,
      fontWeightBold: exportPatch.fontWeightBold,
      lineHeight: exportPatch.lineHeight,
      letterSpacing: exportPatch.letterSpacing,
      drawBoldTextInBrightColors: exportPatch.drawBoldTextInBrightColors,
    });
    term.open(host);
    // xterm 6 移除了 canvas 渲染器：DOM 渲染器不产出 canvas，逐帧取像素必须
    // 挂 WebGL renderer。两个此前就存在的坑在此一并修掉：screenElement 下第
    // 一块 canvas 是链接下划线的 2d renderLayer（透明，querySelector 会抓错），
    // 真画布按「能取到 webgl2 上下文」选中（getContext 幂等无副作用）；
    // preserveDrawingBuffer 是 WebglAddon 的构造参数（0.20 beta 起改为 options
    // 对象；默认 false，关闭时合成后回读全零像素），导出终端显式开启——主终端
    // 不取像素，维持默认。GPU 被
    // 禁/context 耗尽挂不上 renderer（DOM 渲染无 canvas）时，走 !screen 分支
    // 给出 replayExportFailed 明确错误，而不是永远空帧。
    attachWebglRenderer(term, () => new WebglAddon({ preserveDrawingBuffer: true }));
    const screen =
      (Array.from(host.querySelectorAll("canvas")) as HTMLCanvasElement[])
        .find((c) => c.getContext("webgl2")) ?? null;
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d");
    if (!screen || !context) throw new Error(t("replayExportFailed"));
    canvas.width = screen.width;
    canvas.height = screen.height;
    const timeline = buildTimeline(events, 1);
    const plan = gifFramePlan(timeline, FRAME_INTERVAL_MS, MAX_FRAMES);
    const frames: Array<{ rgba: Uint8Array; delayMs: number }> = [];
    let written = 0;
    for (const boundary of plan) {
      while (written < boundary) {
        term.write(events[written]!.data);
        written += 1;
      }
      // 等两帧渲染再取像素：正常窗口双 rAF 精确等待；标签页被隐藏等场景
      // rAF 永不回调，用 250ms 定时兜底，导出流程永不悬挂在 Encoding…。
      await new Promise<void>((resolve) => {
        let settled = false;
        const settle = () => {
          if (settled) return;
          settled = true;
          resolve();
        };
        requestAnimationFrame(() => requestAnimationFrame(settle));
        window.setTimeout(settle, 250);
      });
      context.drawImage(screen, 0, 0);
      frames.push({ rgba: new Uint8Array(context.getImageData(0, 0, canvas.width, canvas.height).data), delayMs: FRAME_INTERVAL_MS });
    }
    term.dispose();
    term = null;
    const gif = encodeGif(canvas.width, canvas.height, frames);
    if (nativeSave) {
      const writable = await nativeSave.createWritable();
      await writable.write(gif);
      await writable.close();
    } else if (rpcAvailable() && window.dbxPlugin.fileTransfer) {
      // DBX hosts own the native save dialog here, so the user can choose the
      // destination instead of silently losing the file in an unknown folder.
      const fileTransfer = window.dbxPlugin.fileTransfer;
      const target = await fileTransfer.beginSave({ name: fileName, contentType: "image/gif", size: gif.byteLength });
      if (!target) return; // 用户在原生保存框取消：安静结束，不提示导出成功
      try {
        // issue #116：同 SFTP/trzsz 落盘——transfer 列表只收 ArrayBuffer。
        await fileTransfer.write(target.handleId, 0, standaloneArrayBuffer(gif));
        await fileTransfer.finish(target.handleId);
      } catch (cause) {
        await fileTransfer.cancel(target.handleId).catch(() => undefined);
        throw cause;
      }
      showNotice(t("replayExported"));
    } else {
      // 沙箱 iframe（宿主 fileTransfer 缺失）下的可靠路径：sidecar 落盘到
      // 下载目录（或「每次询问」选择的目录），完成后提示完整路径。
      // web/docker（sidecar 不在本机）走宿主 host.saveFile——iframe 内
      // <a download> 被浏览器静默丢弃（issue #93）。
      const local = await probeLocalCapabilities();
      if (!local?.canSaveLocal) {
        await saveHostFile([gif], fileName);
        showNotice(t("replayExported"));
        return;
      }
      let targetDir = "";
      let setDefaultAfter = false;
      if (!loadDownloadUseDefaultDir()) {
        const chosen = await askDownloadTarget(fileName);
        if (chosen === undefined) return;
        targetDir = chosen.dir.trim();
        setDefaultAfter = chosen.setDefault;
      }
      const conflict = await resolveDownloadConflictFor(targetDir, fileName);
      if (conflict === undefined) return;
      const saved = await rpc<{ localPath: string; name: string }>("local/saveFile", {
        name: fileName,
        dataBase64: window.dbxPlugin.encodeBase64(gif),
        targetDir: targetDir || loadDownloadDir() || undefined,
        conflict: conflict === "overwrite" ? "overwrite" : undefined,
      });
      const savedPath = saved.localPath;
      showNotice(t("downloadedTo", { name: saved.name, path: savedPath }), [
        { label: t("openDownloadedFile"), run: () => void openTransferTarget(savedPath) },
        { label: t("revealInFolder"), run: () => void revealTransferTarget(savedPath) },
      ]);
      if (setDefaultAfter) applyChosenDirAsDefault(targetDir);
      return;
    }
    showNotice(t("replayExported"));
  } finally {
    term?.dispose();
    host.remove();
  }
}
