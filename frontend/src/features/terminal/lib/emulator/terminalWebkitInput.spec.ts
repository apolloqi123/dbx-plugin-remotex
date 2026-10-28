import { describe, expect, it } from "vitest";
import { createWebkitInputController, isMacWebkitInputFallbackRequired } from "./terminalWebkitInput";

const keydown = (key: string, keyCode: number, at: number, isComposing = false) => ({
  key,
  keyCode,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  isComposing,
  at,
});
const input = (data: string, at: number, isComposing = false) => ({ data, inputType: "insertText", isComposing, at });

describe("macOS WebKit terminal input fallback", () => {
  it("emits every character when input precedes keyCode=229 keydown", () => {
    const controller = createWebkitInputController();

    expect(controller.input(input("c", 10))).toBe("c");
    expect(controller.keydown(keydown("c", 229, 11))).toBe("suppress");
    expect(controller.input(input("l", 40))).toBe("l");
    expect(controller.keydown(keydown("l", 229, 41))).toBe("suppress");
    expect(controller.input(input("e", 70))).toBe("e");
    expect(controller.keydown(keydown("e", 229, 71))).toBe("suppress");
  });

  it("handles keyCode=229 before input without starting xterm's deferred diff", () => {
    const controller = createWebkitInputController();

    expect(controller.keydown(keydown("x", 229, 10))).toBe("suppress");
    expect(controller.input(input("x", 11))).toBe("x");
  });

  it("leaves normal keydown-first input on xterm's path", () => {
    const controller = createWebkitInputController();

    expect(controller.keydown(keydown("a", 65, 10))).toBe("pass");
    expect(controller.input(input("a", 11))).toBeUndefined();
    expect(controller.keydown(keydown("b", 66, 20))).toBe("pass");
    expect(controller.input(input("b", 21))).toBeUndefined();
  });

  it("does not intercept real composition input or its immediate commit", () => {
    const controller = createWebkitInputController();
    controller.compositionStart();

    expect(controller.keydown(keydown("a", 229, 10, true))).toBe("pass");
    expect(controller.input(input("a", 11, true))).toBeUndefined();
    controller.compositionEnd(20);
    // Within the 5 ms settle window the adapter still stays quiet.
    expect(controller.input(input("a", 21))).toBeUndefined();
  });

  it("suppresses IME-commit ASCII inserts after CJK-to-English switch", () => {
    const controller = createWebkitInputController();
    controller.compositionStart();

    // Simulate typing "dock" while the CJK IME is active: each keystroke
    // arrives as keyCode=229 with isComposing=true.
    for (const ch of "dock") {
      expect(controller.keydown(keydown(ch, 229, 10, true))).toBe("pass");
      expect(controller.input(input(ch, 11, true))).toBeUndefined();
    }

    // User switches to English — composition ends.
    controller.compositionEnd(20);

    // The IME fires insertText input events for the already-committed ASCII
    // characters.  xterm's own IME path emits the full string through onData,
    // so the adapter must suppress these duplicates.
    expect(controller.input(input("d", 30))).toBeUndefined();
    expect(controller.input(input("o", 35))).toBeUndefined();
    expect(controller.input(input("c", 40))).toBeUndefined();
    expect(controller.input(input("k", 45))).toBeUndefined();

    // After the suppression window, normal typing resumes.
    expect(controller.keydown(keydown("x", 88, 200))).toBe("pass");
    expect(controller.input(input("x", 201))).toBeUndefined();
  });

  it("suppresses xterm input processing after CJK IME commit", () => {
    const controller = createWebkitInputController();
    controller.compositionStart();
    for (const ch of "dock") {
      controller.keydown(keydown(ch, 229, 10, true));
    }
    controller.compositionEnd(20);

    // Within the suppression window, xterm's own input processing should be
    // suppressed to prevent double-emission.
    expect(controller.shouldSuppressXtermInput(30)).toBe(true);
    expect(controller.shouldSuppressXtermInput(90)).toBe(true);

    // After the window, xterm should process normally.
    expect(controller.shouldSuppressXtermInput(200)).toBe(false);
  });

  it("does not suppress xterm input without IME keydowns during composition", () => {
    const controller = createWebkitInputController();
    controller.compositionStart();
    controller.compositionEnd(10);

    // No keyCode=229 keydowns during composition — xterm should not be suppressed.
    expect(controller.shouldSuppressXtermInput(50)).toBe(false);
  });

  it("does not suppress normal typing right after compositionEnd without IME keydowns", () => {
    const controller = createWebkitInputController();
    controller.compositionStart();
    controller.compositionEnd(10);

    // No keyCode=229 keydowns happened during composition, so the adapter
    // should NOT suppress — the user is genuinely typing after the switch.
    expect(controller.keydown(keydown("a", 65, 50))).toBe("pass");
    expect(controller.input(input("a", 51))).toBeUndefined();
  });

  it("normalizes WebKit non-breaking space and ignores shortcuts/non-ASCII text", () => {
    const controller = createWebkitInputController();

    expect(controller.input(input("\u00a0", 10))).toBe(" ");
    expect(controller.keydown({ ...keydown("v", 229, 20), metaKey: true })).toBe("pass");
    expect(controller.input(input("你", 30))).toBeUndefined();
  });

  it("only enables the adapter on macOS", () => {
    expect(isMacWebkitInputFallbackRequired("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe(true);
    expect(isMacWebkitInputFallbackRequired("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(false);
  });
});
