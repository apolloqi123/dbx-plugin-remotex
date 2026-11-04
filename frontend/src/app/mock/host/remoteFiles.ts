/**
 * 远程桌面文件传输浮窗夹具（RDP/VNC 工具栏「文件传输」）。
 *
 * 只提供走查所需的形状：本地/远端各一层目录、连接成功、上传/下载任务立即
 * 完成（发一次 filetransfer/progress + filetransfer/done），以及文件剪贴板
 * 的读写成功。真实传输由 sidecar 的 SSH/SFTP 会话负责。
 */

import { UNHANDLED, emitHostEvent, type Unhandled } from "./shared";

const LOCAL_ROOT = "/Users/mock";
// Matches the shared mock SFTP home (/home/demo) so sftp/list is served by
// the existing fixture; filetransfer/connect only supplies the session id.
const REMOTE_ROOT = "/home/demo";
let taskSeq = 0;

function localEntries(path: string): Array<Record<string, unknown>> {
  const base = path || LOCAL_ROOT;
  if (base === "/") {
    return [
      { name: "Users", path: "/Users", kind: "directory", size: 0, hidden: false, symlink: false },
    ];
  }
  return [
    { name: "Documents", path: `${base}/Documents`, kind: "directory", size: 0, hidden: false, symlink: false },
    { name: "Downloads", path: `${base}/Downloads`, kind: "directory", size: 0, hidden: false, symlink: false },
    { name: "notes.txt", path: `${base}/notes.txt`, kind: "file", size: 1024, hidden: false, symlink: false },
    { name: ".zshrc", path: `${base}/.zshrc`, kind: "file", size: 512, hidden: true, symlink: false },
  ];
}


function finishTask(direction: "upload" | "download", taskId: string): void {
  const params = {
    taskId,
    sessionId: "mock-filetransfer",
    direction,
    status: "done",
    filesDone: 1,
    filesTotal: 1,
    bytesDone: 1024,
    bytesTotal: 1024,
    percent: 100,
  };
  setTimeout(() => {
    emitHostEvent({ method: "filetransfer/progress", params });
    emitHostEvent({
      method: "filetransfer/done",
      params: {
        taskId,
        sessionId: "mock-filetransfer",
        direction,
        filesDone: 1,
        filesTotal: 1,
        ...(direction === "download" ? { localPaths: [`${LOCAL_ROOT}/readme.md`] } : {}),
      },
    });
  }, 120);
}

export async function handleRemoteFilesInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  const input = (params ?? {}) as Record<string, unknown>;
  if (method === "filetransfer/connect") {
    return {
      sessionId: "mock-filetransfer",
      home: REMOTE_ROOT,
      host: typeof input.host === "string" ? input.host : "mock.example",
      username: typeof input.username === "string" ? input.username : "mock",
    };
  }
  if (method === "filetransfer/disconnect") return { success: true };
  if (method === "filetransfer/local/list") {
    const path = typeof input.path === "string" && input.path ? input.path : LOCAL_ROOT;
    return {
      path,
      parent: path === "/" ? null : path.slice(0, Math.max(1, path.lastIndexOf("/"))) || "/",
      home: LOCAL_ROOT,
      drives: [],
      entries: localEntries(path),
    };
  }
  if (method === "filetransfer/upload/start" || method === "filetransfer/download/start") {
    const direction = method.endsWith("upload/start") ? "upload" : "download";
    const taskId = `mock-transfer-${(taskSeq += 1)}`;
    finishTask(direction, taskId);
    return { taskId };
  }
  if (method === "filetransfer/cancel") return { success: true };
  if (method === "filetransfer/ping") return { alive: true };
  if (method === "filetransfer/clipboard/files") {
    return { supported: true, files: [`${LOCAL_ROOT}/notes.txt`] };
  }
  if (method === "filetransfer/credentials/forget") return { success: true };
  if (method === "filetransfer/clipboard/write") return { success: true, count: 1 };
  // The fixture has no X11 clipboard to read; report a supported tool with an
  // empty file list so the VNC copy poll stays quiet.
  if (method === "filetransfer/clipboard/remote") return { supported: true, paths: [] };
  if (method === "filetransfer/clipboard/fetch") {
    return { taskId: `mock-clipboard-${(taskSeq += 1)}` };
  }
  return UNHANDLED;
}
