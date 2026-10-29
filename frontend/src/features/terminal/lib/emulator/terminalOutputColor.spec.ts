import { afterEach, describe, expect, it } from "vitest";
import {
  colorizeLine,
  colorizeText,
  createTerminalOutputColorizer,
  isOutputColorizationEnabled,
  isStructuredLine,
  OUTPUT_COLOR_MAX_CHUNK_BYTES,
  setOutputColorizationEnabled,
} from "./terminalOutputColor";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: false });

function bytes(text: string): Uint8Array {
  return encoder.encode(text);
}

function text(data: Uint8Array): string {
  return decoder.decode(data);
}

function colorizer(alternate = false) {
  return createTerminalOutputColorizer({ isAlternateScreen: () => alternate });
}

afterEach(() => {
  setOutputColorizationEnabled(true);
});

describe("isStructuredLine", () => {
  it("accepts logs, JSON, systemd, trees and URL lines", () => {
    expect(isStructuredLine('{"level":"info","msg":"ok"}')).toBe(true);
    expect(isStructuredLine('  "level": "info",')).toBe(true);
    expect(isStructuredLine("2026-10-29T16:40:10+08:00 level=info msg=started")).toBe(true);
    expect(isStructuredLine("10:00:04 WARN  upstream slower than 500ms")).toBe(true);
    expect(isStructuredLine("Oct 29 16:40:10 middleware dockerd[1317]: time=... level=info")).toBe(true);
    expect(isStructuredLine("Active: active (running) since Fri 2026-10-16 16:21:14 CST")).toBe(true);
    expect(isStructuredLine("├─2063 /usr/bin/docker-proxy -proto tcp -host-ip 0.0.0.0")).toBe(true);
    expect(isStructuredLine("Docs: https://docs.docker.com")).toBe(true);
  });

  it("leaves prose, listings and source output alone", () => {
    expect(isStructuredLine("Docker Application Container Engine")).toBe(false);
    expect(isStructuredLine("-rw-r--r--  1 root root  4096 Oct 29 16:40 deploy.sh")).toBe(false);
    expect(isStructuredLine("modified:   frontend/src/app/main.ts")).toBe(false);
    expect(isStructuredLine("const answer = 42;")).toBe(false);
    expect(isStructuredLine("")).toBe(false);
  });
});

describe("colorizeLine", () => {
  it("wraps levels, timestamps, keys and numbers with SGR", () => {
    const line = "2026-10-29T16:40:10Z ERROR request id=42 failed";
    const colored = colorizeLine(line);
    expect(colored).toContain("\x1b[1;91mERROR\x1b[0m");
    expect(colored).toContain("\x1b[32m42\x1b[0m");
    expect(colored).toContain("\x1b[94mid\x1b[0m");
    expect(colored).toContain("\x1b[2m2026-10-29T16:40:10Z\x1b[0m");
  });

  it("keeps CRLF intact and returns plain lines untouched", () => {
    const structured = colorizeLine("Tasks: 346\r");
    expect(structured.endsWith("\r")).toBe(true);
    expect(structured).not.toContain("\n");
    expect(colorizeLine("just words here")).toBe("just words here");
  });

  it("does not double-color lines that already carry escapes", () => {
    const escaped = "\x1b[32mActive: active (running)\x1b[0m";
    // 结构化行判定通过，但 tokenize 把 ESC 当普通文本 → 不再插入新的 SGR。
    expect(colorizeLine(escaped)).toBe(escaped);
  });
});

describe("colorizeText", () => {
  it("colorizes every complete line", () => {
    const input = "INFO up\nplain words\nERROR down\n";
    const out = colorizeText(input);
    expect(out.split("\n")).toHaveLength(4);
    expect(out).toContain("\x1b[94mINFO\x1b[0m");
    expect(out).toContain("plain words");
    expect(out).toContain("\x1b[1;91mERROR\x1b[0m");
  });

  it("keeps text without newlines unchanged", () => {
    expect(colorizeText("INFO no newline")).toBe("INFO no newline");
  });
});

