// @vitest-environment happy-dom
import { ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useSftpCreateDirectory } from "./useSftpCreateDirectory";

const globalWindow = window as unknown as { dbxPlugin?: unknown };

afterEach(() => {
  delete globalWindow.dbxPlugin;
});

describe("useSftpCreateDirectory target directory", () => {
  it("creates inside the folder-row target and refreshes that directory", async () => {
    const invoke = vi.fn(async () => ({}));
    globalWindow.dbxPlugin = { invoke };
    const targetDir = ref<string | undefined>("/etc");
    const refreshAfterCreate = vi.fn(async () => undefined);
    const { operationDraft, createDirectory } = useSftpCreateDirectory({
      showError: vi.fn(),
      session: ref({ sessionId: "session-1" }),
      sudoMode: ref(false),
      currentPath: ref("/home/demo"),
      targetDir,
      joinRemote: (parent, name) => `${parent}/${name}`,
      loadDirectory: vi.fn(async () => undefined),
      refreshAfterCreate,
    });

    operationDraft.value = "newdir";
    await createDirectory();

    expect(invoke).toHaveBeenCalledWith("sftp/createDirectory", {
      sessionId: "session-1",
      path: "/etc/newdir",
    });
    expect(targetDir.value).toBeUndefined();
    expect(refreshAfterCreate).toHaveBeenCalledWith("/etc");
  });

  it("falls back to the current directory when no folder target was set", async () => {
    const invoke = vi.fn(async () => ({}));
    globalWindow.dbxPlugin = { invoke };
    const targetDir = ref<string | undefined>();
    const refreshAfterCreate = vi.fn(async () => undefined);
    const { operationDraft, createDirectory } = useSftpCreateDirectory({
      showError: vi.fn(),
      session: ref({ sessionId: "session-1" }),
      sudoMode: ref(false),
      currentPath: ref("/home/demo"),
      targetDir,
      joinRemote: (parent, name) => `${parent}/${name}`,
      loadDirectory: vi.fn(async () => undefined),
      refreshAfterCreate,
    });

    operationDraft.value = "newdir";
    await createDirectory();

    expect(invoke).toHaveBeenCalledWith("sftp/createDirectory", {
      sessionId: "session-1",
      path: "/home/demo/newdir",
    });
    expect(refreshAfterCreate).toHaveBeenCalledWith("/home/demo");
  });
});
