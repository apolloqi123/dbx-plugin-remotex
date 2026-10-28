// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick } from "vue";
import { usePortForwards } from "./usePortForwards";

const { loadProfiles, saveProfileMock, deleteProfileMock } = vi.hoisted(() => ({
  loadProfiles: vi.fn((): unknown[] => []),
  saveProfileMock: vi.fn(),
  deleteProfileMock: vi.fn(),
}));
vi.mock("../lib/tunnelProfiles", () => ({
  loadTunnelProfiles: loadProfiles,
  saveTunnelProfile: saveProfileMock,
  deleteTunnelProfile: deleteProfileMock,
}));

const liveRow = (overrides: Record<string, unknown> = {}) => ({
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
  bytesUp: 100,
  bytesDown: 200,
  ...overrides,
});

const preset = {
  id: "preset-1",
  connectionId: "c-1",
  kind: "local" as const,
  listenHost: "127.0.0.1",
  listenPort: "8080",
  targetHost: "db.internal",
  targetPort: "5432",
};
const idlePreset = { ...preset, id: "preset-2", listenPort: "3000", targetHost: "web.internal", targetPort: "80" };

afterEach(() => {
  vi.unstubAllGlobals();
  loadProfiles.mockReturnValue([]);
  saveProfileMock.mockReset();
  deleteProfileMock.mockReset();
});

function setup(handlers: Record<string, (params: unknown) => unknown> = {}, clipboard?: () => { bridge: { writeText: (text: string) => Promise<void>; readText: () => Promise<string> } }) {
  let eventHandler: ((event: DbxPluginEvent) => void) | undefined;
  const invoke = vi.fn(async (method: string, params?: unknown) => {
    const handler = handlers[method];
    return handler ? handler(params) : {};
  });
  vi.stubGlobal("dbxPlugin", {
    invoke,
    request: vi.fn(async () => undefined),
    reopenConnection: vi.fn(async () => undefined),
    onEvent: (callback: typeof eventHandler) => {
      eventHandler = callback;
      return () => undefined;
    },
  });
  const scope = effectScope();
  const controller = scope.run(() =>
    usePortForwards({
      connectionId: () => "c-1",
      independent: () => true,
      sessionId: () => null,
      clipboard,
      onError: () => undefined,
    }),
  )!;
  return { controller, invoke, scope, emit: (event: DbxPluginEvent) => eventHandler?.(event) };
}

describe("usePortForwards", () => {
  it("loads the list, presets and interfaces on open and merges cards", async () => {
    loadProfiles.mockReturnValue([preset, idlePreset]);
    const { controller } = setup({
      "ssh/forward/list": () => ({ forwards: [liveRow()] }),
      "ssh/forward/interfaces": () => ({ interfaces: [{ name: "lo0", addr: "127.0.0.1", isLoopback: true }] }),
    });
    await controller.open();
    expect(controller.forwards.value.map((row) => row.id)).toEqual(["live-1"]);
    expect(controller.profiles.value).toHaveLength(2);
    expect(controller.interfaces.value).toHaveLength(1);
    expect(controller.cards.value.map((card) => card.key)).toEqual(["forward:live-1", "profile:preset-2"]);
    expect(controller.summary.value).toMatchObject({ running: 1, connectionsActive: 1, bytesUp: 100 });
    expect(controller.inactiveProfiles.value.map((row) => row.id)).toEqual(["preset-2"]);
  });

  it("applies state events and drops stopped tunnels", async () => {
    const { controller, emit } = setup({ "ssh/forward/list": () => ({ forwards: [liveRow()] }) });
    await controller.open();
    emit({ method: "ssh/forward/state", params: { id: "live-1", connectionId: "c-1", state: "error", error: "refused" } } as DbxPluginEvent);
    expect(controller.forwards.value[0]).toMatchObject({ state: "error", error: "refused" });
    emit({ method: "ssh/forward/state", params: { id: "live-1", connectionId: "c-1", state: "stopped" } } as DbxPluginEvent);
    expect(controller.forwards.value).toEqual([]);
  });

  it("starts and stops a mapping through the independent connection id", async () => {
    const { controller, invoke } = setup({
      "ssh/forward/list": () => ({ forwards: [] }),
      "ssh/forward/start": () => ({ forward: liveRow() }),
      "ssh/forward/stop": () => ({ success: true }),
    });
    await controller.open();
    const started = await controller.start({ kind: "local", listenHost: "127.0.0.1", listenPort: "8080", targetHost: "db.internal", targetPort: "5432" });
    expect(started).toBe(true);
    expect(invoke).toHaveBeenCalledWith("ssh/forward/start", {
      connectionId: "c-1",
      kind: "local",
      listenHost: "127.0.0.1",
      listenPort: 8080,
      targetHost: "db.internal",
      targetPort: 5432,
    });
    expect(controller.forwards.value).toHaveLength(1);
    await controller.stop("live-1");
    expect(invoke).toHaveBeenCalledWith("ssh/forward/stop", { id: "live-1" });
    expect(controller.forwards.value).toEqual([]);
  });

  it("starts only idle presets and counts them", async () => {
    loadProfiles.mockReturnValue([preset, idlePreset]);
    const { controller } = setup({
      "ssh/forward/list": () => ({ forwards: [liveRow()] }),
      "ssh/forward/start": (params) => {
        const input = params as Record<string, unknown>;
        return { forward: liveRow({ id: `live-${input.listenPort}`, listenPort: input.listenPort, boundPort: input.listenPort, targetHost: input.targetHost, targetPort: input.targetPort }) };
      },
    });
    await controller.open();
    await controller.startSaved();
    expect(controller.startedCount.value).toBe(1);
    expect(controller.forwards.value.map((row) => row.id)).toEqual(["live-1", "live-3000"]);
    expect(controller.message.value).toBe("");
  });

  it("copies the bound address through the clipboard bridge", async () => {
    const writeText = vi.fn(async () => undefined);
    const { controller } = setup({}, () => ({ bridge: { writeText, readText: async () => "" } }));
    await expect(controller.copyAddress(liveRow())).resolves.toBe("127.0.0.1:8080");
    expect(writeText).toHaveBeenCalledWith("127.0.0.1:8080");
  });

  it("reports a failed list without wiping the current rows", async () => {
    const { controller } = setup({
      "ssh/forward/list": () => {
        throw new Error("sidecar offline");
      },
    });
    await controller.open();
    expect(controller.error.value).toBe("sidecar offline");
    expect(controller.forwards.value).toEqual([]);
    await nextTick();
  });
});
