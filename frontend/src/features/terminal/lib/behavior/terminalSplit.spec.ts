import { describe, expect, it, vi } from "vitest";
import {
  buildSplitSessionOpenParams,
  createSplitFrameSequencer,
  parseSplitOutputFrame,
  SPLIT_DEFAULT_COLS,
  SPLIT_DEFAULT_ROWS,
  splitSessionWorkbenchId,
  type SplitOutputFrame,
} from "./terminalSplit";

function frame(sequence: number, text: string, stream = 0): Uint8Array {
  const data = new TextEncoder().encode(text);
  const bytes = new Uint8Array(9 + data.byteLength);
  bytes[0] = stream;
  new DataView(bytes.buffer).setBigUint64(1, BigInt(sequence), false);
  bytes.set(data, 9);
  return bytes;
}

function parsed(sequence: number, text: string, stream = 0): SplitOutputFrame {
  return { stream, sequence, data: new TextEncoder().encode(text) };
}

describe("splitSessionWorkbenchId", () => {
  it("gives the split PTY its own workbench id", () => {
    // The split session is deliberately not attached to the main workbench id:
    // a webview reload must never attach the orphan split instead of the main PTY.
    expect(splitSessionWorkbenchId("workbench-1")).toBe("workbench-1#split");
    expect(splitSessionWorkbenchId("workbench-1")).not.toBe(splitSessionWorkbenchId("workbench-2"));
  });

  it("keeps the suffixed id distinct even for an empty workbench id", () => {
    expect(splitSessionWorkbenchId("")).toBe("#split");
  });
});

describe("parseSplitOutputFrame", () => {
  it("decodes the stream tag, big-endian sequence and payload", () => {
    expect(parseSplitOutputFrame(frame(258, "hello"))).toEqual({
      stream: 0,
      sequence: 258,
      data: new TextEncoder().encode("hello"),
    });
  });

  it("preserves the state stream tag", () => {
    expect(parseSplitOutputFrame(frame(7, "ssh-transport-disconnected", 2))).toEqual({
      stream: 2,
      sequence: 7,
      data: new TextEncoder().encode("ssh-transport-disconnected"),
    });
  });

  it("accepts an empty data payload and a non-zero byteOffset", () => {
    const source = frame(3, "", 1);
    const padded = new Uint8Array(source.byteLength + 2);
    padded.set(source, 2);
    expect(parseSplitOutputFrame(padded.subarray(2))).toEqual({ stream: 1, sequence: 3, data: new Uint8Array() });
  });

  it("rejects frames shorter than the 9-byte header", () => {
    expect(parseSplitOutputFrame(new Uint8Array())).toBeNull();
    expect(parseSplitOutputFrame(new Uint8Array(8))).toBeNull();
    expect(parseSplitOutputFrame(frame(1, "").subarray(0, 8))).toBeNull();
  });

  it("rejects sequences outside the safe integer range", () => {
    const bytes = new Uint8Array(9);
    new DataView(bytes.buffer).setBigUint64(1, BigInt(Number.MAX_SAFE_INTEGER) + 1n, false);
    expect(parseSplitOutputFrame(bytes)).toBeNull();
  });
});

