/**
 * "Match window" desktop size for RDP and VNC.
 *
 * Deliberately uses CSS pixels instead of `devicePixelRatio`: feeding the
 * pane's physical pixels to Windows made a Retina pane negotiate a ~4K
 * desktop, so remote text and icons were rendered far too small even though
 * the bitmap itself was sharp. Sizing to CSS pixels keeps the remote UI at a
 * comfortable scale; the canvas is then upscaled by the compositor.
 */
export const DESKTOP_MATCH_MAX_WIDTH = 2560;
export const DESKTOP_MATCH_MAX_HEIGHT = 1600;
export const DESKTOP_MATCH_MIN_WIDTH = 800;
export const DESKTOP_MATCH_MIN_HEIGHT = 600;

const FALLBACK_SIZE = { width: 1280, height: 800 } as const;

/** Fit a pane's CSS size into the remote-desktop contract while preserving its aspect. */
export function matchWindowDesktopSize(cssWidth: number, cssHeight: number): { width: number; height: number } {
  if (!Number.isFinite(cssWidth) || !Number.isFinite(cssHeight) || cssWidth <= 0 || cssHeight <= 0) {
    return { ...FALLBACK_SIZE };
  }
  let width = cssWidth;
  let height = cssHeight;
  // Tiny panes still negotiate a usable desktop; the canvas scales it down.
  const grow = Math.max(1, DESKTOP_MATCH_MIN_WIDTH / width, DESKTOP_MATCH_MIN_HEIGHT / height);
  width *= grow;
  height *= grow;
  // Large panes stay inside the comfortable cap (and the 3840×2160 contract).
  const shrink = Math.min(1, DESKTOP_MATCH_MAX_WIDTH / width, DESKTOP_MATCH_MAX_HEIGHT / height);
  width *= shrink;
  height *= shrink;
  return {
    width: evenInRange(Math.round(width), 640, 3840),
    height: evenInRange(Math.round(height), 480, 2160),
  };
}

function evenInRange(value: number, min: number, max: number): number {
  const clamped = Math.max(min, Math.min(max, value));
  return clamped % 2 === 0 ? clamped : clamped - 1;
}
