import { describe, expect, it } from "vitest";
import { readU64 } from "./terminalFrameSequence";

function frameWithSequence(sequence: bigint): Uint8Array {
  const frame = new Uint8Array(9);
  new DataView(frame.buffer).setBigUint64(1, sequence, false);
  return frame;
}

describe("readU64", () => {
  it("matches DataView for sequences around the 32-bit boundary", () => {
    const sequences = [
      0n,
      1n,
      0x7fffffffn,
      0xffffffffn,
      0x1_0000_0000n,
      0x1_2345_6789n,
      // Largest sequence the sidecar can reach while staying exact (< 2^53).
      (1n << 53n) - 1n,
    ];
    for (const sequence of sequences) {
      expect(readU64(frameWithSequence(sequence), 1)).toBe(Number(sequence));
    }
  });

  it("reads the sequence that follows the stream byte", () => {
    const frame = frameWithSequence(0x0102_0304_0506_0708n);
    frame[0] = 2;
    expect(readU64(frame, 1)).toBe(0x0102_0304_0506_0708);
  });
});
