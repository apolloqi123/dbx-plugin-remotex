// @vitest-environment happy-dom
// 容器终端页：每条命令一次独立 docker/exec（65s 客户端超时，后端 60s 封顶）、
// 命令回显 + 输出 + 退出码、↑/↓ 历史（上限 50）、清空、只读拦截、容器切换清空、
// 错误内联与忙碌期防重复提交、旧容器请求在新容器运行中返回时不抢忙碌态/焦点。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ContainerTerminalTab from "./ContainerTerminalTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";
import { DOCKER_ENGINE_STORE_KEY } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

const t = (key: string, values?: Record<string, string | number>) =>
  values ? `${key} ${Object.values(values).join(" ")}` : key;

type Call = { method: string; params: Record<string, unknown>; options?: { timeoutMs?: number } };
type MountedTab = ReturnType<typeof mount>;

const wrappers: MountedTab[] = [];

function container(overrides: Partial<DockerContainer> = {}): DockerContainer {
  return {
    id: "abc123def456",
    name: "web-nginx",
    image: "nginx:1.27",
    state: "running",
    status: "Up 3 minutes",
    ports: "8080->80/tcp",
    createdAt: "2026-10-26T00:00:00Z",
    ...overrides,
  };
}

function stubHost(handler: (params: Record<string, unknown>) => unknown): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string, params: Record<string, unknown>, options?: { timeoutMs?: number }) => {
      calls.push({ method, params, options });
      return handler(params);
    },
  });
  return calls;
}

function mountTab(
  options: { container?: DockerContainer; canWrite?: boolean; sessionId?: string | null; connectionId?: string } = {},
) {
  const props = {
    t,
    container: options.container ?? container(),
    canWrite: options.canWrite,
    connectionId: options.connectionId,
    // null = 显式不传 sessionId（走本机 daemon 目标）。
    ...(options.sessionId !== null ? { sessionId: options.sessionId ?? "sess-1" } : {}),
  };
  const wrapper = mount(ContainerTerminalTab, { attachTo: document.body, props });
  wrappers.push(wrapper);
  return wrapper;
}

async function submit(wrapper: MountedTab, command: string): Promise<void> {
  const input = wrapper.get("[data-exec-input]");
  await input.setValue(command);
  await input.trigger("keydown", { key: "Enter" });
  await flushPromises();
  await nextTick();
}

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

