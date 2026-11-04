// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
// @ts-expect-error — Node built-in types are not included in the frontend tsconfig.
import { spawnSync } from "node:child_process";
// @ts-expect-error — Same Node-only test dependency as above.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
// @ts-expect-error — Same Node-only test dependency as above.
import { platform, tmpdir } from "node:os";
// @ts-expect-error — Same Node-only test dependency as above.
import { join } from "node:path";
import ContainerListPane from "./ContainerListPane.vue";
import { DOCKER_ENGINE_STORE_KEY, saveDockerEngineSettings } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

type Row = { id: string; name: string; state: string; image?: string; status?: string; ports?: string };

const directories: string[] = [];
const shells = ["bash", "zsh", "fish", "ash", "sh"];

afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

/** Only the external host bridge is replaced; the real list/action/menu path runs. */
function stubHost(rows: Row[]): void {
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string) => {
      if (method === "ssh/sessions/list") {
        return { sessions: [{ sessionId: "test-session", connectionId: "test-connection", connected: true }] };
      }
      if (method === "docker/list") return { available: true, containers: rows };
      throw new Error(`Unexpected host call: ${method}`);
    },
  });
}

function mountList() {
  return mount(ContainerListPane, {
    attachTo: document.body,
    props: { t: (key: string) => key, sessionId: "test-session", connectionId: "test-connection" },
  });
}

describe("container list selection", () => {
  it("selects the first running container after the first load", async () => {
    stubHost([
      { id: "db111", name: "db", state: "exited", image: "postgres:16", status: "Exited (0) 2 minutes ago" },
      { id: "web222", name: "web-nginx", state: "running", image: "nginx:1.27", ports: "0.0.0.0:8080->80/tcp" },
    ]);
    const wrapper = mountList();
    try {
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ name: "web-nginx", state: "running" });
    } finally {
      wrapper.unmount();
    }
  });

  it("falls back to the first container when the selection disappears", async () => {
    const rows: Row[] = [
      { id: "db111", name: "db", state: "exited", image: "postgres:16", status: "Exited (0) 3 minutes ago" },
      { id: "web222", name: "web-nginx", state: "running", image: "nginx:1.27" },
    ];
    stubHost(rows);
    const wrapper = mountList();
    try {
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ name: "web-nginx" });
      rows.splice(1, 1);
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ name: "db" });
    } finally {
      wrapper.unmount();
    }
  });

  it("emits null when the list empties", async () => {
    const rows: Row[] = [
      { id: "db111", name: "db", state: "exited", image: "postgres:16", status: "Exited (0) 4 minutes ago" },
      { id: "web222", name: "web-nginx", state: "running", image: "nginx:1.27" },
    ];
    stubHost(rows);
    const wrapper = mountList();
    try {
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ name: "web-nginx" });
      rows.splice(0);
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it("re-emits the selected row when polling changes its identity fields", async () => {
    const rows: Row[] = [
      { id: "web222", name: "web-nginx", state: "running", status: "Up 1 minute", image: "nginx:1.27", ports: "0.0.0.0:8080->80/tcp" },
    ];
    stubHost(rows);
    const wrapper = mountList();
    try {
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ state: "running" });
      rows[0] = { ...rows[0], state: "exited", status: "Exited (0) 1 second ago" };
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({
        name: "web-nginx",
        state: "exited",
        status: "Exited (0) 1 second ago",
      });
    } finally {
      wrapper.unmount();
    }
  });

  it("marks row clicks as explicit selections", async () => {
    stubHost([
      { id: "db111", name: "db", state: "exited", status: "Exited (0) 1 minute ago" },
      { id: "web222", name: "web-nginx", state: "running" },
    ]);
    const wrapper = mountList();
    try {
      await flushPromises();
      // 首载的自动选中只带容器，没有显式标记。
      expect(wrapper.emitted("select")?.[0]).toHaveLength(1);
      await wrapper.get('[data-container-id="db111"]').trigger("click");
      expect(wrapper.emitted("select")?.at(-1)).toEqual([expect.objectContaining({ name: "db" }), true]);
    } finally {
      wrapper.unmount();
    }
  });

  it("emits logs from the row menu", async () => {
    stubHost([{ id: "web222", name: "web-nginx", state: "running" }]);
    const wrapper = mountList();
    try {
      await flushPromises();
      await wrapper.get(".docker-row-trigger").trigger("click");
      await flushPromises();
      const item = Array.from(document.querySelectorAll<HTMLButtonElement>(".docker-row-menu-item"))
        .find((button) => button.textContent?.includes("docker.actionLogs"));
      expect(item, "row menu must offer logs").toBeDefined();
      item!.click();
      expect(wrapper.emitted("logs")?.[0]?.[0]).toMatchObject({ id: "web222" });
    } finally {
      wrapper.unmount();
    }
  });

  it("emits files from the row menu", async () => {
    stubHost([{ id: "web222", name: "web-nginx", state: "running" }]);
    const wrapper = mountList();
    try {
      await flushPromises();
      await wrapper.get(".docker-row-trigger").trigger("click");
      await flushPromises();
      const item = Array.from(document.querySelectorAll<HTMLButtonElement>(".docker-row-menu-item"))
        .find((button) => button.textContent?.includes("docker.filesTab"));
      expect(item, "row menu must offer files").toBeDefined();
      item!.click();
      expect(wrapper.emitted("files")?.[0]?.[0]).toMatchObject({ id: "web222" });
    } finally {
      wrapper.unmount();
    }
  });
});

