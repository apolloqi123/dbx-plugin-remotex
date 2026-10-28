// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import PortForwardDialog from "./PortForwardDialog.vue";

const { loadProfiles } = vi.hoisted(() => ({ loadProfiles: vi.fn((): unknown[] => []) }));
vi.mock("../lib/tunnelProfiles", () => ({ loadTunnelProfiles: loadProfiles, saveTunnelProfile: vi.fn(), deleteTunnelProfile: vi.fn() }));

const running = {
  id: "live-1",
  sessionId: "",
  connectionId: "c-1",
  kind: "local",
  listenHost: "127.0.0.1",
  listenPort: 8080,
  boundPort: 8080,
  targetHost: "db.internal",
  targetPort: 5432,
  state: "active",
  connectionsTotal: 3,
  connectionsActive: 1,
  bytesUp: 1024,
  bytesDown: 2048,
};
const preset = {
  id: "preset-1",
  connectionId: "c-1",
  kind: "local",
  listenHost: "127.0.0.1",
  listenPort: "8080",
  targetHost: "db.internal",
  targetPort: "5432",
};
const idlePreset = {
  ...preset,
  id: "preset-2",
  listenPort: "3000",
  targetHost: "web.internal",
  targetPort: "80",
};

afterEach(() => {
  vi.unstubAllGlobals();
  loadProfiles.mockReturnValue([]);
});

function stubHost(handlers: Record<string, (params: unknown) => unknown> = {}) {
  let onEvent: ((event: DbxPluginEvent) => void) | undefined;
  const invoke = vi.fn(async (method: string, params?: unknown) => {
    const handler = handlers[method];
    return handler ? handler(params) : {};
  });
  vi.stubGlobal("dbxPlugin", {
    invoke,
    request: vi.fn(async () => undefined),
    reopenConnection: vi.fn(async () => undefined),
    onEvent: (callback: typeof onEvent) => {
      onEvent = callback;
      return () => undefined;
    },
  });
  return { invoke, emit: (event: DbxPluginEvent) => onEvent?.(event) };
}

function mountManager(props: Record<string, unknown> = {}) {
  return mount(PortForwardDialog, {
    props: { locale: "en", open: true, connectionId: "c-1", sessionId: null, independent: true, standalone: true, ...props },
  });
}

describe("port forward manager", () => {
  it("renders running tunnels and idle presets as one card list", async () => {
    loadProfiles.mockReturnValue([preset, idlePreset]);
    stubHost({ "ssh/forward/list": () => ({ forwards: [running] }), "ssh/forward/interfaces": () => ({ interfaces: [] }) });
    const wrapper = mountManager();
    await flushPromises();
    const cards = wrapper.findAll(".forward-card");
    expect(cards).toHaveLength(2);
    expect(cards[0].text()).toContain("127.0.0.1:8080 → db.internal:5432");
    expect(cards[0].text()).toContain("Preset");
    expect(cards[0].text()).toContain("Active");
    expect(cards[1].text()).toContain("127.0.0.1:3000 → web.internal:80");
    expect(cards[1].text()).toContain("Preset");
    expect(wrapper.find(".forward-summary").text()).toContain("1 running");
    expect(wrapper.find(".forward-summary").text()).toContain("1 connections");
    wrapper.unmount();
  });

  it("picks up a tunnel started from the connection menu", async () => {
    let visible = false;
    const host = stubHost({
      "ssh/forward/list": () => ({ forwards: visible ? [running] : [] }),
      "ssh/forward/interfaces": () => ({ interfaces: [] }),
    });
    const wrapper = mountManager();
    await flushPromises();
    expect(wrapper.findAll(".forward-card")).toHaveLength(0);
    visible = true;
    host.emit({ method: "ssh/forward/state", params: { id: "live-1", connectionId: "c-1", state: "active" } } as DbxPluginEvent);
    await flushPromises();
    expect(wrapper.findAll(".forward-card")).toHaveLength(1);
    expect(wrapper.text()).toContain("db.internal:5432");
    wrapper.unmount();
  });

  it("stops a running tunnel and removes it from the list", async () => {
    const host = stubHost({
      "ssh/forward/list": () => ({ forwards: [running] }),
      "ssh/forward/interfaces": () => ({ interfaces: [] }),
      "ssh/forward/stop": () => ({ success: true }),
    });
    const wrapper = mountManager();
    await flushPromises();
    await wrapper.find(".forward-card .forward-icon-button--danger").trigger("click");
    await flushPromises();
    expect(host.invoke).toHaveBeenCalledWith("ssh/forward/stop", { id: "live-1" });
    expect(wrapper.findAll(".forward-card")).toHaveLength(0);
    wrapper.unmount();
  });

  it("starts an idle preset from its card", async () => {
    loadProfiles.mockReturnValue([idlePreset]);
    const host = stubHost({
      "ssh/forward/list": () => ({ forwards: [] }),
      "ssh/forward/interfaces": () => ({ interfaces: [] }),
      "ssh/forward/start": () => ({ forward: { ...running, id: "live-3000", listenPort: 3000, boundPort: 3000, targetHost: "web.internal", targetPort: 80 } }),
    });
    const wrapper = mountManager();
    await flushPromises();
    await wrapper.find(".forward-card .forward-icon-button--start").trigger("click");
    await flushPromises();
    expect(host.invoke).toHaveBeenCalledWith("ssh/forward/start", {
      connectionId: "c-1",
      kind: "local",
      listenHost: "127.0.0.1",
      listenPort: 3000,
      targetHost: "web.internal",
      targetPort: 80,
    });
    wrapper.unmount();
  });

  it("opens an empty manager with the compact direction form and starts a SOCKS5 mapping", async () => {
    const host = stubHost({
      "ssh/forward/list": () => ({ forwards: [] }),
      "ssh/forward/interfaces": () => ({ interfaces: [] }),
      "ssh/forward/start": () => ({ forward: { ...running, id: "live-1080", kind: "dynamic", listenPort: 1080, boundPort: 1080, targetHost: "", targetPort: 0 } }),
    });
    const wrapper = mountManager();
    await flushPromises();
    expect(wrapper.find(".forward-empty").exists()).toBe(true);
    expect(wrapper.find(".forward-form").exists()).toBe(true);
    const directions = wrapper.findAll(".forward-direction");
    expect(directions).toHaveLength(3);
    await directions[2].trigger("click");
    expect(wrapper.findAll(".forward-field")).toHaveLength(1);
    expect(wrapper.find(".forward-preview").text()).toContain("SOCKS5");
    const inputs = wrapper.findAll(".forward-field input");
    await inputs[1].setValue("1080");
    await wrapper.find(".forward-form").trigger("submit");
    await flushPromises();
    expect(host.invoke).toHaveBeenCalledWith("ssh/forward/start", {
      connectionId: "c-1",
      kind: "dynamic",
      listenHost: "127.0.0.1",
      listenPort: 1080,
    });
    expect(wrapper.findAll(".forward-card")).toHaveLength(1);
    wrapper.unmount();
  });

  it("keeps the toolbar dialog as a modal and the manager as inline content", async () => {
    stubHost({ "ssh/forward/list": () => ({ forwards: [] }), "ssh/forward/interfaces": () => ({ interfaces: [] }) });
    const inline = mountManager();
    await flushPromises();
    expect(inline.find("section.forwards-modal--standalone").exists()).toBe(true);
    inline.unmount();

    const modal = mountManager({ standalone: false });
    await flushPromises();
    expect(document.querySelector('[data-slot="dialog-overlay"]')).not.toBeNull();
    expect(document.body.textContent).toContain("Port forwards");
    modal.unmount();
  });
});