describe("ContainerTerminalTab", () => {
  it("runs one exec, echoes the command, and prints output with the exit code", async () => {
    const calls = stubHost(() => ({ success: true, output: "Linux web 6.1\n", exitCode: 0 }));
    const wrapper = mountTab();
    await submit(wrapper, "uname -a");

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe("docker/exec");
    expect(calls[0].params).toEqual({
      sessionId: "sess-1",
      containerId: "abc123def456",
      command: "uname -a",
    });
    expect(calls[0].options).toEqual({ timeoutMs: 65_000 });

    const output = wrapper.get("[data-exec-output]").text();
    expect(output).toContain("$ uname -a");
    expect(output).toContain("Linux web 6.1");
    expect(output).toContain("docker.execExitCode 0");
    expect((wrapper.get("[data-exec-input]").element as HTMLInputElement).value).toBe("");
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(false);
    expect(document.activeElement).toBe(wrapper.get("[data-exec-input]").element);
  });

  it("targets the local daemon without a session and forwards engine params", async () => {
    pluginStore.setItem(
      DOCKER_ENGINE_STORE_KEY,
      JSON.stringify({ "conn-1": { cli: "podman", socket: "", host: "tcp://127.0.0.1:2375" } }),
    );
    const calls = stubHost(() => ({ success: true, output: "", exitCode: 0 }));
    const wrapper = mountTab({ sessionId: null, connectionId: "conn-1" });
    await submit(wrapper, "id");

    expect(calls[0].params).toEqual({
      target: "local",
      containerId: "abc123def456",
      command: "id",
      cli: "podman",
      host: "tcp://127.0.0.1:2375",
    });
  });

  it("navigates command history with ArrowUp/ArrowDown and restores the draft", async () => {
    stubHost((params) => ({ success: true, output: `${String(params.command)}\n`, exitCode: 0 }));
    const wrapper = mountTab();
    await submit(wrapper, "first");
    await submit(wrapper, "second");

    const input = wrapper.get("[data-exec-input]");
    await input.trigger("keydown", { key: "ArrowUp" });
    expect((input.element as HTMLInputElement).value).toBe("second");
    await input.trigger("keydown", { key: "ArrowUp" });
    expect((input.element as HTMLInputElement).value).toBe("first");
    await input.trigger("keydown", { key: "ArrowDown" });
    expect((input.element as HTMLInputElement).value).toBe("second");
    await input.trigger("keydown", { key: "ArrowDown" });
    expect((input.element as HTMLInputElement).value).toBe("");
  });

  it("keeps at most 50 commands in history", async () => {
    stubHost(() => ({ success: true, output: "ok\n", exitCode: 0 }));
    const wrapper = mountTab();
    for (let index = 0; index < 55; index += 1) {
      await submit(wrapper, `cmd-${index}`);
    }

    const input = wrapper.get("[data-exec-input]");
    for (let step = 0; step < 50; step += 1) {
      await input.trigger("keydown", { key: "ArrowUp" });
    }
    // 保留最近 50 条：cmd-5 .. cmd-54；再按 ↑ 仍停在最旧一条。
    expect((input.element as HTMLInputElement).value).toBe("cmd-5");
    await input.trigger("keydown", { key: "ArrowUp" });
    expect((input.element as HTMLInputElement).value).toBe("cmd-5");
  });

  it("clears the output without clearing the command history", async () => {
    stubHost(() => ({ success: true, output: "hello\n", exitCode: 0 }));
    const wrapper = mountTab();
    await submit(wrapper, "echo hello");
    expect(wrapper.get("[data-exec-output]").text()).toContain("hello");

    await wrapper.get('[data-action="exec-clear"]').trigger("click");
    expect(wrapper.get("[data-exec-output]").text()).toBe("");
    expect(wrapper.find("[data-exec-empty]").exists()).toBe(true);

    const input = wrapper.get("[data-exec-input]");
    await input.trigger("keydown", { key: "ArrowUp" });
    expect((input.element as HTMLInputElement).value).toBe("echo hello");
  });

  it("ignores empty commands and blocks execution when the connection is read-only", async () => {
    const calls = stubHost(() => ({ success: true, output: "", exitCode: 0 }));
    const wrapper = mountTab();
    await submit(wrapper, "   ");
    expect(calls).toEqual([]);

    const readOnly = mountTab({ canWrite: false });
    expect(readOnly.get("[data-exec-readonly]").text()).toBe("docker.execReadOnly");
    expect((readOnly.get("[data-exec-input]").element as HTMLInputElement).disabled).toBe(true);
    await submit(readOnly, "rm -rf /tmp/x");
    expect(calls).toEqual([]);
  });

  it("clears output, input and history when the container changes", async () => {
    stubHost(() => ({ success: true, output: "old container\n", exitCode: 0 }));
    const wrapper = mountTab();
    await submit(wrapper, "hostname");
    expect(wrapper.get("[data-exec-output]").text()).toContain("old container");

    await wrapper.setProps({ container: container({ id: "def456abc789", name: "db" }) });
    await nextTick();
    expect(wrapper.get("[data-exec-output]").text()).toBe("");
    expect((wrapper.get("[data-exec-input]").element as HTMLInputElement).value).toBe("");

    const input = wrapper.get("[data-exec-input]");
    await input.trigger("keydown", { key: "ArrowUp" });
    expect((input.element as HTMLInputElement).value).toBe("");
  });

  it("shows RPC errors inline and stays usable afterwards", async () => {
    stubHost(() => {
      throw new Error("permission denied while trying to connect to the Docker daemon socket");
    });
    const wrapper = mountTab();
    await submit(wrapper, "ps aux");

    expect(wrapper.get("[data-exec-output]").text()).toContain("$ ps aux");
    expect(wrapper.get("[data-exec-error]").text()).toContain("docker.execFailed");
    expect(wrapper.get("[data-exec-error]").text()).toContain("permission denied");
    expect((wrapper.get("[data-exec-input]").element as HTMLInputElement).disabled).toBe(false);
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(false);
  });

  it("prevents a second submission while the first command is still running", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const calls: Array<Record<string, unknown>> = [];
    vi.stubGlobal("dbxPlugin", {
      invoke: async (_method: string, params: Record<string, unknown>) => {
        calls.push(params);
        await gate;
        return { success: true, output: "slow done\n", exitCode: 0 };
      },
    });
    const wrapper = mountTab();
    const input = wrapper.get("[data-exec-input]");
    await input.setValue("sleep 5");
    await input.trigger("keydown", { key: "Enter" });
    await nextTick();
    expect(calls).toHaveLength(1);
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(true);

    await input.setValue("second");
    await input.trigger("keydown", { key: "Enter" });
    await nextTick();
    expect(calls).toHaveLength(1);

    release();
    await flushPromises();
    expect(calls).toHaveLength(1);
    expect(wrapper.get("[data-exec-output]").text()).toContain("slow done");
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(false);
  });

  it("does not clear the new container's busy state when the old command settles", async () => {
    const pending: Array<{ resolve: (value: unknown) => void }> = [];
    vi.stubGlobal("dbxPlugin", {
      invoke: (_method: string, params: Record<string, unknown>) =>
        new Promise((resolve) => {
          pending.push({ resolve });
        }),
    });
    const wrapper = mountTab();
    await submit(wrapper, "old-command");
    expect(pending).toHaveLength(1);
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(true);

    // 切容器时 watcher 清 busy，允许在新容器里继续执行。
    await wrapper.setProps({ container: container({ id: "def456abc789", name: "db" }) });
    await nextTick();
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(false);

    await submit(wrapper, "new-command");
    expect(pending).toHaveLength(2);
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(true);

    // 旧容器的响应在新命令仍在运行时返回：不得清 busy、不得写入新容器输出。
    pending[0].resolve({ success: true, output: "old output\n", exitCode: 0 });
    await flushPromises();
    await nextTick();
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(true);
    const output = wrapper.get("[data-exec-output]").text();
    expect(output).not.toContain("old output");
    expect(output).toContain("$ new-command");

    // 新命令收尾后才允许清 busy 并把焦点交回输入框。
    pending[1].resolve({ success: true, output: "new output\n", exitCode: 0 });
    await flushPromises();
    await nextTick();
    expect(wrapper.find("[data-exec-busy]").exists()).toBe(false);
    expect(wrapper.get("[data-exec-output]").text()).toContain("new output");
    expect(document.activeElement).toBe(wrapper.get("[data-exec-input]").element);
  });
});
