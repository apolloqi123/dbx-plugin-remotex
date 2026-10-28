import { describe, expect, it } from "vitest";
import { Osc7DirectoryParser } from "./terminalDirectoryTracking";

const enc = (text: string) => new TextEncoder().encode(text);

describe("Osc7DirectoryParser", () => {
  it("returns nothing for colored output without an OSC frame", () => {
    const parser = new Osc7DirectoryParser();
    expect(parser.push(enc("\u001b[32mok\u001b[0m plain output\r\n"))).toEqual([]);
    // The decoder never diverged: a later OSC frame still parses normally.
    expect(parser.push(enc("prompt\u001b]7;file://host/tmp\u0007"))).toEqual(["/tmp"]);
  });

  it("joins an OSC 7 frame split at the ESC boundary", () => {
    const parser = new Osc7DirectoryParser();
    expect(parser.push(enc("tail\u001b"))).toEqual([]);
    expect(parser.push(enc("]7;file://host/srv/app\u0007next"))).toEqual(["/srv/app"]);
  });

  it("joins a frame split inside the prefix and drops a stray trailing escape", () => {
    const parser = new Osc7DirectoryParser();
    expect(parser.push(enc("\u001b]"))).toEqual([]);
    expect(parser.push(enc("7;file://host/var\u0007"))).toEqual(["/var"]);
    // A trailing ESC that never becomes an OSC must not poison the next frame.
    expect(parser.push(enc("noise\u001b"))).toEqual([]);
    expect(parser.push(enc("more noise"))).toEqual([]);
    expect(parser.push(enc("\u001b]7;file://host/home\u0007"))).toEqual(["/home"]);
  });

  it("handles ST-terminated frames split across chunks", () => {
    const parser = new Osc7DirectoryParser();
    expect(parser.push(enc("\u001b]7;file://host/a\u001b"))).toEqual([]);
    expect(parser.push(enc("\\prompt"))).toEqual(["/a"]);
  });
});
