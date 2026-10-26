// @vitest-environment happy-dom
// 上传显示名（Task 11）：容器上传写入的是服务端消毒后的暂存文件名，任务卡
// 必须展示用户选中的原文件名；远端路径不受影响。
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { TransferTask } from "../lib/transferQueue";
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
        const waiter = ops.uploadAckWaiters.get("t1");
        if (waiter) {
          window.clearTimeout(waiter.timer);
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
});
