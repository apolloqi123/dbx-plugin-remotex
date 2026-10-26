// 容器文件传输编排（Task 11 SSH / Task 12 本机）：只测 RPC 次序与清理，
// 具体的 SFTP 下载/上传管线由注入的 downloadRemoteFile/uploadSource 代表。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { rpc } from "../../../shared/lib/rpc";
import { useContainerFileTransfer } from "./useContainerFileTransfer";

vi.mock("../../../shared/lib/rpc", () => ({ rpc: vi.fn() }));

// 宿主 RPC 是泛型函数：经 unknown 转成 vi.fn 的 Mock 形状，spec 里才能自由 mockImplementation。
const rpcMock = rpc as unknown as ReturnType<typeof vi.fn>;

const MAX_BYTES = 512 * 1024 * 1024;

const remoteExport = () => ({
  stagingPath: "/tmp/dbx-sshx.1/report.log",
  size: 3,
  name: "report.log",
  location: "remote",
});

const localExport = () => ({
  stagingPath: "/tmp/dbx-sshx.1/report.log",
  size: 3,
  name: "report.log",
  location: "local",
});

const file = { name: "report.log", path: "/var/report.log", kind: "file" as const, size: 3 };

/** 按方法分派默认响应：个别用例覆盖单方法，避免依赖调用顺序。 */
function mockRpc(handlers: Record<string, (params: Record<string, unknown>) => unknown> = {}): void {
  rpcMock.mockImplementation(async (method: string, params?: unknown) => {
    const handler = handlers[method];
    if (handler) return handler(params as Record<string, unknown>);
    if (method === "docker/files/stat") throw new Error("stat: no such file or directory");
    if (method === "docker/files/cleanup") return { removed: true };
    if (method === "docker/files/import") return { path: (params as { destPath?: string })?.destPath };
    return {};
  });
}

function oversizedFile(name = "big.bin"): File {
  const big = new File(["x"], name);
  Object.defineProperty(big, "size", { value: MAX_BYTES + 1 });
  return big;
}

function setup(options: {
  sessionId?: string;
  local?: boolean;
  canWrite?: boolean;
  currentPath?: string;
  confirmOk?: boolean;
  downloadRejects?: Error;
} = {}) {
  const downloadRemoteFile = vi.fn(async () => undefined);
  const uploadSource = vi.fn(async () => true);
  const confirmDialog = vi.fn(async () => options.confirmOk !== false);
  const showError = vi.fn();
  const showNotice = vi.fn();
  if (options.downloadRejects) downloadRemoteFile.mockRejectedValueOnce(options.downloadRejects);
  const transfer = useContainerFileTransfer({
    t: (key) => key,
    sessionId: options.local ? undefined : options.sessionId ?? "s",
    containerId: "cid",
    currentPath: options.currentPath ?? "/srv/app",
    canWrite: options.canWrite ?? true,
    showError,
    showNotice,
    downloadRemoteFile,
    uploadSource,
    confirmDialog,
  });
  return { transfer, downloadRemoteFile, uploadSource, confirmDialog, showError, showNotice };
}

beforeEach(() => {
  rpcMock.mockReset();
  mockRpc();
});

describe("SSH container downloads", () => {
  it("exports then downloads then cleans up", async () => {
    mockRpc({ "docker/files/export": () => remoteExport() });
    const { transfer, downloadRemoteFile } = setup();

    await transfer.download(file);

    expect(rpc).toHaveBeenNthCalledWith(1, "docker/files/export", {
      sessionId: "s",
      containerId: "cid",
      path: "/var/report.log",
    });
    expect(downloadRemoteFile).toHaveBeenCalledWith({
      remotePath: "/tmp/dbx-sshx.1/report.log",
      name: "report.log",
      size: 3,
    });
    expect(rpc).toHaveBeenLastCalledWith("docker/files/cleanup", {
      sessionId: "s",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    });
  });

  it("cleans up when the download fails", async () => {
    mockRpc({ "docker/files/export": () => remoteExport() });
    const { transfer, showError } = setup({ downloadRejects: new Error("boom") });

    await expect(transfer.download(file)).rejects.toThrow("boom");

    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.objectContaining({
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    }));
    expect(showError).toHaveBeenCalled();
  });

  it("keeps the transfer result when cleanup fails", async () => {
    mockRpc({
      "docker/files/export": () => remoteExport(),
      "docker/files/cleanup": () => {
        throw new Error("rm failed");
      },
    });
    const { transfer, showNotice, showError } = setup();

    await expect(transfer.download(file)).resolves.toBeUndefined();

    expect(showNotice).toHaveBeenCalledWith("docker.filesCleanupFailed");
    expect(showError).not.toHaveBeenCalled();
  });

  it("refuses directories before exporting", async () => {
    const { transfer } = setup();

    await expect(transfer.download({ name: "etc", path: "/etc", kind: "dir" })).rejects.toThrow(
      "docker.filesDirNotDownloadable",
    );

    expect(rpc).not.toHaveBeenCalledWith("docker/files/export", expect.anything());
  });

  it("rejects files above 512 MiB before exporting", async () => {
    const { transfer, showError } = setup();

    await expect(
      transfer.download({ name: "big.bin", path: "/big.bin", kind: "file", size: MAX_BYTES + 1 }),
    ).rejects.toThrow("docker.filesTooLarge");

    expect(rpc).not.toHaveBeenCalledWith("docker/files/export", expect.anything());
    expect(showError).toHaveBeenCalled();
  });
});

