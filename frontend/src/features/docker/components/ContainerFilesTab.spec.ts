// @vitest-environment happy-dom
// 容器文件页（Task 10）：目录浏览骨架。rpc 全量 mock：列表/stat 的载荷形状
// 由 dockerFilePaths.spec 单测，这里只验证页面接线（进入目录、面包屑、刷新、
// 上一级、目录不可下载、只读禁用上传、缺 ls 与空态）。
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { rpc } from "../../../shared/lib/rpc";
import ContainerFilesTab from "./ContainerFilesTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";
import { setDockerTransferBridge } from "../lib/dockerFileTransfer";

vi.mock("../../../shared/lib/rpc", () => ({ rpc: vi.fn() }));

// 宿主 RPC 是泛型函数：经 unknown 转成 vi.fn 的 Mock 形状，spec 里才能自由 mockImplementation。
const rpcMock = rpc as unknown as ReturnType<typeof vi.fn>;

const t = (key: string) => key;
const container: DockerContainer = {
  id: "abcdef123456",
  name: "web-nginx",
  image: "nginx:1.27",
  state: "running",
  status: "Up 3 minutes",
  ports: "0.0.0.0:8080->80/tcp",
  createdAt: "2026-10-26T00:00:00Z",
};

const LIST_RESPONSES: Record<string, unknown> = {
  "/": {
    path: "/",
    parent: null,
    entries: [
      { name: "app.log", path: "/app.log", kind: "file" },
      { name: "etc", path: "/etc", kind: "dir" },
      { name: "var", path: "/var", kind: "dir" },
    ],
  },
  "/etc": {
    path: "/etc",
    parent: "/",
    entries: [
      { name: "hosts", path: "/etc/hosts", kind: "file" },
      { name: "nginx", path: "/etc/nginx", kind: "dir" },
    ],
  },
};

function mountTab(props: Record<string, unknown> = {}) {
  // Vue casts an absent Boolean prop to false: writable is the explicit default here.
  return mount(ContainerFilesTab, {
    props: { t, container, sessionId: "s", canWrite: true, ...props },
  });
}

beforeEach(() => {
  rpcMock.mockReset();
  rpcMock.mockImplementation(async (method: string, params?: unknown) => {
    if (method === "docker/files/list") {
      const path = (params as { path?: string } | undefined)?.path ?? "/";
      return LIST_RESPONSES[path] ?? { path, parent: null, entries: [] };
    }
    if (method === "docker/files/stat") {
      // 默认目标不存在：上传的覆盖预检直接放行（stat 失败 = 不存在）。
      throw new Error("stat: no such file or directory");
    }
    return {};
  });
});

afterEach(() => {
  setDockerTransferBridge(undefined);
});