describe("createTerminalOutputColorizer", () => {
  it("colorizes complete lines and passes the trailing partial through raw", () => {
    const output = text(colorizer().transform(bytes("INFO started\nINFO partial")));
    expect(output).toContain("\x1b[94mINFO\x1b[0m started");
    expect(output.endsWith("INFO partial")).toBe(true);
  });

  it("passes through when disabled, in the alternate screen, or on ESC lines", () => {
    const chunk = bytes("INFO hello\n");
    setOutputColorizationEnabled(false);
    expect(colorizer().transform(chunk)).toBe(chunk);
    setOutputColorizationEnabled(true);

    expect(colorizer(true).transform(chunk)).toBe(chunk);

    const prompt = bytes("\x1b]633;A\x07[root@host ~]# ");
    expect(colorizer().transform(prompt)).toBe(prompt);
  });

  it("colors output lines while leaving prompt lines with escapes untouched", () => {
    const chunk = bytes(
      "\x1b]633;A\x07user@host:~$ tail -n 2 app.log\r\n2026-10-29T16:40:10Z INFO ready\r\n",
    );
    const out = text(colorizer().transform(chunk));
    expect(out.startsWith("\x1b]633;A\x07user@host:~$ tail -n 2 app.log\r\n")).toBe(true);
    expect(out).toContain("\x1b[94mINFO\x1b[0m");
    expect(out).toContain("\x1b[2m2026-10-29T16:40:10Z\x1b[0m");
  });

  it("passes through chunks without newlines and oversized chunks", () => {
    const echo = bytes("INFO echo");
    expect(colorizer().transform(echo)).toBe(echo);

    const huge = bytes(`INFO ${"x".repeat(OUTPUT_COLOR_MAX_CHUNK_BYTES)}\n`);
    expect(colorizer().transform(huge)).toBe(huge);
  });

  it("preserves a leading continuation byte left by a split multibyte character", () => {
    const complete = bytes("INFO 中文日志\n");
    const lead = complete.findIndex((byte) => byte >= 0xc0 && byte <= 0xef);
    const splitAt = lead + 1; // 切在「中」字内部，下一块以续字节开头
    const head = complete.subarray(0, splitAt);
    const tail = colorizer().transform(complete.subarray(splitAt));
    expect(tail[0]).toBe(complete[splitAt]);
    const joined = new Uint8Array([...head, ...tail]);
    expect(text(joined)).toBe("INFO 中文日志\n");
    expect(text(joined)).not.toContain("\ufffd");
  });

  it("passes invalid UTF-8 through untouched", () => {
    const invalid = new Uint8Array([0x49, 0x4e, 0x46, 0x4f, 0x20, 0xff, 0xfe, 0x0a]);
    expect(colorizer().transform(invalid)).toBe(invalid);
  });

  it("enforces the per-second budget until reset", () => {
    let clock = 0;
    const instance = createTerminalOutputColorizer({
      isAlternateScreen: () => false,
      now: () => clock,
    });
    const line = bytes(`INFO ${"y".repeat(3900)}\n`);
    let colored = 0;
    for (let index = 0; index < 400; index += 1) {
      if (instance.transform(line) !== line) colored += 1;
    }
    expect(colored).toBeGreaterThan(0);
    expect(colored).toBeLessThan(400);

    // 下一个时间窗恢复着色；reset 同样复位窗口。
    clock += 1000;
    expect(instance.transform(line)).not.toBe(line);
    clock += 1000;
    instance.reset();
    expect(instance.transform(line)).not.toBe(line);
  });

  it("exposes the global switch state", () => {
    setOutputColorizationEnabled(false);
    expect(isOutputColorizationEnabled()).toBe(false);
    setOutputColorizationEnabled(true);
    expect(isOutputColorizationEnabled()).toBe(true);
  });
});
