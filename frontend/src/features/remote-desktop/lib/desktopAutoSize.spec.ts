import { describe, expect, it } from "vitest";
import {
  matchWindowDesktopSize,
  DESKTOP_MATCH_MAX_HEIGHT,
  DESKTOP_MATCH_MAX_WIDTH,
  DESKTOP_MATCH_MIN_HEIGHT,
  DESKTOP_MATCH_MIN_WIDTH,
} from "./desktopAutoSize";

describe("remote desktop match-window size", () => {
  it("uses CSS pixels (no devicePixelRatio multiplier)", () => {
    // A Retina 1980x1100 CSS pane used to become a ~4K desktop.
    expect(matchWindowDesktopSize(1980, 1100)).toEqual({ width: 1980, height: 1100 });
  });

  it("caps large panes while preserving their aspect", () => {
    expect(matchWindowDesktopSize(5000, 3200)).toEqual({ width: 2500, height: 1600 });
    expect(matchWindowDesktopSize(6000, 2000)).toEqual({ width: 2560, height: 852 });
  });

  it("grows tiny panes to the minimum desktop", () => {
    expect(matchWindowDesktopSize(400, 300)).toEqual({ width: 800, height: 600 });
  });

  it("rounds to even dimensions inside the RDP contract", () => {
    const size = matchWindowDesktopSize(1441, 901);
    expect(size.width % 2).toBe(0);
    expect(size.height % 2).toBe(0);
    expect(size.width).toBeGreaterThanOrEqual(640);
    expect(size.height).toBeGreaterThanOrEqual(480);
    expect(size.width).toBeLessThanOrEqual(3840);
    expect(size.height).toBeLessThanOrEqual(2160);
  });

  it("falls back to a sane desktop for unmeasured panes", () => {
    expect(matchWindowDesktopSize(0, 0)).toEqual({ width: 1280, height: 800 });
    expect(matchWindowDesktopSize(Number.NaN, 900)).toEqual({ width: 1280, height: 800 });
  });

  it("exposes the documented bounds", () => {
    expect(DESKTOP_MATCH_MAX_WIDTH).toBe(2560);
    expect(DESKTOP_MATCH_MAX_HEIGHT).toBe(1600);
    expect(DESKTOP_MATCH_MIN_WIDTH).toBe(800);
    expect(DESKTOP_MATCH_MIN_HEIGHT).toBe(600);
  });
});
