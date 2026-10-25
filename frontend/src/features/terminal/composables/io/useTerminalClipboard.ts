import { ref, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import { readClipboardText, writeClipboardText, type ClipboardDeps } from "../../../../shared/lib/clipboardBridge";
import { buildPasteConfirmation, type PasteConfirmation } from "../../lib/behavior/dangerousCommands";
import { resolveTerminalPasteText, type TerminalCopyCache } from "../../lib/behavior/terminalInteraction";
import { transformPasteText, type TerminalBehaviorSettings } from "../../lib/appearance/terminalBehavior";

export interface TerminalClipboardOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  terminal: () => Terminal | undefined;
  terminalCopyCache: TerminalCopyCache;
  terminalMenuOpen: Ref<boolean>;
  menuSelectionSnapshot: Ref<string>;
  terminalBehavior: Readonly<Ref<TerminalBehaviorSettings>>;
  terminalTransferBusy: () => boolean;
  session: Readonly<Ref<{ sessionId: string } | undefined>>;
  localSession: Readonly<Ref<{ sessionId: string } | null>>;
  telnetSession: Readonly<Ref<{ sessionId: string } | null>>;
  serialSession: Readonly<Ref<{ sessionId: string } | null>>;
  isDisposed: () => boolean;
  closeHistoryPanel: () => void;
  resetGutterTimestamps: () => void;
  hideGhostSuggestion: () => void;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  trackPendingInput: (data: string) => void;
  sendTerminalBytes: (data: Uint8Array) => void;
}

/**
 * 终端剪贴板与粘贴链：复制/粘贴/全选/清屏、危险粘贴二次确认队列与
 * 沙箱 iframe 的剪贴板降级（插件视图副本 terminalCopyCache 兜底）。
 * OSC 52 远端写剪贴板仍挂在 createTerminal，只经 options 注入副本缓存。
 */
