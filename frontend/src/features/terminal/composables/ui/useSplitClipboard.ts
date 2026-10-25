/**
 * Clipboard and selection actions for the split (secondary) terminal pane.
 *
 * These are the context-menu actions that operate on the pane's own xterm
 * instance: copy, paste, select-all, clear and the `sudo -v` refresh. The
 * terminal itself is owned by useSplitTerminalRenderer; this composable only
 * reads it through a getter and restores focus after each action, matching the
 * main terminal menu.
 */
import type { Terminal } from "@xterm/xterm";
import { readClipboardText, writeClipboardText, type ClipboardDeps } from "../../../../shared/lib/clipboardBridge";

export interface SplitClipboardOptions {
  getTerminal: () => Terminal | undefined;
  sendInput: (data: string) => void;
  closeMenu: () => void;
  /** 会话可写 + 连接态/传输占用：sudo 刷新的禁用门。 */
  isConnected: () => boolean;
  isTransferBusy: () => boolean;
  /** 选区变化后同步菜单可用态（hasSelection 由渲染器持有）。 */
  syncSelection: () => void;
}

export function useSplitClipboard(options: SplitClipboardOptions) {
  function clipboardDeps(): ClipboardDeps {
    return {
      bridge: window.dbxPlugin?.clipboard ?? null,
      nativeClipboard: typeof navigator !== "undefined" ? navigator.clipboard ?? null : null,
    };
  }

  function copySelection() {
    const text = options.getTerminal()?.getSelection() ?? "";
    if (!text) return;
    void writeClipboardText(text, clipboardDeps()).catch(() => undefined);
    options.getTerminal()?.focus();
  }

  async function pasteClipboard() {
    try {
      const text = await readClipboardText(clipboardDeps());
      if (!text) return;
      // xterm 的 paste 会按 bracketed-paste 模式包装远端需要的转义序列。
      options.getTerminal()?.paste(text);
    } catch {
      // 沙箱拒绝读剪贴板时保持静默；键盘 Cmd/Ctrl+V 仍走浏览器原生 paste。
    } finally {
      options.getTerminal()?.focus();
    }
  }

  function selectAllTerminal() {
    options.getTerminal()?.selectAll();
    options.syncSelection();
    options.getTerminal()?.focus();
  }

  function clearTerminal() {
    options.getTerminal()?.clear();
    options.getTerminal()?.focus();
  }

  function sendSudoRefresh() {
    if (!options.isConnected() || options.isTransferBusy()) return;
    options.closeMenu();
    options.sendInput("sudo -v\r");
    options.getTerminal()?.focus();
  }

  return {
    copySelection,
    pasteClipboard,
    selectAllTerminal,
    clearTerminal,
    sendSudoRefresh,
  };
}
