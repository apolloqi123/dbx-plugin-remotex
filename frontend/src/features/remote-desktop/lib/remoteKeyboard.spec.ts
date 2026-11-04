import { describe, expect, it } from "vitest";
import {
  isLocalOnlyShortcut,
  isMacPlatform,
  isMetaCode,
  isPasteShortcut,
  metaMapsToControl,
  type RemoteKeyChord,
} from "./remoteKeyboard";

function chord(patch: Partial<RemoteKeyChord> = {}): RemoteKeyChord {
  return { key: "c", code: "KeyC", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...patch };
}

describe("remote desktop keyboard policy", () => {
  it("detects macOS hosts", () => {
    expect(isMacPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)")).toBe(true);
    expect(isMacPlatform("Mozilla/5.0 (Windows NT 10.0)", "Win32")).toBe(false);
    expect(isMacPlatform("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64")).toBe(false);
  });

  it("treats Ctrl/Cmd+V as paste, but not Ctrl+Shift+V", () => {
    expect(isPasteShortcut(chord({ code: "KeyV", ctrlKey: true }))).toBe(true);
    expect(isPasteShortcut(chord({ code: "KeyV", metaKey: true }))).toBe(true);
    expect(isPasteShortcut(chord({ code: "KeyV", ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isPasteShortcut(chord({ code: "KeyV", ctrlKey: true, altKey: true }))).toBe(false);
  });

  it("keeps only escape-hatch shortcuts local", () => {
    // Remote editing shortcuts now reach the desktop.
    expect(isLocalOnlyShortcut(chord({ code: "KeyC", ctrlKey: true }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "KeyA", ctrlKey: true }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "KeyZ", ctrlKey: true }), false)).toBe(false);
    // Browser/app escapes stay local; remote F-keys and copy reach the desktop.
    expect(isLocalOnlyShortcut(chord({ code: "F5" }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "F11" }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "F12" }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "KeyC", ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "KeyI", ctrlKey: true, shiftKey: true }), false)).toBe(true);
    expect(isLocalOnlyShortcut(chord({ code: "KeyR", ctrlKey: true }), false)).toBe(true);
  });

  it("keeps macOS Command window controls local", () => {
    expect(isLocalOnlyShortcut(chord({ code: "KeyW", metaKey: true }), true)).toBe(true);
    expect(isLocalOnlyShortcut(chord({ code: "KeyQ", metaKey: true }), true)).toBe(true);
    expect(isLocalOnlyShortcut(chord({ code: "Tab", metaKey: true }), true)).toBe(true);
    expect(isLocalOnlyShortcut(chord({ code: "Digit4", metaKey: true, shiftKey: true }), true)).toBe(true);
    // Cmd+C/A/Z must be forwarded (translated to Ctrl by the surface).
    expect(isLocalOnlyShortcut(chord({ code: "KeyC", metaKey: true }), true)).toBe(false);
    expect(isLocalOnlyShortcut(chord({ code: "KeyA", metaKey: true }), true)).toBe(false);
  });

  it("maps macOS Command to Control and leaves other platforms alone", () => {
    expect(metaMapsToControl(true, chord({ metaKey: true }))).toBe(true);
    expect(metaMapsToControl(true, "MetaLeft")).toBe(true);
    expect(metaMapsToControl(true, "KeyC")).toBe(false);
    expect(metaMapsToControl(false, chord({ metaKey: true }))).toBe(false);
    expect(isMetaCode("MetaRight")).toBe(true);
    expect(isMetaCode("ControlLeft")).toBe(false);
  });
});
