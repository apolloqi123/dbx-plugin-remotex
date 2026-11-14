/**
 * SFTP 文件域夹具：sftp/* 与 sudo 文件别名（list/read/write/copy/move/rename/
 * delete/archive/extract/stat/chmod/mkdir/touch）+ 书签（sftp/bookmarks/*）。
 * 全部落在 mockFileSystem 内存树上，镜像 sidecar 的错误串与返回形状。
 */

import { UNHANDLED, type Unhandled, base64 } from "./shared";
import { mockFileSystem } from "../mockFileSystem";

// SFTP 书签（sftp/bookmarks/*）mock 状态：镜像 lib/sftpBookmarks.ts 的
// 校验形状（label/path 非空、返回完整 bookmark 记录），防"添加书签"在
// 夹具里报 invalid bookmark。
interface MockBookmark { id: string; label: string; path: string; createdAt: number; updatedAt: number }
const bookmarksState: MockBookmark[] = [];
let bookmarkSeq = 0;

/** 从 mock 文件树摘除单个节点（sftp/delete 与批量删除共用的删除单元）。 */
function removeMockNode(path: string) {
  const { parent, name } = mockFileSystem.parentAndName(path);
  const index = parent?.children?.findIndex((child) => child.name === name) ?? -1;
  if (!parent || index < 0) throw new Error(`sftp: no such file: ${name}`);
  parent.children!.splice(index, 1);
}

/** 后端 normalize_delete_items 的 mock 镜像：归一 + 去重 + 祖先折叠。按段
 *  判祖先（`/ab` 不算 `/a` 的后代），父子同时选中时只删祖先——否则父项先
 *  删掉后子项会被误报进 failed。 */
function collapseMockDeletePaths(paths: string[]): string[] {
  const unique = [...new Set(paths.map((path) => mockFileSystem.normalizePath(path)))];
  const selected = new Set(unique);
  const hasSelectedAncestor = (path: string) => {
    let cursor = path;
    for (;;) {
      const index = cursor.lastIndexOf("/");
      if (index <= 0) return false;
      cursor = cursor.slice(0, index);
      if (selected.has(cursor)) return true;
    }
  };
  return unique.filter((path) => !hasSelectedAncestor(path));
}

