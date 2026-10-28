// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import PortForwardMenu from "./PortForwardMenu.vue";
import { usePortForwards } from "../composables/usePortForwards";
import { workbenchMessage } from "../../../shared/lib/i18n";

const { loadProfiles } = vi.hoisted(() => ({ loadProfiles: vi.fn((): unknown[] => []) }));
vi.mock("../lib/tunnelProfiles", () => ({
  loadTunnelProfiles: loadProfiles,
  saveTunnelProfile: vi.fn(),
  deleteTunnelProfile: vi.fn(),
}));

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
  connectionsTotal: 2,
  connectionsActive: 1,
  bytesUp: 0,
  bytesDown: 0,
};
const idlePreset = {
  id: "preset-2",
  connectionId: "c-1",
  kind: "remote",
  listenHost: "127.0.0.1",
  listenPort: "9000",
  targetHost: "localhost",
  targetPort: "3000",
};

afterEach(() => {
  vi.unstubAllGlobals();
  loadProfiles.mockReturnValue([]);
});

async function mountMenu() {
  const invoke = vi.fn(async (method: string) => {
    if (method === "ssh/forward/list") return { forwards: [running] };
    if (method === "ssh/forward/interfaces") return { interfaces: [] };
    if (method === "ssh/forward/start") return { forward: { ...running, id: "live-9000", kind: "remote", listenPort: 9000, boundPort: 9000, targetHost: "localhost", targetPort: 3000 } };
    if (method === "ssh/forward/stop") return { success: true };
    return {};
  });
  vi.stubGlobal("dbxPlugin", {
    invoke,
    request: vi.fn(async () => undefined),
    reopenConnection: vi.fn(async () => undefined),
    onEvent: () => () => undefined,
  });
  const scope = effectScope();
  const controller = scope.run(() =>
    usePortForwards({ connectionId: () => "c-1", independent: () => true, sessionId: () => null }),
  )!;
  const wrapper = mount(PortForwardMenu, {
    props: { controller, t: (key: string, values?: Record<string, string | number>) => workbenchMessage("en", key, values ?? {}) },
  });
  await flushPromises();
  return { wrapper, invoke, scope };
}

describe("PortForwardMenu", () => {
  it("lists running tunnels and saved presets with a running summary", async () => {
    loadProfiles.mockReturnValue([idlePreset]);
    const { wrapper, scope } = await mountMenu();
    expect(wrapper.text()).toContain("1 running");
    expect(wrapper.text()).toContain("127.0.0.1:8080 → db.internal:5432");
    expect(wrapper.text()).toContain("127.0.0.1:9000 ← localhost:3000");
    expect(wrapper.findAll(".forward-menu-row")).toHaveLength(2);
    wrapper.unmount();
    scope.stop();
  });

  it("stops a running tunnel from the quick menu", async () => {
    const { wrapper, invoke, scope } = await mountMenu();
    await wrapper.find(".forward-menu-action:not(.forward-menu-action--start)").trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("ssh/forward/stop", { id: "live-1" });
    wrapper.unmount();
    scope.stop();
  });

  it("starts an idle preset and opens the manager from the footer", async () => {
    loadProfiles.mockReturnValue([idlePreset]);
    const { wrapper, invoke, scope } = await mountMenu();
    await wrapper.find(".forward-menu-action--start").trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("ssh/forward/start", {
      connectionId: "c-1",
      kind: "remote",
      listenHost: "127.0.0.1",
      listenPort: 9000,
      targetHost: "localhost",
      targetPort: 3000,
    });
    const footerButtons = wrapper.findAll(".forward-menu-manage");
    await footerButtons[0].trigger("click");
    await footerButtons[1].trigger("click");
    expect(wrapper.emitted("add")).toHaveLength(1);
    expect(wrapper.emitted("manage")).toHaveLength(1);
    wrapper.unmount();
    scope.stop();
  });
});