describe("container search", () => {
  const rows = (): Row[] => [
    { id: "d4a7c9f1", name: "web-nginx", state: "running", image: "nginx:1.27", ports: "0.0.0.0:8080->80/tcp" },
    { id: "9f8e7d6c", name: "cache-redis", state: "running", image: "redis:7-alpine", ports: "127.0.0.1:6379->6379/tcp" },
    { id: "1a2b3c4d", name: "db-postgres", state: "running", image: "postgres:16" },
  ];

  it("filters rows by substring, subsequence and AND terms, then restores on clear", async () => {
    stubHost(rows());
    const wrapper = mountList();
    try {
      await flushPromises();
      const input = wrapper.get(".docker-search-input");
      expect(input.attributes("placeholder")).toBe("docker.searchPlaceholder");
      expect(wrapper.findAll(".container-row")).toHaveLength(3);

      await input.setValue("redis");
      expect(wrapper.findAll(".container-row")).toHaveLength(1);
      expect(wrapper.text()).toContain("cache-redis");

      // 子序列模糊：wng → web-nginx。
      await input.setValue("wng");
      expect(wrapper.findAll(".container-row")).toHaveLength(1);
      expect(wrapper.text()).toContain("web-nginx");

      // 多词 AND：名字 + 端口。
      await input.setValue("web 8080");
      expect(wrapper.findAll(".container-row")).toHaveLength(1);

      await input.setValue("no-such-container");
      expect(wrapper.findAll(".container-row")).toHaveLength(0);
      expect(wrapper.text()).toContain("docker.searchEmpty");

      await input.trigger("keydown", { key: "Escape" });
      expect((input.element as HTMLInputElement).value).toBe("");
      expect(wrapper.findAll(".container-row")).toHaveLength(3);
      expect(wrapper.find(".docker-search-clear").exists()).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it("shows a clear button that resets the query", async () => {
    stubHost(rows());
    const wrapper = mountList();
    try {
      await flushPromises();
      const input = wrapper.get(".docker-search-input");
      await input.setValue("redis");
      expect(wrapper.find(".docker-search-clear").exists()).toBe(true);
      await wrapper.get(".docker-search-clear").trigger("click");
      expect((input.element as HTMLInputElement).value).toBe("");
      expect(wrapper.findAll(".container-row")).toHaveLength(3);
    } finally {
      wrapper.unmount();
    }
  });

  it("combines with the state filter", async () => {
    stubHost([
      { id: "run111", name: "web-nginx", state: "running" },
      { id: "stop22", name: "web-legacy", state: "exited", status: "Exited (0) 1 minute ago" },
    ]);
    const wrapper = mountList();
    try {
      await flushPromises();
      await wrapper.get(".docker-search-input").setValue("web");
      expect(wrapper.findAll(".container-row")).toHaveLength(2);
      await wrapper.get(".container-filter-select").setValue("stopped");
      expect(wrapper.findAll(".container-row")).toHaveLength(1);
      expect(wrapper.text()).toContain("web-legacy");
    } finally {
      wrapper.unmount();
    }
  });
});

async function terminalCommand(cli = ""): Promise<string> {
  saveDockerEngineSettings("test-connection", { cli, socket: "", host: "" });
  stubHost([{ id: "abc123", name: "test", state: "running" }]);
  const wrapper = mountList();
  let command = "";
  const capture = (event: Event) => { command = (event as CustomEvent<{ command: string }>).detail.command; };
  window.addEventListener("dbx:docker-open-in-terminal", capture);
  try {
    await flushPromises();
    await wrapper.get(".docker-row-trigger").trigger("click");
    await flushPromises();
    const button = Array.from(document.querySelectorAll<HTMLButtonElement>(".docker-row-menu-item"))
      .find((item) => item.textContent?.includes("docker.actionTerminal"));
    expect(button, "SSH container menu must offer a terminal action").toBeDefined();
    button!.click();
    expect(command, "terminal action must emit a command").not.toBe("");
    return command;
  } finally {
    window.removeEventListener("dbx:docker-open-in-terminal", capture);
    wrapper.unmount();
  }
}

function runCommand(command: string, availableShells: string[], exitCode: number, cli = "docker") {
  const directory = mkdtempSync(join(tmpdir(), "dbx-docker-shell-"));
  directories.push(directory);
  const executable = (name: string, body: string) =>
    writeFileSync(join(directory, name), `#!/bin/sh\n${body}\n`, { mode: 0o755 });
  // Emulate the Docker boundary without a daemon/SSH connection. Run the exact
  // container script under a real POSIX shell with a controlled container PATH;
  // do not source the test machine's login profile via sh -l.
  executable(cli, `
if [ "$#" -ne 6 ] || [ "$1" != exec ] || [ "$2" != -it ] || [ "$3" != abc123 ] || [ "$4" != sh ] || [ "$5" != -lc ]; then
  exit 98
fi
exec /bin/sh -c "$6"`);
  for (const shell of availableShells) {
    executable(shell, `printf '%s\\n' '${shell}'\nexit ${exitCode}`);
  }
  return spawnSync("/bin/sh", ["-c", command], { env: { PATH: directory }, encoding: "utf8", timeout: 5_000 });
}

// Windows CI has no guaranteed POSIX /bin/sh; the real shell regression runs on
// macOS/Linux. The normal frontend typecheck still checks this file everywhere.
describe.skipIf(platform() === "win32")("Docker terminal shell selection (#182)", () => {
  it.each([0, 1, 127])("ends the session when Bash exits with status %i", async (exitCode) => {
    const result = runCommand(await terminalCommand(), shells, exitCode);
    expect(result.error).toBeUndefined();
    expect(result.stdout).toBe("bash\n");
    expect(result.stderr).toBe("");
    expect(result.status).toBe(exitCode);
  });

  it.each(shells.slice(1))("selects %s when earlier shells are absent without missing-command noise", async (shell) => {
    const result = runCommand(await terminalCommand(), shells.slice(shells.indexOf(shell)), 127);
    expect(result.error).toBeUndefined();
    expect(result.stdout).toBe(`${shell}\n`);
    expect(result.stderr).toBe("");
    expect(result.status).toBe(127);
  });

  it("uses the configured Podman CLI and preserves a nonzero shell exit", async () => {
    const result = runCommand(await terminalCommand("podman"), shells, 1, "podman");
    expect(result.error).toBeUndefined();
    expect(result.stdout).toBe("bash\n");
    expect(result.stderr).toBe("");
    expect(result.status).toBe(1);
  });
});