export async function handleSftpInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "sftp/list" || method === "sudo/listDir") return { entries: mockFileSystem.list(String((params as Record<string, unknown>)?.path || "/")) };
  if (method === "sftp/home") return { path: "/home/demo" };
  if (method === "sftp/createDirectory" || method === "sudo/mkdir") return mockFileSystem.writeEntry(String((params as Record<string, unknown>)?.path || ""), mockFileSystem.dir(String((params as Record<string, unknown>)?.path || "/").split("/").pop() || "folder"));
  if (method === "sftp/touch") return mockFileSystem.writeEntry(String((params as Record<string, unknown>)?.path || ""), mockFileSystem.file(String((params as Record<string, unknown>)?.path || "").split("/").pop() || "file.txt", 0));
  // 撞名探测（上传 auto-rename 策略）：mock 文件树不做重名裁决，恒按
  // "目标不存在"返回原名（后端同名响应形状：{name, conflict}）。
  if (method === "sftp/rename-unique") {
    const input = params as Record<string, unknown>;
    return { name: String(input.name || ""), conflict: false };
  }
  if (method === "sftp/archive") {
    const input = params as Record<string, unknown>;
    const sources = Array.isArray(input.sourcePaths) ? (input.sourcePaths as string[]) : [];
    const total = sources.reduce((sum, source) => sum + (mockFileSystem.findNode(source)?.size || 1024), 0);
    return mockFileSystem.writeEntry(String(input.archivePath || ""), mockFileSystem.file(String(input.archivePath || "").split("/").pop() || "archive.tar.gz", Math.max(total, 512)));
  }
  if (method === "sftp/extract") {
    const input = params as Record<string, unknown>;
    const destination = String(input.destinationPath || "");
    return mockFileSystem.writeEntry(destination, mockFileSystem.dir(destination.split("/").pop() || "extracted", [mockFileSystem.file("README", 64)]));
  }
  if (method === "sftp/delete" || method === "sudo/remove" || method === "sudo/removeAll") {
    removeMockNode(String((params as Record<string, unknown>)?.path || ""));
    return { success: true };
  }
  // 批量删除（M34）：一次 RPC 逐路径摘除，成功进 deleted、缺失/异常进 failed，
  // 与后端 { deleted, failed } 契约同形（单项失败不中断其余项）。
  if (method === "sftp/deleteMany" || method === "sudo/removeMany") {
    const input = params as Record<string, unknown>;
    const paths = method === "sudo/removeMany"
      ? (Array.isArray(input.paths) ? (input.paths as string[]).map(String) : [])
      : (Array.isArray(input.items)
          ? (input.items as Array<Record<string, unknown>>).map((item) => String(item?.path || ""))
          : []);
    const deleted: string[] = [];
    const failed: Array<{ path: string; error: string }> = [];
    for (const path of collapseMockDeletePaths(paths)) {
      try {
        removeMockNode(path);
        deleted.push(path);
      } catch (cause) {
        failed.push({ path, error: cause instanceof Error ? cause.message : String(cause) });
      }
    }
    return { deleted, failed };
  }
  if (method === "sftp/rename") {
    const input = params as Record<string, unknown>;
    const node = mockFileSystem.findNode(String(input.sourcePath || ""));
    if (!node) throw new Error(`sftp: no such file: ${input.sourcePath}`);
    const { parent: sourceParent, name: sourceName } = mockFileSystem.parentAndName(String(input.sourcePath || ""));
    const targetPath = mockFileSystem.normalizePath(String(input.targetPath || ""));
    const { parent: targetParent, name: targetName } = mockFileSystem.parentAndName(targetPath);
    if (!sourceParent || !targetParent || targetParent.kind !== "directory" || !targetName) throw new Error(`sftp: cannot write ${targetPath}`);
    // 原子语义：先摘除目标位置同名节点（覆盖，对应前端 exists 预检确认），
    // 再摘源、改名、落位——撞名时源文件不再丢失（R3-P2-2 夹具缺陷收口）。
    const targetIndex = targetParent.children?.findIndex((child) => child.name === targetName && child !== node) ?? -1;
    if (targetIndex >= 0) targetParent.children!.splice(targetIndex, 1);
    const sourceIndex = sourceParent.children!.findIndex((child) => child.name === sourceName);
    sourceParent.children!.splice(sourceIndex, 1);
    node.name = targetName;
    targetParent.children!.push(node);
    return { success: true };
  }
  if (method === "sftp/exists") return { exists: !!mockFileSystem.findNode(String((params as Record<string, unknown>)?.path || "")) };
  if (method === "sftp/stat") {
    const node = mockFileSystem.findNode(String((params as Record<string, unknown>)?.path || ""));
    if (!node) throw new Error("sftp: no such file");
    return { path: mockFileSystem.normalizePath(String((params as Record<string, unknown>)?.path || "")), kind: node.kind, ...(node.kind === "file" ? { size: node.size } : {}), modifiedAt: node.modifiedAt, mode: node.permissions };
  }
  if (method === "sftp/chmod" || method === "sudo/chmod") {
    const input = params as Record<string, unknown>;
    const node = mockFileSystem.findNode(String(input.path || ""));
    if (node) node.permissions = String(input.mode || node.permissions);
    return { success: true };
  }
  if (method === "sftp/copy" || method === "sftp/move") {
    // 镜像 sftp/copy | sftp/move 契约：`from`（单个或数组）逐项执行进 `toDir`、
    // 保留基名；overwrite:false 时目标已存在按项失败；逐项回报
    // { from, to, ok, error? }，任一失败 success=false。move 额外摘除源节点。
    const input = params as Record<string, unknown>;
    const sources = Array.isArray(input.from) ? input.from.map(String) : [String(input.from || "")];
    const toDir = mockFileSystem.normalizePath(String(input.toDir || ""));
    const overwrite = input.overwrite === true;
    const targetDir = mockFileSystem.findNode(toDir);
    if (!targetDir || targetDir.kind !== "directory") throw new Error(`sftp: no such directory: ${toDir}`);
    const rows = sources.map((source) => {
      const fromPath = mockFileSystem.normalizePath(source);
      const name = fromPath.split("/").pop() || "";
      const toPath = `${toDir === "/" ? "" : toDir}/${name}`;
      try {
        const node = mockFileSystem.findNode(fromPath);
        if (!node || !name) throw new Error(`sftp: no such file: ${fromPath}`);
        const targetNode = mockFileSystem.findNode(toPath);
        if (targetNode && targetNode !== node && !overwrite) throw new Error(`sftp: target exists: ${toPath}`);
        if (targetNode && targetNode !== node) {
          const { parent } = mockFileSystem.parentAndName(toPath);
          const targetIndex = parent?.children?.indexOf(targetNode) ?? -1;
          if (!parent || targetIndex < 0) throw new Error(`sftp: cannot write ${toPath}`);
          parent.children!.splice(targetIndex, 1);
        }
        if (method === "sftp/move") {
          if (fromPath !== toPath) {
            const { parent: sourceParent } = mockFileSystem.parentAndName(fromPath);
            const sourceIndex = sourceParent?.children?.indexOf(node) ?? -1;
            if (!sourceParent || sourceIndex < 0) throw new Error(`sftp: no such file: ${fromPath}`);
            sourceParent.children!.splice(sourceIndex, 1);
            node.name = name;
            targetDir.children!.push(node);
          }
        } else if (!targetNode || targetNode !== node) {
          const copy = structuredClone(node);
          copy.name = name;
          targetDir.children!.push(copy);
        }
        return { from: fromPath, to: toPath, ok: true };
      } catch (cause) {
        return { from: fromPath, to: toPath, ok: false, error: cause instanceof Error ? cause.message : String(cause) };
      }
    });
    return { success: rows.every((row) => row.ok), results: rows };
  }
  if (method === "sftp/read" || method === "sudo/readFile") {
    const readPath = mockFileSystem.normalizePath(String((params as Record<string, unknown>)?.path || ""));
    const stored = mockFileSystem.readContents(readPath);
    // 已存在的 0 字节文件（sftp/touch / New file 新建）必须回空内容：
    // 之前回退到演示脚本，造成"新建空文件 → 打开是 deploy.sh 内容"的假象。
    const node = mockFileSystem.findNode(readPath);
    const emptyFile = !!node && node.kind === "file" && node.size === 0 && !stored;
    return stored
      ? { dataBase64: base64(stored), truncated: false }
      : emptyFile
        ? { dataBase64: "", truncated: false }
        : { dataBase64: base64(new TextEncoder().encode("#!/usr/bin/env bash\nset -euo pipefail\n\necho deploy\n")), truncated: false };
  }
  if (method === "sftp/write" || method === "sudo/writeFile") {
    // 镜像真实契约：整文件覆写（sftp/write 用 remotePath、sudo/writeFile 用
    // path）。节点必须已存在，内容驻留内存供读回，树节点 size 同步更新。
    const input = params as Record<string, unknown>;
    const writePath = mockFileSystem.normalizePath(String(input.remotePath || input.path || ""));
    const node = mockFileSystem.findNode(writePath);
    if (!node || node.kind !== "file") throw new Error(`sftp: no such file: ${writePath}`);
    const bytes = Uint8Array.from(atob(String(input.dataBase64 || "")), (value) => value.charCodeAt(0));
    mockFileSystem.writeContents(writePath, bytes);
    node.size = bytes.byteLength;
    return { success: true };
  }
  if (method === "sftp/bookmarks/list") {
    return { bookmarks: bookmarksState.map((bookmark) => ({ ...bookmark })) };
  }
  if (method === "sftp/bookmarks/save") {
    const input = (params || {}) as Record<string, unknown>;
    const label = String(input.label || "").trim();
    const path = String(input.path || "").trim();
    if (!label || !path) throw new Error("sftp/bookmarks/save: label and path are required");
    const now = Math.floor(Date.now() / 1000);
    const bookmark: MockBookmark = { id: `bm-visual-${++bookmarkSeq}`, label, path, createdAt: now, updatedAt: now };
    bookmarksState.push(bookmark);
    return { bookmark };
  }
  if (method === "sftp/bookmarks/delete") {
    const id = String((params as Record<string, unknown>)?.id || "");
    const index = bookmarksState.findIndex((bookmark) => bookmark.id === id);
    if (index >= 0) bookmarksState.splice(index, 1);
    return { success: true, removed: index >= 0 };
  }
  return UNHANDLED;
}
