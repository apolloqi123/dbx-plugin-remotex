// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import VncSurface from "./VncSurface.vue";
import RdpSurface from "./RdpSurface.vue";

const originalPlatform = navigator.platform;
const originalBridge = (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;

beforeEach(() => {
  Object.defineProperty(window.navigator, "platform", { value: "MacIntel", configurable: true });
  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = {
    clipboard: { readText: async () => "clip", writeText: async () => undefined },
    decodeBase64: () => new Uint8Array(),
  };
});

afterEach(() => {
  Object.defineProperty(window.navigator, "platform", { value: originalPlatform, configurable: true });
  (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = originalBridge;
});

describe("remote desktop surfaces", () => {
  it("VNC: macOS Cmd sends Control down and releases it on Cmd keyup", async () => {
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      const canvas = wrapper.get("canvas");
      await canvas.trigger("keydown", { code: "MetaLeft", key: "Meta", metaKey: true });
      const down = wrapper.emitted("input") ?? [];
      expect(down).toHaveLength(1);
      expect(down[0][0]).toMatchObject({ kind: "key", keysym: 0xffe3, pressed: true });

      await canvas.trigger("keyup", { code: "MetaLeft", key: "Meta", metaKey: false });
      const events = wrapper.emitted("input") ?? [];
      expect(events).toHaveLength(2);
      expect(events[1][0]).toMatchObject({ kind: "key", keysym: 0xffe3, pressed: false });
    } finally {
      wrapper.unmount();
    }
  });

  it("VNC: macOS Cmd+C copies on the remote as Control+C", async () => {
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      const canvas = wrapper.get("canvas");
      await canvas.trigger("keydown", { code: "MetaLeft", key: "Meta", metaKey: true });
      await canvas.trigger("keydown", { code: "KeyC", key: "c", metaKey: true });
      await canvas.trigger("keyup", { code: "KeyC", key: "c", metaKey: true });
      await canvas.trigger("keyup", { code: "MetaLeft", key: "Meta", metaKey: false });
      expect(wrapper.emitted("input")).toEqual([
        [{ kind: "key", keysym: 0xffe3, pressed: true }],
        [{ kind: "key", keysym: 0x63, pressed: true }],
        [{ kind: "key", keysym: 0x63, pressed: false }],
        [{ kind: "key", keysym: 0xffe3, pressed: false }],
      ]);
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: macOS Cmd+C copies on the remote as Control+C", async () => {
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit" } });
    try {
      const canvas = wrapper.get("canvas");
      await canvas.trigger("keydown", { code: "MetaLeft", key: "Meta", metaKey: true });
      await canvas.trigger("keydown", { code: "KeyC", key: "c", metaKey: true });
      await canvas.trigger("keyup", { code: "KeyC", key: "c", metaKey: true });
      await canvas.trigger("keyup", { code: "MetaLeft", key: "Meta", metaKey: false });
      const events = (wrapper.emitted("input") ?? []).map((call) => call[0]);
      expect(events).toMatchObject([
        { kind: "key-down", scanCode: 0x1d },
        { kind: "key-down", scanCode: 0x2e },
        { kind: "key-up", scanCode: 0x2e },
        { kind: "key-up", scanCode: 0x1d },
      ]);
      expect(events).toHaveLength(4);
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: macOS Cmd sends Control down and releases it on Cmd keyup", async () => {
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit" } });
    try {
      const canvas = wrapper.get("canvas");
      await canvas.trigger("keydown", { code: "MetaLeft", key: "Meta", metaKey: true });
      let events = wrapper.emitted("input") ?? [];
      expect(events).toHaveLength(1);
      expect(events[0][0]).toMatchObject({ kind: "key-down", scanCode: 0x1d });

      await canvas.trigger("keyup", { code: "MetaLeft", key: "Meta", metaKey: false });
      events = wrapper.emitted("input") ?? [];
      expect(events).toHaveLength(2);
      expect(events[1][0]).toMatchObject({ kind: "key-up", scanCode: 0x1d });
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: reports pane size changes so auto-fit can renegotiate resolution", async () => {
    const observer = { callback: null as ResizeObserverCallback | null };
    class FakeResizeObserver {
      constructor(next: ResizeObserverCallback) { observer.callback = next; }
      observe() {}
      disconnect() {}
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit" } });
    try {
      observer.callback?.([{ contentRect: { width: 800, height: 600 } } as ResizeObserverEntry], {} as ResizeObserver);
      expect(wrapper.emitted("container-resize")?.[0]).toEqual([{ width: 800, height: 600 }]);
    } finally {
      wrapper.unmount();
      vi.unstubAllGlobals();
    }
  });

  it("VNC: fit scales the canvas to fill the pane (up and down)", async () => {
    const observer = { callback: null as ResizeObserverCallback | null };
    class FakeResizeObserver {
      constructor(next: ResizeObserverCallback) { observer.callback = next; }
      observe() {}
      disconnect() {}
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      observer.callback?.([{ contentRect: { width: 400, height: 200 } } as ResizeObserverEntry], {} as ResizeObserver);
      const frame = new Uint8Array(44 + 2 * 2 * 4);
      const view = new DataView(frame.buffer);
      view.setBigUint64(0, 1n, true);
      view.setUint32(8, 2, true);
      view.setUint32(12, 2, true);
      view.setUint32(24, 2, true);
      view.setUint32(28, 2, true);
      view.setUint32(32, 8, true);
      view.setUint32(36, 2, true);
      view.setUint32(40, 16, true);
      (wrapper.vm as unknown as { acceptFrame(frame: Uint8Array): boolean }).acceptFrame(frame);
      await nextTick();
      const canvas = wrapper.get("canvas").element as HTMLCanvasElement;
      // 2×2 desktop in a 400×200 pane: contain scales to 200×200 (upscaled).
      expect(canvas.style.width).toBe("200px");
      expect(canvas.style.height).toBe("200px");
    } finally {
      wrapper.unmount();
      vi.unstubAllGlobals();
    }
  });

  it("VNC: reports pane size changes so auto-fit can renegotiate resolution", async () => {
    const observer = { callback: null as ResizeObserverCallback | null };
    class FakeResizeObserver {
      constructor(next: ResizeObserverCallback) { observer.callback = next; }
      observe() {}
      disconnect() {}
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      observer.callback?.([{ contentRect: { width: 1024, height: 768 } } as ResizeObserverEntry], {} as ResizeObserver);
      expect(wrapper.emitted("container-resize")?.[0]).toEqual([{ width: 1024, height: 768 }]);
    } finally {
      wrapper.unmount();
      vi.unstubAllGlobals();
    }
  });

  it("VNC: sendPasteShortcut emits one complete Control+V sequence", () => {
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      (wrapper.vm as unknown as { sendPasteShortcut(): void }).sendPasteShortcut();
      expect(wrapper.emitted("input")).toEqual([
        [{ kind: "key", keysym: 0xffe3, pressed: true }],
        [{ kind: "key", keysym: 0x76, pressed: true }],
        [{ kind: "key", keysym: 0x76, pressed: false }],
        [{ kind: "key", keysym: 0xffe3, pressed: false }],
      ]);
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: sendPasteShortcut emits one complete Control+V sequence", () => {
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit" } });
    try {
      (wrapper.vm as unknown as { sendPasteShortcut(): void }).sendPasteShortcut();
      const events = (wrapper.emitted("input") ?? []).map((call) => call[0]);
      expect(events).toMatchObject([
        { kind: "key-down", scanCode: 0x1d },
        { kind: "key-down", scanCode: 0x2f },
        { kind: "key-up", scanCode: 0x2f },
        { kind: "key-up", scanCode: 0x1d },
      ]);
      expect(events).toHaveLength(4);
    } finally {
      wrapper.unmount();
    }
  });

  it("VNC: Ctrl+V emits a paste request when a clipboard path exists", async () => {
    const wrapper = mount(VncSurface, { props: { scaleMode: "fit" } });
    try {
      await wrapper.get("canvas").trigger("keydown", { code: "KeyV", key: "v", ctrlKey: true });
      expect(wrapper.emitted("paste-request")).toHaveLength(1);
      expect(wrapper.emitted("input")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: Ctrl+V emits a paste request when a clipboard path exists", async () => {
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit" } });
    try {
      await wrapper.get("canvas").trigger("keydown", { code: "KeyV", key: "v", ctrlKey: true });
      expect(wrapper.emitted("paste-request")).toHaveLength(1);
      expect(wrapper.emitted("input")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it("RDP: Ctrl+V is forwarded to the remote when clipboard sharing is disabled", async () => {
    const wrapper = mount(RdpSurface, { props: { scaleMode: "fit", clipboardEnabled: false } });
    try {
      await wrapper.get("canvas").trigger("keydown", { code: "ControlLeft", key: "Control", ctrlKey: true });
      await wrapper.get("canvas").trigger("keydown", { code: "KeyV", key: "v", ctrlKey: true });
      expect(wrapper.emitted("paste-request")).toBeUndefined();
      expect((wrapper.emitted("input") ?? []).map((call) => call[0])).toMatchObject([
        { kind: "key-down", scanCode: 0x1d },
        { kind: "key-down", scanCode: 0x2f },
      ]);
    } finally {
      wrapper.unmount();
    }
  });
});
