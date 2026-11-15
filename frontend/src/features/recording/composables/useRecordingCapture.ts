import { watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import { rpc } from "../../../shared/lib/rpc";
import type { CursorMotion, PointerMotion, RecordingMotionData } from "../lib/replayMotion";

/** Bounded terminal-relative pointer / xterm caret sampling while a session
 * recording is active. Only normalized host coordinates and caret cells are
 * sent; no terminal text, keystrokes, clipboard, credentials, paths or
 * OS-global pointer state ever leaves this composable. */
export function useRecordingCapture(options: {
  recordingActive: Ref<boolean>;
  target: Ref<{ kind: "ssh" | "local"; sessionId: string } | undefined>;
  terminalHost: Ref<HTMLElement | undefined>;
  terminal: () => Terminal | undefined;
}): { dispose(): void } {
  let disposed = false;
  let boundHost: HTMLElement | undefined;
  let caretTimer = 0;
  let lastPointerAt = 0;

  const active = () =>
    !disposed && options.recordingActive.value && options.target.value !== undefined;

  function geometry() {
    const terminal = options.terminal();
    const cols = Math.max(1, Math.min(1000, terminal?.cols || 80));
    const rows = Math.max(1, Math.min(1000, terminal?.rows || 24));
    return { cols, rows };
  }

  function pointerSample(event: PointerEvent, host: HTMLElement): PointerMotion {
    const rect = host.getBoundingClientRect();
    const clamp = (value: number) => Math.min(1, Math.max(0, value));
    const { cols, rows } = geometry();
    return {
      kind: "pointer",
      x: rect.width > 0 ? clamp((event.clientX - rect.left) / rect.width) : 0,
      y: rect.height > 0 ? clamp((event.clientY - rect.top) / rect.height) : 0,
      pressed: event.type === "pointerdown" || (event.type === "pointermove" && event.buttons > 0),
      cols,
      rows,
    };
  }

  function caretSample(): CursorMotion | null {
    const terminal = options.terminal();
    const buffer = terminal?.buffer?.active;
    if (!buffer) return null;
    const { cols, rows } = geometry();
    return {
      kind: "cursor",
      col: Math.min(cols - 1, Math.max(0, buffer.cursorX || 0)),
      row: Math.min(rows - 1, Math.max(0, buffer.cursorY || 0)),
      cols,
      rows,
    };
  }

  function send(data: RecordingMotionData) {
    const target = options.target.value;
    if (!active() || !target) return;
    if (target.kind === "local") {
      void rpc("local/recording/event", { sessionId: target.sessionId, data }).catch(() => undefined);
    } else {
      void rpc("ssh/recording/event", { sessionId: target.sessionId, data }).catch(() => undefined);
    }
  }

  function onPointer(event: PointerEvent) {
    if (!active() || !boundHost) return;
    const now = performance.now();
    if (now - lastPointerAt < 50) return; // at most 20 samples/s
    lastPointerAt = now;
    send(pointerSample(event, boundHost));
  }

  function detach() {
    if (boundHost) {
      boundHost.removeEventListener("pointermove", onPointer);
      boundHost.removeEventListener("pointerdown", onPointer);
      boundHost.removeEventListener("pointerup", onPointer);
      boundHost = undefined;
    }
    if (caretTimer) {
      window.clearInterval(caretTimer);
      caretTimer = 0;
    }
  }

  function sync() {
    detach();
    if (!active()) return;
    const host = options.terminalHost.value;
    if (host) {
      boundHost = host;
      host.addEventListener("pointermove", onPointer);
      host.addEventListener("pointerdown", onPointer);
      host.addEventListener("pointerup", onPointer);
    }
    caretTimer = window.setInterval(() => {
      if (!active()) return;
      const data = caretSample();
      if (data) send(data); // at most 10 samples/s
    }, 100);
  }

  const stopWatch = watch(
    [options.recordingActive, options.target, options.terminalHost],
    sync,
    { flush: "post", immediate: true },
  );

  function dispose() {
    if (disposed) return;
    disposed = true;
    stopWatch();
    detach();
  }

  return { dispose };
}
