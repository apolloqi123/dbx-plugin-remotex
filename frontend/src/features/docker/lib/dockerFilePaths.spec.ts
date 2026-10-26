import { describe, expect, it } from "vitest";
import {
  breadcrumbs,
  containerFileListPayload,
  isMissingLsError,
  joinContainerPath,
  normalizeContainerPath,
  parentPath,
  sortEntries,
  type ContainerFileEntry,
} from "./dockerFilePaths";

describe("container path helpers", () => {
  it("normalizes repeated separators and dot segments", () => {
    expect(normalizeContainerPath("/var//log/../lib/")).toBe("/var/lib");
    expect(normalizeContainerPath("")).toBe("/");
    expect(normalizeContainerPath("/")).toBe("/");
    expect(normalizeContainerPath("var/log")).toBe("/var/log");
    expect(normalizeContainerPath("/../../etc")).toBe("/etc");
    expect(normalizeContainerPath("/srv/app/.")).toBe("/srv/app");
  });

  it("climbs to the parent path and stops at the root", () => {
    expect(parentPath("/var/lib")).toBe("/var");
    expect(parentPath("/var")).toBe("/");
    expect(parentPath("/")).toBe("/");
    expect(parentPath("/var/lib/")).toBe("/var");
  });

  it("joins names onto a directory path", () => {
    expect(joinContainerPath("/srv/app", "report.log")).toBe("/srv/app/report.log");
    expect(joinContainerPath("/", "etc")).toBe("/etc");
    expect(joinContainerPath("/srv/app/", "../x")).toBe("/srv/x");
  });

  it("builds clickable breadcrumb ancestors", () => {
    expect(breadcrumbs("/")).toEqual([{ name: "/", path: "/" }]);
    expect(breadcrumbs("/var/log")).toEqual([
      { name: "/", path: "/" },
      { name: "var", path: "/var" },
      { name: "log", path: "/var/log" },
    ]);
  });
});

describe("container file list payload", () => {
  it("sorts directories first, then names ascending", () => {
    const entries: ContainerFileEntry[] = [
      { name: "b.txt", path: "/b.txt", kind: "file" },
      { name: "zdir", path: "/zdir", kind: "dir" },
      { name: "adir", path: "/adir", kind: "dir" },
      { name: "a.txt", path: "/a.txt", kind: "file" },
    ];
    expect(sortEntries(entries).map((entry) => entry.name)).toEqual([
      "adir",
      "zdir",
      "a.txt",
      "b.txt",
    ]);
  });

  it("drops malformed rows instead of trusting the bridge", () => {
    const payload = containerFileListPayload(
      {
        path: "/var//log/",
        entries: [
          { name: "lib", kind: "dir" },
          { name: "app.log", path: "/var/log/app.log" },
          { name: 7, path: "/var/log/bad" },
          null,
          "junk",
          { name: "", path: "/var/log/empty" },
          { name: "hidden", path: "/var/log/.hidden", kind: "weird" },
        ],
      },
      "/fallback",
    );
    expect(payload.path).toBe("/var/log");
    expect(payload.entries).toEqual([
      { name: "lib", path: "/var/log/lib", kind: "dir" },
      { name: "app.log", path: "/var/log/app.log", kind: "file" },
      { name: "hidden", path: "/var/log/.hidden", kind: "file" },
    ]);
  });

  it("falls back to the requested path when entries are not an array", () => {
    expect(containerFileListPayload({ path: 3, entries: "nope" }, "/srv").entries).toEqual([]);
    expect(containerFileListPayload({ path: 3, entries: "nope" }, "/srv").path).toBe("/srv");
  });

  it("recognizes a container without ls", () => {
    expect(isMissingLsError('docker exec failed: ls: executable file not found in $PATH')).toBe(true);
    expect(isMissingLsError("OCI runtime exec failed: exec: \"ls\": executable file not found in $PATH")).toBe(true);
    expect(isMissingLsError("docker files/list failed (exit 1): Permission denied")).toBe(false);
  });
});