describe("SSH container uploads", () => {
  it("stages over SFTP, imports into the container, then cleans up", async () => {
    mockRpc({
      "docker/files/prepareUpload": () => ({
        stagingPath: "/tmp/dbx-sshx.1/report.log",
        name: "report.log",
      }),
      "docker/files/import": () => ({ path: "/srv/app/report.log" }),
    });
    const { transfer, uploadSource } = setup();
    const picked = new File(["abcd"], "report.log", { type: "text/plain" });

    await transfer.upload(picked);

    expect(uploadSource).toHaveBeenCalledWith(
      "report.log",
      4,
      expect.any(Function),
      undefined,
      "/tmp/dbx-sshx.1",
      { duplicatePreCheckedAbsent: true, displayName: "report.log" },
    );
    const readChunk = (uploadSource.mock.calls[0] as unknown as unknown[])[2] as (
      offset: number,
      length: number,
    ) => Promise<Uint8Array>;
    const bytes = await readChunk(0, 4);
    expect(new TextDecoder().decode(bytes)).toBe("abcd");
    expect(rpc).toHaveBeenCalledWith("docker/files/import", {
      sessionId: "s",
      containerId: "cid",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
      destPath: "/srv/app/report.log",
    });
    expect(rpc).toHaveBeenLastCalledWith("docker/files/cleanup", {
      sessionId: "s",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    });
  });

  it("cleans up the staging file when the import fails", async () => {
    mockRpc({
      "docker/files/prepareUpload": () => ({ stagingPath: "/tmp/dbx-sshx.1/report.log" }),
      "docker/files/import": () => {
        throw new Error("cp failed");
      },
    });
    const { transfer, showError } = setup();

    await expect(transfer.upload(new File(["ab"], "report.log"))).rejects.toThrow("cp failed");

    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.objectContaining({
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    }));
    expect(showError).toHaveBeenCalled();
  });

  it("asks before overwriting an existing destination file", async () => {
    mockRpc({
      "docker/files/stat": () => ({ kind: "file", size: 3 }),
      "docker/files/prepareUpload": () => ({ stagingPath: "/tmp/dbx-sshx.1/report.log" }),
    });
    const { transfer, confirmDialog, uploadSource } = setup({ confirmOk: false });

    await transfer.upload(new File(["abcd"], "report.log"));

    expect(rpc).toHaveBeenCalledWith("docker/files/stat", expect.objectContaining({
      path: "/srv/app/report.log",
    }));
    expect(confirmDialog).toHaveBeenCalledWith("docker.filesOverwriteConfirm");
    expect(uploadSource).not.toHaveBeenCalled();
    expect(rpc).not.toHaveBeenCalledWith("docker/files/prepareUpload", expect.anything());
  });

  it("uploads after the overwrite is confirmed", async () => {
    mockRpc({
      "docker/files/stat": () => ({ kind: "file", size: 3 }),
      "docker/files/prepareUpload": () => ({ stagingPath: "/tmp/dbx-sshx.1/report.log" }),
    });
    const { transfer, uploadSource } = setup({ confirmOk: true });

    await transfer.upload(new File(["abcd"], "report.log"));

    expect(uploadSource).toHaveBeenCalled();
  });

  it("disables uploads on read-only connections", async () => {
    const { transfer, showError } = setup({ canWrite: false });

    await expect(transfer.upload(new File(["ab"], "report.log"))).rejects.toThrow(
      "docker.filesUnsupported",
    );

    expect(rpc).not.toHaveBeenCalledWith("docker/files/prepareUpload", expect.anything());
    expect(showError).toHaveBeenCalled();
  });

  it("rejects oversized uploads before creating staging", async () => {
    const { transfer } = setup();

    await expect(transfer.upload(oversizedFile())).rejects.toThrow("docker.filesTooLarge");

    expect(rpc).not.toHaveBeenCalledWith("docker/files/prepareUpload", expect.anything());
  });
});

