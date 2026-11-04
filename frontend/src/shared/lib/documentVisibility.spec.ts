// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { documentHidden, onDocumentVisibilityChange } from "./documentVisibility";

function setVisibility(value: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { value, configurable: true });
}

describe("documentVisibility", () => {
  it("reports hidden only for the hidden state", () => {
    setVisibility("hidden");
    expect(documentHidden()).toBe(true);
    setVisibility("visible");
    expect(documentHidden()).toBe(false);
  });

  it("notifies listeners and unbinds cleanly", () => {
    const handler = vi.fn();
    const off = onDocumentVisibilityChange(handler);
    setVisibility("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(handler).toHaveBeenLastCalledWith(true);

    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(handler).toHaveBeenLastCalledWith(false);

    off();
    handler.mockClear();
    document.dispatchEvent(new Event("visibilitychange"));
    expect(handler).not.toHaveBeenCalled();
  });
});
