import { describe, expect, it } from "vitest";
import {
  detectLogLevel,
  escapeLogHtml,
  looksLikeLog,
  renderLogLineHtml,
  tokenizeAnsiLine,
  tokenizeLogLine,
  tokenizeOutputLine,
} from "./logTokens";

function kinds(line: string): string[] {
  return tokenizeLogLine(line).tokens.map((token) => token.kind);
}

function textOf(line: string): string {
  return tokenizeLogLine(line).tokens.map((token) => token.text).join("");
}

describe("detectLogLevel", () => {
  it("reads level= / severity: key values", () => {
    expect(detectLogLevel('time="2026-10-29" level=warning msg="slow"')).toBe("warn");
    expect(detectLogLevel('{"level":"error","msg":"boom"}')).toBe("error");
    expect(detectLogLevel("severity: critical something failed")).toBe("error");
  });

  it("reads bracketed, leading and bare uppercase levels", () => {
    expect(detectLogLevel("[INFO] started")).toBe("info");
    expect(detectLogLevel("WARN: disk almost full")).toBe("warn");
    expect(detectLogLevel("2026-10-29 12:00:00 ERROR request failed")).toBe("error");
    expect(detectLogLevel("INFO:root:ready")).toBe("info");
  });

  it("does not treat lowercase prose as a level", () => {
    expect(detectLogLevel("this error happened because of a warning")).toBeNull();
    expect(detectLogLevel("plain output line")).toBeNull();
  });
});

describe("tokenizeLogLine", () => {
  it("colors timestamps, levels and numbers without losing text", () => {
    const line = "2026-10-29T12:00:00.123Z INFO accepted 512 bytes";
    expect(textOf(line)).toBe(line);
    expect(kinds(line)).toEqual(["time", "plain", "level-info", "plain", "number", "plain"]);
  });

  it("colors syslog timestamps", () => {
    const line = "Oct 29 12:00:00 host sshd[42]: Accepted publickey";
    expect(textOf(line)).toBe(line);
    expect(kinds(line)[0]).toBe("time");
  });

  it("treats JSON keys and level values", () => {
    const line = '{"level":"warn","count":3,"ok":true}';
    expect(textOf(line)).toBe(line);
    const tokens = tokenizeLogLine(line).tokens;
    expect(tokens.find((token) => token.kind === "key")).toBeDefined();
    const level = tokens.find((token) => token.kind === "level-warn");
    expect(level?.text).toBe('"warn"');
    expect(tokens.some((token) => token.kind === "number")).toBe(true);
    expect(tokens.some((token) => token.kind === "bool")).toBe(true);
  });

  it("colors key=value keys, quoted strings, IPs and URLs", () => {
    const line = 'upstream=10.0.0.8 url=https://example.com/a?b=1 msg="all good"';
    expect(textOf(line)).toBe(line);
    const tokens = tokenizeLogLine(line).tokens;
    expect(tokens.filter((token) => token.kind === "key").map((token) => token.text)).toEqual([
      "upstream",
      "url",
      "msg",
    ]);
    expect(tokens.some((token) => token.kind === "ip" && token.text === "10.0.0.8")).toBe(true);
    expect(tokens.some((token) => token.kind === "url")).toBe(true);
    expect(tokens.some((token) => token.kind === "string" && token.text === '"all good"')).toBe(true);
  });

  it("dims stack frames and colors traceback headers", () => {
    expect(tokenizeLogLine('    at handler (/srv/app.js:10:3)').tokens[0].kind).toBe("stack");
    const traceback = tokenizeLogLine("Traceback (most recent call last):");
    expect(traceback.tokens[0].kind).toBe("level-error");
    expect(traceback.level).toBe("error");
  });

  it("keeps pathological long lines as one plain token", () => {
    const line = "x".repeat(5000);
    const { tokens } = tokenizeLogLine(line);
    expect(tokens).toEqual([{ kind: "plain", text: line }]);
  });
});

describe("tokenizeAnsiLine", () => {
  it("returns null for lines without escapes", () => {
    expect(tokenizeAnsiLine("plain")).toBeNull();
    expect(tokenizeOutputLine("ERROR boom")[0].kind).toBe("level-error");
  });

  it("maps SGR colors and strips the escape sequences", () => {
    const tokens = tokenizeAnsiLine("\u001b[31mred\u001b[0m normal \u001b[1mbold\u001b[0m")!;
    expect(tokens.map((token) => token.text).join("")).toBe("red normal bold");
    expect(tokens[0]).toEqual({ kind: "ansi-red", text: "red" });
    expect(tokens.some((token) => token.kind === "ansi-bold" && token.text === "bold")).toBe(true);
  });

  it("supports bright, 256-color and stripped OSC titles", () => {
    const bright = tokenizeAnsiLine("\u001b[92mgreen\u001b[39m")!;
    expect(bright[0].kind).toBe("ansi-bright-green");
    const indexed = tokenizeAnsiLine("\u001b[38;5;196mhot\u001b[0m")!;
    expect(indexed[0].kind).toBe("ansi-red");
    const osc = tokenizeAnsiLine("\u001b]0;title\u0007kept")!;
    expect(osc.map((token) => token.text).join("")).toBe("kept");
  });
});

describe("looksLikeLog", () => {
  it("accepts level-shaped and timestamped output", () => {
    expect(looksLikeLog("INFO boot\nWARN slow\nERROR boom")).toBe(true);
    expect(
      looksLikeLog(
        "2026-10-29 12:00:00 starting\n2026-10-29 12:00:01 ready\n2026-10-29 12:00:02 done\n2026-10-29 12:00:03 bye",
      ),
    ).toBe(true);
  });

  it("rejects prose and code", () => {
    expect(looksLikeLog("hello\nworld")).toBe(false);
    expect(looksLikeLog("const a = 1;\nconsole.log(a);")).toBe(false);
    expect(looksLikeLog("one line only")).toBe(false);
  });
});

describe("renderLogLineHtml", () => {
  it("escapes markup and wraps colored segments", () => {
    const html = renderLogLineHtml('ERROR <script>alert("x")</script>');
    expect(html).toContain('<span class="log-level-error">ERROR</span>');
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });

  it("escapes plain text through escapeLogHtml", () => {
    expect(escapeLogHtml("a & b < c > d")).toBe("a &amp; b &lt; c &gt; d");
  });
});