function decodeBase64(encoded: string): Uint8Array {
  const binary = atob(encoded);
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

describe("local container downloads", () => {
  it("saves a local staging file to Downloads and cleans up", async () => {
    mockRpc({
      "docker/files/export": () => localExport(),
      "docker/files/saveLocal": () => ({ localPath: "/Users/me/Downloads/report.log" }),
    });
    const { transfer, showNotice } = setup({ local: true });

    await transfer.download(file);

    expect(rpc).toHaveBeenCalledWith("docker/files/saveLocal", {
      target: "local",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
      name: "report.log",
    });
    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", {
      target: "local",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    });
    const [message, actions] = showNotice.mock.calls[0]!;
    expect(message).toBe("downloadedTo");
    expect(actions?.[0]?.label).toBe("revealInFolder");
    actions?.[0]?.run();
    expect(rpc).toHaveBeenCalledWith("local/reveal", { path: "/Users/me/Downloads/report.log" });
  });

  it("rejects an export that reports more than 512 MiB and still cleans up", async () => {
    mockRpc({
      "docker/files/export": () => ({ ...localExport(), size: MAX_BYTES + 1 }),
    });
    const { transfer } = setup({ local: true });

    await expect(transfer.download(file)).rejects.toThrow("docker.filesTooLarge");

    expect(rpc).not.toHaveBeenCalledWith("docker/files/saveLocal", expect.anything());
    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.objectContaining({
      target: "local",
      stagingPath: "/tmp/dbx-sshx.1/report.log",
    }));
  });
});

