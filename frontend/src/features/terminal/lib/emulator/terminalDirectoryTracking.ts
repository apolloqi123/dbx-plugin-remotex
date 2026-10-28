const OSC7_PREFIX = "\u001b]7;";

/**
 * Byte-level probe before paying for UTF-8 decoding: an OSC starts with
 * `ESC ]`, and a trailing `ESC` may be the first half of one split across
 * frames. Both bytes are ASCII, so multi-byte text can never contain them;
 * colored output (CSI) therefore skips the decoder entirely.
 */
function hasOscStart(bytes: Uint8Array): boolean {
  // Native scan per escape byte: plain output costs one indexOf call instead
  // of a JS byte loop per chunk.
  let index = bytes.indexOf(0x1b);
  while (index !== -1) {
    if (index === bytes.length - 1 || bytes[index + 1] === 0x5d) return true;
    index = bytes.indexOf(0x1b, index + 1);
  }
  return false;
}

/**
 * Longest suffix that can still grow into the OSC 7 prefix. Keeping the old
 * whole 32-char tail forced every later chunk through the decoder forever
 * after the first escape-carrying frame.
 */
function partialOsc7Prefix(text: string): string {
  for (let length = Math.min(OSC7_PREFIX.length - 1, text.length); length > 0; length -= 1) {
    if (text.endsWith(OSC7_PREFIX.slice(0, length))) return OSC7_PREFIX.slice(0, length);
  }
  return "";
}

export class Osc7DirectoryParser {
  private pending = "";
  private readonly decoder = new TextDecoder();

  push(chunk: Uint8Array | string): string[] {
    if (typeof chunk !== "string" && this.pending.length === 0 && !hasOscStart(chunk)) return [];
    this.pending += typeof chunk === "string" ? chunk : this.decoder.decode(chunk, { stream: true });
    const directories: string[] = [];

    while (true) {
      const start = this.pending.indexOf(OSC7_PREFIX);
      if (start < 0) {
        this.pending = partialOsc7Prefix(this.pending);
        break;
      }
      const payloadStart = start + 4;
      const bel = this.pending.indexOf("\u0007", payloadStart);
      const st = this.pending.indexOf("\u001b\\", payloadStart);
      const end = bel < 0 ? st : st < 0 ? bel : Math.min(bel, st);
      if (end < 0) {
        this.pending = this.pending.slice(start);
        break;
      }

      const value = this.pending.slice(payloadStart, end);
      const path = parseOsc7Path(value);
      if (path) directories.push(path);
      this.pending = this.pending.slice(end + (end === st ? 2 : 1));
    }

    return directories;
  }

  reset() {
    this.pending = "";
    this.decoder.decode();
  }
}

export function parseOsc7Path(value: string): string | null {
  if (!value.startsWith("file://")) return null;
  const slash = value.indexOf("/", "file://".length);
  if (slash < 0) return "/";
  try {
    return decodeURIComponent(value.slice(slash)) || "/";
  } catch {
    return value.slice(slash) || "/";
  }
}
