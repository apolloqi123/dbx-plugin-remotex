import { describe, expect, it } from "vitest";
import { cursorFromPointer } from "./remoteCursor";

describe("remote cursor mapping", () => {
  it("maps default and hidden cursors", () => {
    expect(cursorFromPointer({ type: "default" })).toEqual({ changed: true, cursor: "" });
    expect(cursorFromPointer({ type: "hidden" })).toEqual({ changed: true, cursor: "none" });
  });

  it("leaves the cursor untouched for position-only updates", () => {
    expect(cursorFromPointer({ type: "position", x: 10, y: 20 })).toEqual({ changed: false, cursor: "" });
  });

  it("falls back to the default cursor when bitmap decoding is unavailable", () => {
    // happy-dom has no 2D canvas: the helper must not throw and must reset.
    expect(cursorFromPointer({
      type: "bitmap",
      width: 1,
      height: 1,
      hotspotX: 0,
      hotspotY: 0,
      rgbaBase64: "AQID/w==",
    })).toEqual({ changed: true, cursor: "" });
  });

  it("rejects cursors above the 128px custom-cursor limit", () => {
    expect(cursorFromPointer({
      type: "bitmap",
      width: 129,
      height: 1,
      hotspotX: 0,
      hotspotY: 0,
      rgbaBase64: "AQID/w==",
    })).toEqual({ changed: true, cursor: "" });
  });
});
