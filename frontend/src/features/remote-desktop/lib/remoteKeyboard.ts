/**
 * Remote-desktop keyboard policy shared by the VNC and RDP canvases.
 *
 * The surfaces used to drop every Ctrl/Meta combo (except paste) so the
 * webview kept its own shortcuts, which also made Ctrl+C/A/Z unreachable on
 * the remote desktop. This helper keeps only the shortcuts that must stay
 * local to avoid trapping the user, and maps macOS Command to Control so
 * Cmd+C/Cmd+A reach the remote as Ctrl+C/Ctrl+A.
 */
export interface RemoteKeyChord {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** macOS window/tab controls plus system screenshot shortcuts stay local. */
const MAC_LOCAL_CODES: ReadonlySet<string> = new Set([
  "KeyQ",
  "KeyW",
  "KeyM",
  "KeyH",
  "KeyR",
  "KeyT",
  "KeyN",
  "Tab",
  "Backquote",
  "Space",
]);

/** Browser-level keys stay local only where they are the escape hatch; F5/F11/F12
 * are forwarded because remote BIOS menus/IDEs rely on them (Ctrl+R remains the
 * local reload escape). */

export function isMacPlatform(userAgent: string, platform = ""): boolean {
  return /mac/i.test(platform) || /Mac OS X|Macintosh/i.test(userAgent);
}

export function isPasteShortcut(
  event: Pick<RemoteKeyChord, "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "code">,
): boolean {
  return (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.code === "KeyV";
}

export function isMetaCode(code: string): boolean {
  return code === "MetaLeft" || code === "MetaRight";
}

/**
 * Local-only chords: macOS app/window controls (Cmd+Q/W/M/H/R/T/N/Tab/`/Space,
 * Cmd+Shift+3/4/5, Cmd+Ctrl+F fullscreen), Ctrl+R reload and Ctrl+Shift+I/J
 * devtools everywhere. Remote copy (Ctrl+Shift+C), F5/F11/F12 and the rest are
 * forwarded so remote terminals, BIOS menus and IDEs stay usable.
 */
export function isLocalOnlyShortcut(event: RemoteKeyChord, isMac: boolean): boolean {
  if (event.ctrlKey && event.shiftKey && ["KeyI", "KeyJ"].includes(event.code)) return true;
  if (event.ctrlKey && !event.shiftKey && event.code === "KeyR") return true;
  if (!isMac || !event.metaKey) return false;
  if (MAC_LOCAL_CODES.has(event.code)) return true;
  if (event.shiftKey && ["Digit3", "Digit4", "Digit5"].includes(event.code)) return true;
  return event.ctrlKey && event.code === "KeyF";
}

/**
 * macOS Command is translated to Control (both its keydown/up and its role as
 * a modifier), so Cmd+C copies on the remote instead of sending Windows+C.
 * Windows/Linux Meta stays the remote Windows/Super key.
 */
export function metaMapsToControl(
  isMac: boolean,
  event: Pick<RemoteKeyChord, "metaKey" | "code"> | string,
): boolean {
  if (!isMac) return false;
  return typeof event === "string" ? isMetaCode(event) : event.metaKey || isMetaCode(event.code);
}