export function useTerminalClipboard(options: TerminalClipboardOptions) {
  const {
    t,
    terminal: terminalGet,
    terminalCopyCache,
    terminalMenuOpen,
    menuSelectionSnapshot,
    terminalBehavior,
    terminalTransferBusy,
    session,
    localSession,
    telnetSession,
    serialSession,
    isDisposed,
    closeHistoryPanel,
    resetGutterTimestamps,
    hideGhostSuggestion,
    showNotice,
    showError,
    trackPendingInput,
    sendTerminalBytes,
  } = options;

const pasteConfirm = ref<PasteConfirmation>();
let pasteConfirmResolver: ((accepted: boolean) => void) | undefined;
let pasteConfirmAskChain: Promise<unknown> = Promise.resolve();

/** 通用剪贴板写入 + 已复制提示（行菜单/侧栏菜单共用；失败走 sftp 错误条）。 */
function copyTextToClipboard(value: string, noticeKey: string, values?: Record<string, string | number>) {
  void writeClipboardText(value, clipboardDeps())
    .then(() => showNotice(t(noticeKey, values)))
    .catch((cause) => showError(cause instanceof Error ? cause : new Error(String(cause))));
}

// 沙箱 iframe 的剪贴板依赖注入：宿主桥是 optional 且现网宿主未提供，
// 缺失/拒绝时由 clipboardBridge 逐级降级（见 lib/clipboardBridge.ts）。
function clipboardDeps(): ClipboardDeps {
  return {
    bridge: window.dbxPlugin.clipboard ?? null,
    nativeClipboard: typeof navigator !== "undefined" ? (navigator as Navigator & { clipboard?: ClipboardDeps["nativeClipboard"] }).clipboard ?? null : null,
  };
}

async function copyTerminalSelection() {
  // 选区以菜单打开瞬间的快照优先：弹层接管焦点后实时 getSelection() 可能
  // 已被清空（clipboard 档直通路径不受影响，仍取实时选区）。
  const text = menuSelectionSnapshot.value || terminalGet()?.getSelection() || "";
  menuSelectionSnapshot.value = "";
  // 无论成败都关菜单：静默早退会让菜单挂着，看起来像"复制无效"。
  terminalMenuOpen.value = false;
  if (!text) {
    showError(new Error(t("terminalCopyNoSelection")), "terminal");
    terminalGet()?.focus();
    return;
  }
  terminalCopyCache.set(text);
  try {
    await writeClipboardText(text, clipboardDeps());
    showNotice(t("terminalCopied"));
  } catch {
    showError(new Error(t("terminalCopyUnavailable")), "terminal");
  }
  terminalGet()?.focus();
}

async function pasteTerminal() {
  terminalMenuOpen.value = false;
  let clipboardText: string | null = null;
  let clipboardReadBlocked = false;
  try {
    clipboardText = await readClipboardText(clipboardDeps());
  } catch {
    // 宿主桥缺失 + 沙箱拒绝读：降级到插件视图内的复制副本（选中复制、
    // 菜单复制、远端 OSC 52 都会写入），XShell 式「选中→右键」因此闭环。
    clipboardReadBlocked = true;
  }
  const text = clipboardReadBlocked
    ? resolveTerminalPasteText({ cachedText: terminalCopyCache.get(), selectionText: terminalGet()?.getSelection() || null })
    : clipboardText || null;
  if (!text) {
    // 无任何可用来源：引导走原生 paste 快捷键（Ctrl+V 走 paste 事件，不依赖读权限）。
    showError(new Error(t("terminalPasteUseShortcut")), "terminal");
    terminalGet()?.focus();
    return;
  }
  await sendConfirmedPaste(text);
}

function interceptTerminalPaste(event: ClipboardEvent) {
  event.preventDefault();
  event.stopPropagation();
  // 粘贴路径不出 ghost：立即消除（onData 不经手粘贴正文，下一次可打印键入重算）。
  hideGhostSuggestion();
  const text = event.clipboardData?.getData("text/plain") || "";
  if (!text) return;
  void sendConfirmedPaste(text);
}

async function sendConfirmedPaste(text: string) {
  // 粘贴文本变换的唯一收口：所有粘贴路径（原生 Ctrl+V、右键/菜单粘贴、中键粘贴）
  // 都经这里，保证「去首尾空白 / 换行折空格」只实现一次、不会分叉。
  // 默认两开关均为关闭，因此这里的默认行为与改动前逐字节一致。
  const payload = transformPasteText(text, terminalBehavior.value);
  if (!payload) return;
  const accepted = await confirmRiskyPaste(payload);
  if (!accepted) {
    terminalGet()?.focus();
    return;
  }
  // 会话门按活动传输判定（sendTerminalBytes 已分派 SSH/本地/Telnet/串口）：
  // 只看 SSH session 会让本地 Shell（及 Telnet/串口）的粘贴静默无效果——
  // 确认弹窗照出、字节却永不落终端（用户感知为「右键粘贴没动静」）。
  if (terminalTransferBusy()) return;
  if (!session.value && !localSession.value && !telnetSession.value && !serialSession.value) return;
  trackPendingInput(payload);
  sendTerminalBytes(new TextEncoder().encode(payload));
  terminalGet()?.focus();
}

function confirmRiskyPaste(text: string): Promise<boolean> {
  // 多行/超长粘贴警告可关（对标 Tabby「Clipboard → Warn on multi-line paste」）；
  // 危险命令（rm -rf 等）的确认是安全兜底，不受该开关约束，永远要确认。
  const confirmation = buildPasteConfirmation(text, { warnOnMultiline: terminalBehavior.value.warnOnMultilinePaste });
  if (!confirmation.required) return Promise.resolve(true);
  const asked = pasteConfirmAskChain.then(
    () =>
      new Promise<boolean>((resolve) => {
        // 工作台已卸载：排队中的询问直接拒绝，不留孤儿弹窗态。
        if (isDisposed()) {
          resolve(false);
          return;
        }
        pasteConfirmResolver = resolve;
        pasteConfirm.value = confirmation;
      }),
  );
  pasteConfirmAskChain = asked.catch(() => undefined);
  return asked;
}

function resolvePasteConfirm(accepted: boolean) {
  pasteConfirm.value = undefined;
  const resolve = pasteConfirmResolver;
  pasteConfirmResolver = undefined;
  resolve?.(accepted);
}

function selectAllTerminal() {
  terminalGet()?.selectAll();
  terminalMenuOpen.value = false;
  terminalGet()?.focus();
}

function clearTerminal() {
  terminalGet()?.clear();
  closeHistoryPanel();
  resetGutterTimestamps();
  terminalMenuOpen.value = false;
  terminalGet()?.focus();
}

  return {
    pasteConfirm,
    copyTextToClipboard,
    clipboardDeps,
    copyTerminalSelection,
    pasteTerminal,
    interceptTerminalPaste,
    sendConfirmedPaste,
    confirmRiskyPaste,
    resolvePasteConfirm,
    selectAllTerminal,
    clearTerminal,
  };
}
