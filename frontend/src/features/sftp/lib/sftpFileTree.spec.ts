import { describe, expect, it } from "vitest";
import type { SftpEntryKind, SftpSanitizedEntry } from "./sftpEntries";
import {
  applyFileTreeChildren,
  buildFileTreeChildren,
  createFileTreeRoot,
  findFileTreeNode,
  flattenVisibleTreeRows,
  isSftpTreeRequestCurrent,
  markCollapsedTreeNodesStale,
  markFileTreeStale,
  type SftpFileTreeNode,
} from "./sftpFileTree";

function entry(name: string, kind: SftpEntryKind = "file", parent = "/"): SftpSanitizedEntry {
  const path = parent === "/" ? `/${name}` : `${parent}/${name}`;
  return { name, uri: `sftp:${path}`, kind };
}

function filledRoot(entries: SftpSanitizedEntry[]): SftpFileTreeNode {
  const root = createFileTreeRoot();
  applyFileTreeChildren(root, "/", entries);
  return root;
}

function paths(root: SftpFileTreeNode, filter = {}): string[] {
  return flattenVisibleTreeRows(root, filter).map((row) => row.node.path);
}

describe("sftp file tree model", () => {
  it("creates a collapsed, unloaded directory root with an sftp uri", () => {
    const root = createFileTreeRoot();
    expect(root).toMatchObject({
      path: "/",
      name: "/",
      kind: "directory",
      expanded: false,
      loaded: false,
      loading: false,
      children: [],
    });
    expect(root.entry).toMatchObject({ uri: "sftp:/", kind: "directory", name: "/" });
  });

  it("builds folders and files, filters hidden names and sorts folders first numerically", () => {
    const nodes = buildFileTreeChildren([
      entry("file10"),
      entry("beta", "directory"),
      entry("notes.txt"),
      entry("alpha", "directory"),
      entry("file2"),
      entry("latest", "symlink"),
      entry(".secret"),
      entry(".env"),
      entry("__pycache__", "directory"),
    ]);

    expect(nodes.map((node) => node.name)).toEqual([
      "alpha",
      "beta",
      "file2",
      "file10",
      "latest",
      "notes.txt",
    ]);
    expect(nodes.map((node) => node.path)).toEqual([
      "/alpha",
      "/beta",
      "/file2",
      "/file10",
      "/latest",
      "/notes.txt",
    ]);
    expect(nodes.map((node) => node.entry.kind)).toEqual([
      "directory",
      "directory",
      "file",
      "file",
      "symlink",
      "file",
    ]);
  });

  it("includes hidden entries when showHidden is true", () => {
    const nodes = buildFileTreeChildren(
      [entry("beta", "directory"), entry(".secret"), entry("__pycache__", "directory"), entry(".env")],
      true,
    );
    expect(nodes).toHaveLength(4);
    expect(nodes.map((node) => node.name)).toEqual(expect.arrayContaining([".secret", "__pycache__", "beta", ".env"]));
    expect(nodes.slice(0, 2).every((node) => node.kind === "directory")).toBe(true);
  });

  it("applies children, finds nodes, expands the parent and drops stale apply results", () => {
    const root = createFileTreeRoot();
    const parent = applyFileTreeChildren(root, "/", [entry("a", "directory"), entry("b.txt")]);
    expect(parent).toBe(root);
    expect(root.loaded).toBe(true);
    expect(root.expanded).toBe(true);
    expect(findFileTreeNode(root, "/a")?.name).toBe("a");
    expect(findFileTreeNode(root, "/missing")).toBeNull();
    expect(applyFileTreeChildren(root, "/missing", [entry("x")])).toBeNull();
  });

  it("preserves a child folder's expanded state and loaded subtree when refreshing its parent", () => {
    const root = createFileTreeRoot();
    applyFileTreeChildren(root, "/", [entry("a", "directory")]);
    const a = findFileTreeNode(root, "/a")!;
    applyFileTreeChildren(root, "/a", [entry("inner.txt", "file", "/a")]);
    a.expanded = false;

    applyFileTreeChildren(root, "/", [entry("a", "directory"), entry("new.txt")]);

    const refreshed = findFileTreeNode(root, "/a")!;
    expect(refreshed).not.toBe(a);
    expect(refreshed.expanded).toBe(false);
    expect(refreshed.loaded).toBe(true);
    expect(refreshed.children.map((child) => child.name)).toEqual(["inner.txt"]);
  });

  it("marks the whole tree stale while keeping structure", () => {
    const root = createFileTreeRoot();
    applyFileTreeChildren(root, "/", [entry("a", "directory")]);
    applyFileTreeChildren(root, "/a", [entry("inner.txt", "file", "/a")]);

    markFileTreeStale(root);

    expect(root.loaded).toBe(false);
    expect(root.expanded).toBe(true);
    expect(root.children).toHaveLength(1);
    expect(findFileTreeNode(root, "/a")?.loaded).toBe(false);
    expect(findFileTreeNode(root, "/a")?.children).toHaveLength(1);
  });

  it("marks only loaded collapsed directories stale so the next expansion re-lists them", () => {
    const root = createFileTreeRoot();
    applyFileTreeChildren(root, "/", [entry("open", "directory"), entry("closed", "directory")]);
    applyFileTreeChildren(root, "/open", [entry("fresh.txt", "file", "/open")]);
    applyFileTreeChildren(root, "/closed", [entry("old.txt", "file", "/closed")]);
    findFileTreeNode(root, "/closed")!.expanded = false;

    markCollapsedTreeNodesStale(root);

    expect(findFileTreeNode(root, "/open")!.loaded).toBe(true);
    expect(findFileTreeNode(root, "/closed")!.loaded).toBe(false);
    // Structure is kept while collapsed (batch counts), but reloading the
    // parent must not resurrect the stale subtree for an unloaded child.
    expect(findFileTreeNode(root, "/closed")!.children.map((child) => child.name)).toEqual(["old.txt"]);
    applyFileTreeChildren(root, "/", [entry("open", "directory"), entry("closed", "directory")]);
    expect(findFileTreeNode(root, "/closed")!.loaded).toBe(false);
    expect(findFileTreeNode(root, "/closed")!.children).toEqual([]);
  });

  it("drops late tree requests after epoch, session or node replacement", () => {
    const root = createFileTreeRoot();
    applyFileTreeChildren(root, "/", [entry("a", "directory")]);
    const node = findFileTreeNode(root, "/a")!;
    const current = { epoch: 3, sessionId: "session-a", root };

    expect(isSftpTreeRequestCurrent({ epoch: 3, sessionId: "session-a", node }, current)).toBe(true);
    expect(isSftpTreeRequestCurrent({ epoch: 2, sessionId: "session-a", node }, current)).toBe(false);
    expect(isSftpTreeRequestCurrent({ epoch: 3, sessionId: "session-b", node }, current)).toBe(false);
    expect(isSftpTreeRequestCurrent({ epoch: 3, sessionId: undefined, node }, current)).toBe(false);

    // A parent refresh replaces the child node object: an old toggle response
    // must not write children into the replacement.
    applyFileTreeChildren(root, "/", [entry("a", "directory")]);
    const replaced = findFileTreeNode(root, "/a")!;
    expect(replaced).not.toBe(node);
    expect(isSftpTreeRequestCurrent({ epoch: 3, sessionId: "session-a", node }, current)).toBe(false);
    expect(isSftpTreeRequestCurrent({ epoch: 3, sessionId: "session-a", node: replaced }, current)).toBe(true);
  });

  it("flattens only expanded folders", () => {
    const root = filledRoot([entry("docs", "directory"), entry("readme.txt")]);
    applyFileTreeChildren(root, "/docs", [entry("nested.txt", "file", "/docs")]);

    expect(paths(root)).toEqual(["/", "/docs", "/docs/nested.txt", "/readme.txt"]);
    expect(flattenVisibleTreeRows(root).map((row) => row.depth)).toEqual([0, 1, 2, 1]);

    findFileTreeNode(root, "/docs")!.expanded = false;
    expect(paths(root)).toEqual(["/", "/docs", "/readme.txt"]);

    root.expanded = false;
    expect(paths(root)).toEqual(["/"]);
  });

  it("includes matching descendants and their ancestors when searching", () => {
    const root = filledRoot([entry("docs", "directory"), entry("readme.md")]);
    applyFileTreeChildren(root, "/docs", [
      entry("target.txt", "file", "/docs"),
      entry("other.txt", "file", "/docs"),
    ]);
    findFileTreeNode(root, "/docs")!.expanded = true;

    expect(paths(root, { search: "target" })).toEqual(["/", "/docs", "/docs/target.txt"]);
    expect(paths(root, { search: "other" })).toEqual(["/", "/docs", "/docs/other.txt"]);
    expect(flattenVisibleTreeRows(root, { search: "target" }).map((row) => row.depth)).toEqual([0, 1, 2]);
  });

  it("keeps a folder whose own name matches the search", () => {
    const root = filledRoot([entry("docs", "directory"), entry("readme.md")]);
    applyFileTreeChildren(root, "/docs", [entry("nested.txt", "file", "/docs")]);
    findFileTreeNode(root, "/docs")!.expanded = true;

    expect(paths(root, { search: "docs" })).toEqual(["/", "/docs"]);
  });

  it("returns no rows when nothing matches and ignores surrounding search whitespace", () => {
    const root = filledRoot([entry("docs", "directory"), entry("readme.md")]);
    expect(paths(root, { search: "missing" })).toEqual([]);
    expect(paths(root, { search: "  " })).toEqual(["/", "/docs", "/readme.md"]);
  });

  it("filters by type while keeping directory ancestors of matching files", () => {
    const root = filledRoot([entry("with-file", "directory"), entry("empty", "directory"), entry("root.txt")]);
    applyFileTreeChildren(root, "/with-file", [entry("data.bin", "file", "/with-file")]);
    applyFileTreeChildren(root, "/empty", []);
    findFileTreeNode(root, "/with-file")!.expanded = true;
    findFileTreeNode(root, "/empty")!.expanded = true;

    expect(paths(root, { type: "directory" })).toEqual(["/", "/empty", "/with-file"]);
    expect(paths(root, { type: "file" })).toEqual(["/", "/with-file", "/with-file/data.bin", "/root.txt"]);
    expect(paths(root, { type: "all" })).toEqual([
      "/",
      "/empty",
      "/with-file",
      "/with-file/data.bin",
      "/root.txt",
    ]);
  });

  it("combines search and type filters", () => {
    const root = filledRoot([entry("docs", "directory"), entry("root.txt")]);
    applyFileTreeChildren(root, "/docs", [
      entry("target.txt", "file", "/docs"),
      entry("target-folder", "directory", "/docs"),
    ]);
    findFileTreeNode(root, "/docs")!.expanded = true;

    expect(paths(root, { search: "target", type: "file" })).toEqual([
      "/",
      "/docs",
      "/docs/target-folder",
      "/docs/target.txt",
    ]);
    expect(paths(root, { search: "target", type: "directory" })).toEqual(["/", "/docs", "/docs/target-folder"]);
  });
});