describe("ContainerFilesTab transport wiring", () => {
  it("downloads a file through the registered transfer bridge", async () => {
    const downloadRemoteFile = vi.fn(async () => undefined);
    setDockerTransferBridge({ downloadRemoteFile, uploadSource: vi.fn(async () => true) });
    rpcMock.mockImplementation(async (method: string) => {
      if (method === "docker/files/list") return LIST_RESPONSES["/"];
      if (method === "docker/files/export") {
        return {
          stagingPath: "/tmp/dbx-sshx.1/app.log",
          size: 3,
          name: "app.log",
          location: "remote",
        };
      }
      if (method === "docker/files/cleanup") return { removed: true };
      return {};
    });
    const wrapper = mountTab();
    await flushPromises();

    await wrapper.get('[data-download="app.log"]').trigger("click");
    await flushPromises();

    expect(downloadRemoteFile).toHaveBeenCalledWith({
      remotePath: "/tmp/dbx-sshx.1/app.log",
      name: "app.log",
      size: 3,
    });
    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.objectContaining({
      stagingPath: "/tmp/dbx-sshx.1/app.log",
    }));
  });

  it("offers a hidden file input and uploads the picked file", async () => {
    const uploadSource = vi.fn(async () => true);
    setDockerTransferBridge({ downloadRemoteFile: vi.fn(async () => undefined), uploadSource });
    rpcMock.mockImplementation(async (method: string) => {
      if (method === "docker/files/list") return LIST_RESPONSES["/"];
      if (method === "docker/files/prepareUpload") {
        return { stagingPath: "/tmp/dbx-sshx.1/upload.bin", name: "upload.bin" };
      }
      if (method === "docker/files/import") return { path: "/upload.bin" };
      if (method === "docker/files/cleanup") return { removed: true };
      return {};
    });
    const wrapper = mountTab();
    await flushPromises();

    const input = wrapper.get("[data-files-upload-input]");
    const picked = new File(["abc"], "upload.bin");
    Object.defineProperty(input.element, "files", { value: [picked], configurable: true });
    await input.trigger("change");
    await flushPromises();

    expect(uploadSource).toHaveBeenCalledWith(
      "upload.bin",
      3,
      expect.any(Function),
      undefined,
      "/tmp/dbx-sshx.1",
      expect.objectContaining({ duplicatePreCheckedAbsent: true, displayName: "upload.bin" }),
    );
    expect(rpc).toHaveBeenCalledWith("docker/files/import", expect.objectContaining({
      stagingPath: "/tmp/dbx-sshx.1/upload.bin",
      destPath: "/upload.bin",
    }));
  });

  it("shows local upload progress and cancels it when the tab unmounts", async () => {
    setDockerTransferBridge({
      downloadRemoteFile: vi.fn(async () => undefined),
      uploadSource: vi.fn(async () => true),
    });
    let releaseChunk: ((value: unknown) => void) | undefined;
    const chunkGate = new Promise<unknown>((resolve) => {
      releaseChunk = resolve;
    });
    rpcMock.mockImplementation(async (method: string) => {
      if (method === "docker/files/list") return LIST_RESPONSES["/"];
      if (method === "docker/files/upload/begin") return { uploadId: "u1", maxBytes: 512 * 1024 * 1024 };
      if (method === "docker/files/upload/chunk") return chunkGate;
      if (method === "docker/files/upload/cancel") return { canceled: true };
      if (method === "docker/files/cleanup") return { removed: true };
      return {};
    });
    // 本机目标（无 sessionId）：分片上传走 docker/files/upload/*。
    const wrapper = mountTab({ sessionId: undefined });
    await flushPromises();

    const input = wrapper.get("[data-files-upload-input]");
    Object.defineProperty(input.element, "files", {
      value: [new File(["abc"], "report.log")],
      configurable: true,
    });
    await input.trigger("change");

    await vi.waitFor(() => {
      expect(rpc).toHaveBeenCalledWith("docker/files/upload/chunk", expect.anything());
    });
    expect(wrapper.find("[data-upload-progress]").exists()).toBe(true);
    expect(wrapper.get('[data-action="upload"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-action="upload"]').attributes("title")).toBe("docker.filesUploading");

    wrapper.unmount();
    releaseChunk?.({ received: 3 });
    await flushPromises();

    expect(rpc).toHaveBeenCalledWith("docker/files/upload/cancel", {
      target: "local",
      uploadId: "u1",
    });
  });

  it("starts one download when a file is double-clicked again while busy", async () => {
    const downloadRemoteFile = vi.fn(async () => undefined);
    setDockerTransferBridge({ downloadRemoteFile, uploadSource: vi.fn(async () => true) });
    let releaseExport: ((value: unknown) => void) | undefined;
    const exportGate = new Promise<unknown>((resolve) => {
      releaseExport = resolve;
    });
    rpcMock.mockImplementation(async (method: string) => {
      if (method === "docker/files/list") return LIST_RESPONSES["/"];
      if (method === "docker/files/export") return exportGate;
      if (method === "docker/files/cleanup") return { removed: true };
      return {};
    });
    const wrapper = mountTab();
    await flushPromises();

    await wrapper.get('[data-name="app.log"]').trigger("dblclick");
    await wrapper.get('[data-name="app.log"]').trigger("dblclick");
    releaseExport?.({
      stagingPath: "/tmp/dbx-sshx.1/app.log",
      size: 3,
      name: "app.log",
      location: "remote",
    });
    await flushPromises();

    expect(
      rpcMock.mock.calls.filter(([method]) => method === "docker/files/export"),
    ).toHaveLength(1);
    expect(downloadRemoteFile).toHaveBeenCalledTimes(1);
    expect(wrapper.get('[data-download="app.log"]').find(".spin").exists()).toBe(false);
  });
});


