import { describe, expect, it } from "vitest";
import {
  filterSelectableSftpEntries,
  filterSelectableSftpUris,
  isProtectedSftpDirectoryPath,
  resolveSftpContextMenuSelection,
} from "./sftpContextMenu";

const pathFromUri = (uri: string) => uri.replace(/^sftp:/, "");

describe("sftp context menu decisions", () => {
  it("replaces the selection when right-clicking an unselected row", () => {
    expect(resolveSftpContextMenuSelection("sftp:/home/b", ["sftp:/home/a"])).toEqual(["sftp:/home/b"]);
    // Empty selection behaves the same: the clicked row becomes the selection.
    expect(resolveSftpContextMenuSelection("sftp:/home/b", [])).toEqual(["sftp:/home/b"]);
  });

  it("keeps an existing multi-selection when the clicked row is part of it", () => {
    expect(resolveSftpContextMenuSelection("sftp:/home/b", ["sftp:/home/a", "sftp:/home/b"])).toBeNull();
    expect(resolveSftpContextMenuSelection("sftp:/home/a", ["sftp:/home/a"])).toBeNull();
  });

  it("protects the root, the current directory and its ancestors", () => {
    expect(isProtectedSftpDirectoryPath("/", "/home/user")).toBe(true);
    expect(isProtectedSftpDirectoryPath("/home/user", "/home/user")).toBe(true);
    expect(isProtectedSftpDirectoryPath("/home", "/home/user")).toBe(true);
    expect(isProtectedSftpDirectoryPath("/home/", "/home/user")).toBe(true);
  });

  it("keeps unrelated siblings and descendants unprotected", () => {
    expect(isProtectedSftpDirectoryPath("/home/other", "/home/user")).toBe(false);
    expect(isProtectedSftpDirectoryPath("/home/user/docs", "/home/user")).toBe(false);
    // Segment boundaries: /home2 is not an ancestor of /home/user.
    expect(isProtectedSftpDirectoryPath("/home2", "/home/user")).toBe(false);
    expect(isProtectedSftpDirectoryPath("/home/user2", "/home/user")).toBe(false);
  });

  it("normalizes encoded and non-canonical paths before comparing", () => {
    expect(isProtectedSftpDirectoryPath("/home/../home", "/home/user")).toBe(true);
    expect(isProtectedSftpDirectoryPath("/home%2Fuser", "/home/user")).toBe(true);
  });
});

describe("selectable SFTP selection filter", () => {
  const entries = [
    { uri: "sftp:/", kind: "directory" },
    { uri: "sftp:/home", kind: "directory" },
    { uri: "sftp:/home/user", kind: "directory" },
    { uri: "sftp:/home/other", kind: "directory" },
    { uri: "sftp:/home/user/notes.txt", kind: "file" },
    { uri: "sftp:/home/user/link", kind: "symlink" },
  ];

  it("drops root, current and ancestor directories while keeping files and unrelated dirs", () => {
    const kept = filterSelectableSftpEntries(entries, "/home/user", pathFromUri).map((entry) => entry.uri);
    expect(kept).toEqual(["sftp:/home/other", "sftp:/home/user/notes.txt", "sftp:/home/user/link"]);
  });

  it("filters URI selections without touching file URIs", () => {
    const directoryUris = new Set(entries.filter((entry) => entry.kind === "directory").map((entry) => entry.uri));
    const kept = filterSelectableSftpUris(
      entries.map((entry) => entry.uri),
      "/home/user",
      pathFromUri,
      (uri) => directoryUris.has(uri),
    );
    expect(kept).toEqual(["sftp:/home/other", "sftp:/home/user/notes.txt", "sftp:/home/user/link"]);
    // 未知 URI 按非目录保留，不会因为树尚未加载而误删选择。
    expect(filterSelectableSftpUris(["sftp:/unknown"], "/home/user", pathFromUri, () => false)).toEqual(["sftp:/unknown"]);
  });
});
