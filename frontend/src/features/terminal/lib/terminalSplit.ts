/**
 * Split terminal protocol helpers.
 *
 * A split pane is a second PTY channel on the SAME authenticated SSH transport
 * (no second dial, no second MFA prompt). The wire format is the existing
 * terminal frame shape: 1 byte stream tag + big-endian u64 sequence + data.
 *
 * Everything in this module is pure so frame parsing, replay ordering and the
 * open-request shape can be unit-tested without a host bridge or xterm.
 */

/** The split session owns a distinct workbench id so a webview reload can
 * never attach the orphan split PTY in place of the main workbench PTY. */
export const SPLIT_SESSION_WORKBENCH_SUFFIX = "#split";
export const SPLIT_DEFAULT_COLS = 80;
export const SPLIT_DEFAULT_ROWS = 24;
export const SPLIT_OUTPUT_CHANNEL_PREFIX = "ssh/terminal/out/";
export const SPLIT_INPUT_CHANNEL_PREFIX = "ssh/terminal/in/";
/** TerminalFrame stream tag used for in-band state/disconnect messages. */
export const SPLIT_STATE_STREAM_TAG = 2;
/** Bound the out-of-order buffer; a replay repairs anything dropped past it. */
export const SPLIT_PENDING_FRAME_LIMIT = 1024;

const FRAME_HEADER_BYTES = 9;

export interface SplitOutputFrame {
  /** 0 = stdout, 1 = stderr, 2 = state/disconnect, other = terminal output. */
  stream: number;
  sequence: number;
  data: Uint8Array;
}

export interface SplitSessionOpenParams {
  connectionId: string;
  workbenchId: string;
  requestedSessionId: string;
  reuseAuthenticatedTransport: true;
  reuseAuthenticatedSessionId: string;
  cols: number;
  rows: number;
}

export interface BuildSplitSessionOpenParamsInput {
  connectionId: string;
  workbenchId: string;
  mainSessionId: string;
  requestedSessionId: string;
  cols?: number;
  rows?: number;
}

export interface SplitFrameSequencerOptions {
  /**
   * Called once per detected hole with the last contiguous sequence. The
   * caller replays from there; a replay that fills the hole advances the
   * cursor and re-arms detection for the next hole.
   */
  onGap?(afterSequence: number): void;
  pendingLimit?: number;
}

export interface SplitFrameSequencer {
  /** Accepts parsed frames in arrival order. */
  push(frame: SplitOutputFrame): void;
  /** Clears buffered frames and restarts ordering at `afterSequence`. */
  reset(afterSequence?: number): void;
  readonly lastSequence: number;
  readonly pendingCount: number;
}

/** `${workbenchId}#split` — a distinct workbench scope for the split session. */
export function splitSessionWorkbenchId(workbenchId: string): string {
  return `${workbenchId}${SPLIT_SESSION_WORKBENCH_SUFFIX}`;
}

export function splitSessionOutputChannel(sessionId: string): string {
  return `${SPLIT_OUTPUT_CHANNEL_PREFIX}${sessionId}`;
}

export function splitSessionInputChannel(sessionId: string): string {
  return `${SPLIT_INPUT_CHANNEL_PREFIX}${sessionId}`;
}

/**
 * Parses one binary terminal frame from the host bridge.
 *
 * Returns null for short frames and sequences outside the JS safe-integer
 * range; those are unusable for ordering and must not poison the cursor.
 */
export function parseSplitOutputFrame(bytes: Uint8Array): SplitOutputFrame | null {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength < FRAME_HEADER_BYTES) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const rawSequence = view.getBigUint64(1, false);
  if (rawSequence > BigInt(Number.MAX_SAFE_INTEGER)) return null;
  return {
    stream: bytes[0],
    sequence: Number(rawSequence),
    // Copy: the bridge may reuse the frame buffer after dispatch.
    data: bytes.slice(FRAME_HEADER_BYTES),
  };
}

/**
 * Contiguous frame ordering for one split output stream.
 *
 * Frames are written only when they directly follow the last delivered
 * sequence. Out-of-order frames are buffered, duplicates/stale frames are
 * ignored, and a persistent hole triggers a single replay request until the
 * cursor advances.
 */
export function createSplitFrameSequencer(
  onData: (data: Uint8Array, sequence: number) => void,
  onState?: (state: string, sequence: number) => void,
  options: SplitFrameSequencerOptions = {},
): SplitFrameSequencer {
  const pendingLimit = Math.max(1, Math.floor(options.pendingLimit ?? SPLIT_PENDING_FRAME_LIMIT));
  const decoder = new TextDecoder();
  const pending = new Map<number, SplitOutputFrame>();
  let lastSequence = 0;
  // The cursor for which a replay was already requested. Progress changes it,
  // so a replay that fills part of a hole can request the next repair.
  let gapRequestedAt = -1;

  function requestGapReplay() {
    let firstPending = Number.POSITIVE_INFINITY;
    for (const sequence of pending.keys()) {
      if (sequence < firstPending) firstPending = sequence;
    }
    if (!Number.isFinite(firstPending) || firstPending <= lastSequence + 1) {
      gapRequestedAt = -1;
      return;
    }
    if (gapRequestedAt === lastSequence) return;
    gapRequestedAt = lastSequence;
    options.onGap?.(lastSequence);
  }

  function drain() {
    let frame = pending.get(lastSequence + 1);
    while (frame) {
      pending.delete(lastSequence + 1);
      lastSequence += 1;
      if (frame.stream === SPLIT_STATE_STREAM_TAG) {
        onState?.(decoder.decode(frame.data), lastSequence);
      } else {
        onData(frame.data, lastSequence);
      }
      frame = pending.get(lastSequence + 1);
    }

    if (pending.size > pendingLimit) {
      // A flood must not grow the buffer without bound. Drop what is buffered
      // and let a fresh replay re-deliver from the last contiguous frame.
      pending.clear();
      gapRequestedAt = -1;
      options.onGap?.(lastSequence);
      gapRequestedAt = lastSequence;
      return;
    }

    requestGapReplay();
  }

  return {
    push(frame) {
      const sequence = frame?.sequence;
      if (!Number.isSafeInteger(sequence) || sequence <= 0) return;
      if (sequence <= lastSequence) return;
      if (pending.has(sequence)) return;
      pending.set(sequence, frame);
      drain();
    },
    reset(afterSequence = 0) {
      pending.clear();
      lastSequence = Number.isFinite(afterSequence) && afterSequence > 0 ? Math.floor(afterSequence) : 0;
      gapRequestedAt = -1;
    },
    get lastSequence() {
      return lastSequence;
    },
    get pendingCount() {
      return pending.size;
    },
  };
}

function splitDimension(value: number | undefined, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  const rounded = Math.floor(value);
  return rounded > 0 ? rounded : fallback;
}

/**
 * Builds the `ssh/session/open` request for the split PTY: same connection,
 * same authenticated transport as the main session, a distinct workbench id,
 * and the real measured spawn geometry (80x24 until the pane is measurable).
 */
export function buildSplitSessionOpenParams(input: BuildSplitSessionOpenParamsInput): SplitSessionOpenParams {
  return {
    connectionId: input.connectionId,
    workbenchId: splitSessionWorkbenchId(input.workbenchId),
    requestedSessionId: input.requestedSessionId,
    reuseAuthenticatedTransport: true,
    reuseAuthenticatedSessionId: input.mainSessionId,
    cols: splitDimension(input.cols, SPLIT_DEFAULT_COLS),
    rows: splitDimension(input.rows, SPLIT_DEFAULT_ROWS),
  };
}