describe("ContainerFilesTab browsing", () => {
  it("renders directories before files and enters one on double click", async () => {
    const wrapper = mountTab();
    await flushPromises();

    const names = wrapper.findAll(".docker-file-row").map((row) => row.attributes("data-name"));
    expect(names).toEqual(["etc", "var", "app.log"]);
    // Row text stays just the name (directories carry no size meta).
    expect(wrapper.findAll(".docker-file-row").map((row) => row.text())).toContain("etc");
    expect(wrapper.get('[data-name="etc"]').attributes("data-kind")).toBe("dir");

    await wrapper.get('[data-name="etc"]').trigger("dblclick");
    await flushPromises();

    expect(rpc).toHaveBeenCalledWith("docker/files/list", expect.objectContaining({ path: "/etc" }));
    expect(wrapper.findAll(".docker-file-row").map((row) => row.attributes("data-name"))).toEqual([
      "nginx",
      "hosts",
    ]);
    expect(wrapper.find('[data-crumb="/etc"]').exists()).toBe(true);
  });

  it("navigates with the breadcrumb, parent and refresh buttons", async () => {
    const wrapper = mountTab();
    await flushPromises();
    await wrapper.get('[data-name="etc"]').trigger("dblclick");
    await flushPromises();

    await wrapper.get('[data-crumb="/"]').trigger("click");
    await flushPromises();
    expect(wrapper.find('[data-name="etc"]').exists()).toBe(true);

    await wrapper.get('[data-action="parent"]').trigger("click");
    await flushPromises();
    expect(rpc).toHaveBeenLastCalledWith("docker/files/list", expect.objectContaining({ path: "/" }));

    rpcMock.mockClear();
    await wrapper.get('[data-action="refresh"]').trigger("click");
    await flushPromises();
    expect(rpc).toHaveBeenCalledWith("docker/files/list", expect.objectContaining({ path: "/" }));
  });

  it("disables the parent button at the root and keeps directories out of downloads", async () => {
    const wrapper = mountTab();
    await flushPromises();

    expect(wrapper.get('[data-action="parent"]').attributes("disabled")).toBeDefined();
    const dirDownload = wrapper.get('[data-download="etc"]');
    expect(dirDownload.attributes("disabled")).toBeDefined();
    expect(dirDownload.attributes("title")).toBe("docker.filesDirNotDownloadable");
  });

  it("defers the root load until the tab is visible", async () => {
    const wrapper = mountTab({ visible: false });
    await flushPromises();
    expect(rpcMock.mock.calls.filter((call) => call[0] === "docker/files/list")).toHaveLength(0);

    await wrapper.setProps({ visible: true });
    await flushPromises();
    expect(rpcMock.mock.calls.filter((call) => call[0] === "docker/files/list")).toHaveLength(1);
    expect(wrapper.findAll(".docker-file-row").map((row) => row.attributes("data-name"))).toEqual([
      "etc",
      "var",
      "app.log",
    ]);
  });

  it("disables upload and import on read-only connections", async () => {
    const wrapper = mountTab({ canWrite: false });
    await flushPromises();

    const upload = wrapper.get('[data-action="upload"]');
    expect(upload.attributes("disabled")).toBeDefined();
    expect(upload.attributes("title")).toBe("docker.filesUnsupported");
  });

  it("shows a dedicated message when the container has no ls", async () => {
    rpcMock.mockRejectedValue(
      new Error('OCI runtime exec failed: exec: "ls": executable file not found in $PATH'),
    );
    const wrapper = mountTab();
    await flushPromises();

    expect(wrapper.get('[data-files-error]').text()).toBe("docker.filesToolMissing");
    const upload = wrapper.get('[data-action="upload"]');
    expect(upload.attributes("disabled")).toBeDefined();
    expect(upload.attributes("title")).toBe("docker.filesToolMissing");
    expect(wrapper.find(".docker-file-row").exists()).toBe(false);
  });

  it("labels directory loading separately from downloads", async () => {
    let releaseList: ((value: unknown) => void) | undefined;
    const listGate = new Promise<unknown>((resolve) => {
      releaseList = resolve;
    });
    rpcMock.mockImplementation(async (method: string) => {
      if (method === "docker/files/list") return listGate;
      return {};
    });
    const wrapper = mountTab();
    await flushPromises();

    expect(wrapper.get("[data-files-loading]").text()).toBe("loading");
    releaseList?.({ path: "/", parent: null, entries: [] });
    await flushPromises();
    expect(wrapper.find("[data-files-loading]").exists()).toBe(false);
  });

  it("shows an empty state instead of pretending a failed list succeeded", async () => {
    rpcMock.mockResolvedValue({ path: "/", parent: null, entries: [] });
    const wrapper = mountTab();
    await flushPromises();

    expect(wrapper.get('[data-files-empty]').text()).toBe("docker.filesEmpty");
    expect(wrapper.find('[data-files-error]').exists()).toBe(false);
  });

  it("reloads the root when the selected container changes", async () => {
    const wrapper = mountTab();
    await flushPromises();
    await wrapper.get('[data-name="etc"]').trigger("dblclick");
    await flushPromises();

    rpcMock.mockClear();
    await wrapper.setProps({ container: { ...container, id: "ffffffffffff", name: "other" } });
    await flushPromises();

    expect(rpc).toHaveBeenCalledWith("docker/files/list", expect.objectContaining({ path: "/" }));
  });
});