describe("local container uploads", () => {
  it("uploads chunks in order, finishes, imports and cleans up", async () => {
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u1", maxBytes: MAX_BYTES }),
      "docker/files/upload/chunk": () => ({ received: 3 }),
      "docker/files/upload/finish": () => ({
        stagingPath: "/tmp/dbx-sshx-upload-u1/report.log",
        size: 3,
      }),
    });
    const { transfer } = setup({ local: true });

    await transfer.upload(new File(["abc"], "report.log", { type: "text/plain" }));

    expect(rpc).toHaveBeenCalledWith("docker/files/upload/begin", {
      target: "local",
      name: "report.log",
    });
    expect(rpc).toHaveBeenCalledWith("docker/files/upload/chunk", {
      target: "local",
      uploadId: "u1",
      offset: 0,
      dataBase64: "YWJj",
    });
    expect(rpc).toHaveBeenCalledWith("docker/files/upload/finish", {
      target: "local",
      uploadId: "u1",
    });
    expect(rpc).toHaveBeenCalledWith("docker/files/import", expect.objectContaining({
      target: "local",
      containerId: "cid",
      stagingPath: "/tmp/dbx-sshx-upload-u1/report.log",
      destPath: "/srv/app/report.log",
    }));
    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", {
      target: "local",
      stagingPath: "/tmp/dbx-sshx-upload-u1/report.log",
    });
  });

  it("splits files above 256 KiB into ordered chunks", async () => {
    const size = 300 * 1024;
    const receivedOffsets: number[] = [];
    const chunkSizes: number[] = [];
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u2", maxBytes: MAX_BYTES }),
      "docker/files/upload/chunk": (params) => {
        receivedOffsets.push(params.offset as number);
        const bytes = decodeBase64(params.dataBase64 as string);
        chunkSizes.push(bytes.length);
        return { received: (params.offset as number) + bytes.length };
      },
      "docker/files/upload/finish": () => ({
        stagingPath: "/tmp/dbx-sshx-upload-u2/big.bin",
        size,
      }),
    });
    const { transfer } = setup({ local: true, currentPath: "/" });
    const bytes = new Uint8Array(size);
    bytes[0] = 1;
    bytes[256 * 1024] = 2;

    await transfer.upload(new File([bytes], "big.bin"));

    expect(receivedOffsets).toEqual([0, 256 * 1024]);
    expect(chunkSizes).toEqual([256 * 1024, size - 256 * 1024]);
    expect(rpc).toHaveBeenCalledWith("docker/files/import", expect.objectContaining({
      stagingPath: "/tmp/dbx-sshx-upload-u2/big.bin",
      destPath: "/big.bin",
    }));
  });

  it("cancels and cleans up when the received offset does not advance", async () => {
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u3", maxBytes: MAX_BYTES }),
      // 后端只接受顺序写入：回包不推进 = 协议异常，必须中止并取消上传。
      "docker/files/upload/chunk": () => ({ received: 0 }),
    });
    const { transfer, showError } = setup({ local: true });

    await expect(transfer.upload(new File(["abc"], "report.log"))).rejects.toThrow(
      "docker.filesTransferFailed",
    );

    expect(rpc).toHaveBeenCalledWith("docker/files/upload/cancel", {
      target: "local",
      uploadId: "u3",
    });
    expect(showError).toHaveBeenCalled();
  });

  it("exposes byte progress and cancels an in-flight upload", async () => {
    let releaseChunk: ((value: unknown) => void) | undefined;
    const chunkGate = new Promise<unknown>((resolve) => {
      releaseChunk = resolve;
    });
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u4", maxBytes: MAX_BYTES }),
      "docker/files/upload/chunk": () => chunkGate,
      "docker/files/upload/finish": () => ({ stagingPath: "/tmp/dbx-sshx-upload-u4/report.log" }),
    });
    const { transfer, showNotice } = setup({ local: true });
    const picked = new File(["abc"], "report.log");

    const pending = transfer.upload(picked);
    await vi.waitFor(() => {
      expect(rpc).toHaveBeenCalledWith("docker/files/upload/chunk", expect.anything());
    });
    expect(transfer.uploading.value).toMatchObject({ name: "report.log", percent: 0 });

    await transfer.cancelUpload();
    releaseChunk?.({ received: 3 });

    await expect(pending).rejects.toThrow("docker.filesCancel");
    expect(rpc).toHaveBeenCalledWith("docker/files/upload/cancel", {
      target: "local",
      uploadId: "u4",
    });
    expect(showNotice).toHaveBeenCalledWith("docker.filesCancel");
    expect(transfer.uploading.value).toBeUndefined();
  });

  it("rejects oversized local uploads before begin", async () => {
    const { transfer } = setup({ local: true });

    await expect(transfer.upload(oversizedFile())).rejects.toThrow("docker.filesTooLarge");

    expect(rpc).not.toHaveBeenCalledWith("docker/files/upload/begin", expect.anything());
  });

  it("cleans up the staging file when the local import fails", async () => {
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u5", maxBytes: MAX_BYTES }),
      "docker/files/upload/chunk": () => ({ received: 3 }),
      "docker/files/upload/finish": () => ({ stagingPath: "/tmp/dbx-sshx-upload-u5/report.log" }),
      "docker/files/import": () => {
        throw new Error("cp failed");
      },
    });
    const { transfer } = setup({ local: true });

    await expect(transfer.upload(new File(["abc"], "report.log"))).rejects.toThrow("cp failed");

    expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.objectContaining({
      target: "local",
      stagingPath: "/tmp/dbx-sshx-upload-u5/report.log",
    }));
  });
});

describe("local upload lifecycle", () => {
  it("cancels the in-flight upload when the tab is disposed", async () => {
    let releaseChunk: ((value: unknown) => void) | undefined;
    const chunkGate = new Promise<unknown>((resolve) => {
      releaseChunk = resolve;
    });
    mockRpc({
      "docker/files/upload/begin": () => ({ uploadId: "u6", maxBytes: MAX_BYTES }),
      "docker/files/upload/chunk": () => chunkGate,
    });
    const { transfer } = setup({ local: true });

    const pending = transfer.upload(new File(["abc"], "report.log"));
    await vi.waitFor(() => {
      expect(rpc).toHaveBeenCalledWith("docker/files/upload/chunk", expect.anything());
    });
    transfer.dispose();
    releaseChunk?.({ received: 3 });

    await expect(pending).rejects.toThrow("docker.filesCancel");
    expect(rpc).toHaveBeenCalledWith("docker/files/upload/cancel", {
      target: "local",
      uploadId: "u6",
    });
  });
});
