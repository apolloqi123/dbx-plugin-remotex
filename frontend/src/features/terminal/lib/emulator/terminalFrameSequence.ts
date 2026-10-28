/**
 * Terminal binary frame helpers: the wire prefix is `stream u8 | sequence u64
 * big-endian | payload` (see backend `TerminalFrame::encode`).
 */

/**
 * Reads the big-endian u64 sequence without allocating a DataView. Values the
 * sidecar produces stay far below 2^53, so the high/low split is exact.
 */
export function readU64(bytes: Uint8Array, offset: number): number {
  const high =
    ((bytes[offset] << 24) >>> 0) +
    (bytes[offset + 1] << 16) +
    (bytes[offset + 2] << 8) +
    bytes[offset + 3];
  const low =
    ((bytes[offset + 4] << 24) >>> 0) +
    (bytes[offset + 5] << 16) +
    (bytes[offset + 6] << 8) +
    bytes[offset + 7];
  return high * 0x1_0000_0000 + low;
}

/** Sequence field offset inside a terminal binary frame. */
export const TERMINAL_FRAME_SEQUENCE_OFFSET = 1;
