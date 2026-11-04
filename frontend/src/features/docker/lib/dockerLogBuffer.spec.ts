import { describe, expect, it } from "vitest";
import {
  FOLLOW_PAUSE_PX,
  FOLLOW_RESUME_PX,
  LogBuffer,
  MAX_LOG_CHARS,
  MAX_LOG_LINES,
  shouldFollow,
} from "./dockerLogBuffer";

describe("LogBuffer", () => {
  it("returns only the lines completed by each chunk", () => {
    const buffer = new LogBuffer();
    expect(buffer.append("hel")).toEqual([]);
    expect(buffer.append("lo\nwor")).toEqual(["hello"]);
    expect(buffer.append("ld\nsecond\n")).toEqual(["world", "second"]);
    expect(buffer).toBeDefined();
  });

  it("joins partial lines across chunks", () => {
    const buffer = new LogBuffer();
    buffer.append("hel");
    buffer.append("lo\nworld");
    expect(buffer.text()).toBe("hello\nworld");
    expect(buffer.lineCount).toBe(2);
  });

  it("keeps complete lines without a trailing newline", () => {
    const buffer = new LogBuffer();
    buffer.append("one\n");
    expect(buffer.text()).toBe("one");
    expect(buffer.lineCount).toBe(1);
    buffer.append("two\n");
    expect(buffer.text()).toBe("one\ntwo");
    expect(buffer.lineCount).toBe(2);
  });

  it("counts each appended chunk exactly once", () => {
    const buffer = new LogBuffer();
    buffer.append("a");
    buffer.append("\nb");
    buffer.append("\n");
    expect(buffer.text()).toBe("a\nb");
    expect(buffer.lineCount).toBe(2);
  });

  it("ignores empty chunks and keeps the pending partial", () => {
    const buffer = new LogBuffer();
    buffer.append("");
    buffer.append("tail-marker");
    expect(buffer.text()).toBe("tail-marker");
  });

  it("trims from the head at the line cap", () => {
    const buffer = new LogBuffer();
    buffer.append(Array.from({ length: MAX_LOG_LINES + 2 }, (_, i) => `line-${i}`).join("\n"));
    expect(buffer.lineCount).toBe(MAX_LOG_LINES);
    expect(buffer.text().startsWith("line-2\n")).toBe(true);
    expect(buffer.truncated).toBe(true);
  });

  it("returns only the retained tail of a burst larger than the cap", () => {
    const buffer = new LogBuffer();
    const total = MAX_LOG_LINES * 5;
    const added = buffer.append(Array.from({ length: total }, (_, i) => `line-${i}`).join("\n") + "\n");
    expect(added).toHaveLength(MAX_LOG_LINES);
    expect(added[0]).toBe(`line-${total - MAX_LOG_LINES}`);
    expect(added.at(-1)).toBe(`line-${total - 1}`);
    expect(buffer.text()).not.toContain("line-0\n");
  });

  it("does not flag truncation below the caps", () => {
    const buffer = new LogBuffer();
    buffer.append(Array.from({ length: 10 }, (_, i) => `line-${i}`).join("\n"));
    expect(buffer.truncated).toBe(false);
    expect(buffer.lineCount).toBe(10);
  });

  it("trims from the head at the character cap", () => {
    const buffer = new LogBuffer();
    const line = "x".repeat(1_000);
    const count = Math.ceil(MAX_LOG_CHARS / line.length) + 50;
    buffer.append(Array.from({ length: count }, () => line).join("\n"));
    expect(buffer.truncated).toBe(true);
    expect(buffer.text().length).toBeLessThanOrEqual(MAX_LOG_CHARS);
    expect(buffer.text().endsWith(line)).toBe(true);
    expect(buffer.lineCount).toBeLessThan(count);
  });

  it("trims a single partial line that exceeds the character cap", () => {
    const buffer = new LogBuffer();
    // 没有换行：整段都是一个 partial，丢行策略无法把它裁到上限内。
    buffer.append("y".repeat(MAX_LOG_CHARS + 5_000));
    expect(buffer.truncated).toBe(true);
    expect(buffer.lineCount).toBe(1);
    expect(buffer.text().length).toBeLessThanOrEqual(MAX_LOG_CHARS);
    expect(buffer.text().endsWith("y")).toBe(true);
  });

  it("clears lines, pending partial and the truncation flag", () => {
    const buffer = new LogBuffer();
    buffer.append(Array.from({ length: MAX_LOG_LINES + 5 }, (_, i) => `l${i}`).join("\n"));
    buffer.append("\npartial");
    expect(buffer.truncated).toBe(true);
    buffer.clear();
    expect(buffer.text()).toBe("");
    expect(buffer.lineCount).toBe(0);
    expect(buffer.truncated).toBe(false);
  });
});

describe("shouldFollow", () => {
  it("detects follow from scroll distance", () => {
    expect(shouldFollow(900, 100, 1000)).toBe(true);
    expect(shouldFollow(850, 100, 1000)).toBe(false);
  });

  it("resumes within the 4px resume tolerance", () => {
    expect(FOLLOW_RESUME_PX).toBe(4);
    expect(shouldFollow(896, 100, 1000)).toBe(true);
    expect(shouldFollow(895, 100, 1000)).toBe(false);
  });

  it("keeps the pause threshold wider than the resume threshold", () => {
    expect(FOLLOW_PAUSE_PX).toBe(24);
    expect(FOLLOW_PAUSE_PX).toBeGreaterThan(FOLLOW_RESUME_PX);
  });
});