describe("createSplitFrameSequencer", () => {
  it("writes contiguous frames in order and ignores stale duplicates", () => {
    const output: string[] = [];
    const sequencer = createSplitFrameSequencer((data) => output.push(new TextDecoder().decode(data)));

    sequencer.push(parsed(1, "one"));
    sequencer.push(parsed(2, "two"));
    sequencer.push(parsed(1, "duplicate"));
    sequencer.push(parsed(2, "duplicate"));

    expect(output).toEqual(["one", "two"]);
    expect(sequencer.lastSequence).toBe(2);
    expect(sequencer.pendingCount).toBe(0);
  });

  it("buffers out-of-order frames until the gap is filled", () => {
    const output: string[] = [];
    const sequencer = createSplitFrameSequencer((data) => output.push(new TextDecoder().decode(data)));

    sequencer.push(parsed(1, "one"));
    sequencer.push(parsed(3, "three"));
    sequencer.push(parsed(4, "four"));
    expect(output).toEqual(["one"]);
    expect(sequencer.pendingCount).toBe(2);

    sequencer.push(parsed(2, "two"));

    expect(output).toEqual(["one", "two", "three", "four"]);
    expect(sequencer.pendingCount).toBe(0);
  });

  it("ignores frames at or below the last contiguous sequence", () => {
    const onData = vi.fn();
    const sequencer = createSplitFrameSequencer(onData);
    sequencer.push(parsed(1, "one"));
    sequencer.push(parsed(2, "two"));
    sequencer.push(parsed(1, "old"));
    sequencer.push(parsed(0, "zero"));

    expect(onData).toHaveBeenCalledTimes(2);
    expect(sequencer.lastSequence).toBe(2);
  });

  it("decodes state frames and never forwards them as terminal output", () => {
    const onData = vi.fn();
    const onState = vi.fn();
    const sequencer = createSplitFrameSequencer(onData, onState);

    sequencer.push(parsed(1, "ssh-transport-disconnected", 2));

    expect(onState).toHaveBeenCalledWith("ssh-transport-disconnected", 1);
    expect(onData).not.toHaveBeenCalled();
  });

  it("requests a replay once per detected gap after the last contiguous sequence", () => {
    const onGap = vi.fn();
    const sequencer = createSplitFrameSequencer(() => undefined, undefined, { onGap });

    sequencer.push(parsed(1, "one"));
    sequencer.push(parsed(3, "three"));
    sequencer.push(parsed(4, "four"));
    expect(onGap).toHaveBeenCalledTimes(1);
    expect(onGap).toHaveBeenLastCalledWith(1);

    // A replay that does not fill the hole must not turn into a request storm;
    // only real progress changes the replay cursor.
    sequencer.push(parsed(5, "five"));
    expect(onGap).toHaveBeenCalledTimes(1);

    sequencer.push(parsed(2, "two"));
    expect(onGap).toHaveBeenCalledTimes(1);
  });

  it("requests another replay if progress exposes a second gap", () => {
    const onGap = vi.fn();
    const sequencer = createSplitFrameSequencer(() => undefined, undefined, { onGap });

    sequencer.push(parsed(3, "three"));
    expect(onGap).toHaveBeenLastCalledWith(0);

    sequencer.push(parsed(1, "one"));
    // last contiguous advanced to 1 but 2 is still missing.
    expect(onGap).toHaveBeenCalledTimes(2);
    expect(onGap).toHaveBeenLastCalledWith(1);
  });

  it("reset clears pending frames and moves the cursor for a new session", () => {
    const output: string[] = [];
    const sequencer = createSplitFrameSequencer((data) => output.push(new TextDecoder().decode(data)));

    sequencer.push(parsed(5, "old-pending"));
    sequencer.reset(10);
    sequencer.push(parsed(11, "new"));
    sequencer.push(parsed(5, "stale"));

    expect(output).toEqual(["new"]);
    expect(sequencer.lastSequence).toBe(11);
    expect(sequencer.pendingCount).toBe(0);
  });

  it("reset without an argument restarts from sequence 1", () => {
    const output: string[] = [];
    const sequencer = createSplitFrameSequencer((data) => output.push(new TextDecoder().decode(data)));
    sequencer.push(parsed(9, "nine"));
    sequencer.reset();
    sequencer.push(parsed(1, "one"));
    expect(output).toEqual(["one"]);
  });

  it("ignores invalid sequence numbers", () => {
    const onData = vi.fn();
    const sequencer = createSplitFrameSequencer(onData);
    sequencer.push(parsed(0, "zero"));
    sequencer.push(parsed(-1, "negative"));
    sequencer.push(parsed(Number.NaN, "nan"));
    sequencer.push(parsed(Number.MAX_SAFE_INTEGER + 1, "unsafe"));

    expect(onData).not.toHaveBeenCalled();
    expect(sequencer.lastSequence).toBe(0);
  });
});

describe("buildSplitSessionOpenParams", () => {
  it("opens a new PTY on the authenticated main transport", () => {
    expect(
      buildSplitSessionOpenParams({
        connectionId: "conn-1",
        workbenchId: "workbench-1",
        mainSessionId: "main-session-1",
        requestedSessionId: "requested-1",
        cols: 132,
        rows: 43,
      }),
    ).toEqual({
      connectionId: "conn-1",
      workbenchId: "workbench-1#split",
      requestedSessionId: "requested-1",
      reuseAuthenticatedTransport: true,
      reuseAuthenticatedSessionId: "main-session-1",
      cols: 132,
      rows: 43,
    });
  });

  it("falls back to 80x24 until the pane has a measurable size", () => {
    const params = buildSplitSessionOpenParams({
      connectionId: "conn-1",
      workbenchId: "workbench-1",
      mainSessionId: "main-session-1",
      requestedSessionId: "requested-1",
      cols: 0,
      rows: 0,
    });

    expect(params.cols).toBe(SPLIT_DEFAULT_COLS);
    expect(params.rows).toBe(SPLIT_DEFAULT_ROWS);
  });

  it("normalizes fractional and invalid dimensions", () => {
    const params = buildSplitSessionOpenParams({
      connectionId: "conn-1",
      workbenchId: "workbench-1",
      mainSessionId: "main-session-1",
      requestedSessionId: "requested-1",
      cols: 101.9,
      rows: Number.NaN,
    });

    expect(params.cols).toBe(101);
    expect(params.rows).toBe(SPLIT_DEFAULT_ROWS);
  });
});
