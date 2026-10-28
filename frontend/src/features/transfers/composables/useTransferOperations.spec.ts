// @vitest-environment happy-dom
// 上传显示名（Task 11）：容器上传写入的是服务端消毒后的暂存文件名，任务卡
// 必须展示用户选中的原文件名；远端路径不受影响。
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { TransferTask } from "../lib/transferQueue";
import { UPLOAD_SPOOL_WINDOW, resolveUploadAck } from "../lib/uploadAck";
import { useTransferOperations } from "./useTransferOperations";

const globalWindow = window as unknown as { dbxPlugin?: unknown };

describe("useTransferOperations upload display name", () => {
  it("uses displayName for the task card while uploading to the sanitized path", async () => {
    const starts: Array<Record<string, unknown>> = [];
    const transferTasks: Record<string, TransferTask> = {};
    const ops = useTransferOperations({
      t: (key) => key,
      session: ref({ sessionId: "s1" }),
      currentPath: ref("/srv/app"),
      resolveUploadDuplicateName: async () => ({ proceed: true, name: "unused" }),
      joinRemote: (dir, name) => `${dir}/${name}`,
      writeU64: (bytes, offset, value) => {
        new DataView(bytes.buffer).setBigUint64(offset, BigInt(value));
      },
      showError: vi.fn(),
      transferTasks,
      waitForTransferCompletion: async () => undefined,
      sudoUpload: () => false,
    });
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async (method: string, params?: unknown) => {
        if (method === "sftp/upload/start") {
          starts.push(params as Record<string, unknown>);
          return { taskId: "t1", chunkSize: 3 };
        }
        return {};
      }),
      sendBinary: vi.fn(async () => {
        const waiters = ops.uploadAckWaiters.get("t1");
        const waiter = waiters?.[0];
        if (waiter) {
          window.clearTimeout(waiter.timer);
          waiters!.shift();
          waiter.resolve();
        }
      }),
    };

    await ops.uploadSource(
      "sanitized.bin",
      3,
      async () => new Uint8Array([1, 2, 3]),
      undefined,
      "/tmp/dbx-sshx.1",
      { duplicatePreCheckedAbsent: true, displayName: "report.log" },
    );

    // 暂存路径仍用消毒后的名字，任务卡展示用户选中的原文件名。
    expect(starts[0]).toMatchObject({ remotePath: "/tmp/dbx-sshx.1/sanitized.bin" });
    expect(transferTasks.t1?.fileName).toBe("report.log");
  });

  it("keeps the spool window in flight instead of waiting for each ack", async () => {
    const transferTasks: Record<string, TransferTask> = {};
    const ops = useTransferOperations({
      t: (key) => key,
      session: ref({ sessionId: "s1" }),
      currentPath: ref("/srv/app"),
      resolveUploadDuplicateName: async () => ({ proceed: true, name: "unused" }),
      joinRemote: (dir, name) => `${dir}/${name}`,
      writeU64: (bytes, offset, value) => {
        new DataView(bytes.buffer).setBigUint64(offset, BigInt(value));
      },
      showError: vi.fn(),
      transferTasks,
      waitForTransferCompletion: async () => undefined,
      sudoUpload: () => false,
    });
    const sendBinary = vi.fn(async () => undefined);
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async (method: string) => {
        if (method === "sftp/upload/start") return { taskId: "t1", chunkSize: 1 };
        return {};
      }),
      sendBinary,
    };

    const upload = ops.uploadSource(
      "a.bin",
      3,
      async (offset) => new Uint8Array([offset]),
      undefined,
      "/tmp",
      { duplicatePreCheckedAbsent: true },
    );

    // 窗口未满时三片都应先发出（不逐片等 ack）。
    await vi.waitFor(() => expect(sendBinary).toHaveBeenCalledTimes(UPLOAD_SPOOL_WINDOW));
    expect(ops.uploadAckWaiters.get("t1")?.map((waiter) => waiter.nextOffset)).toEqual([1, 2, 3]);

    for (const nextOffset of [1, 2, 3]) {
      expect(resolveUploadAck(ops.uploadAckWaiters, "t1", nextOffset)).toBe(true);
    }
    await upload;
    expect(ops.uploadAckWaiters.size).toBe(0);
  });
});
