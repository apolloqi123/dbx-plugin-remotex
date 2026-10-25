// 详情卡纯函数：entry → 详情、stat 合并优先级（stat > ownerName/groupName >
// entry）、以及详情行顺序/格式化（字节、时间、缺失占位）。
import { describe, expect, it } from "vitest";
import type { SftpEntryKind, SftpSanitizedEntry } from "../entries/sftpEntries";
import {
  mergeSftpDetail,
  sftpDetailFromEntry,
  sftpDetailRows,
  type SftpDetailInfo,
} from "./sftpDetails";

const t = (key: string) => key;

function makeEntry(overrides: Partial<SftpSanitizedEntry> = {}): SftpSanitizedEntry {
  return {
    name: "file.txt",
    uri: "sftp:/home/deploy/file.txt",
    kind: "file" as SftpEntryKind,
    ...overrides,
  };
}

describe("sftpDetailFromEntry", () => {
  it("copies the fields available on the entry and uses the fallback path", () => {
    const entry = makeEntry({
      size: 12,
      modifiedAt: 1_700_000_000_000,
      permissions: "-rw-r--r--",
      owner: "deploy",
      group: "staff",
    });

    expect(sftpDetailFromEntry(entry, "/home/deploy/file.txt")).toEqual({
      path: "/home/deploy/file.txt",
      kind: "file",
      size: 12,
      modifiedAt: 1_700_000_000_000,
      permissions: "-rw-r--r--",
      owner: "deploy",
      group: "staff",
    });
  });

  it("omits absent fields and keeps size zero", () => {
    expect(sftpDetailFromEntry(makeEntry(), "/fallback/path")).toEqual({
      path: "/fallback/path",
      kind: "file",
    });
    expect(sftpDetailFromEntry(makeEntry({ size: 0 }), "/x").size).toBe(0);
  });

  it("keeps the directory kind and never invents metadata", () => {
    const detail = sftpDetailFromEntry(makeEntry({ kind: "directory", uri: "sftp:/var" }), "/var");
    expect(detail).toEqual({ path: "/var", kind: "directory" });
  });
});

describe("mergeSftpDetail", () => {
  it("returns the entry detail when stat is missing", () => {
    const entry = makeEntry({ size: 3, permissions: "rw-r--r--", owner: "entry-owner" });
    expect(mergeSftpDetail(entry, null, "/entry/path")).toEqual(sftpDetailFromEntry(entry, "/entry/path"));
    expect(mergeSftpDetail(entry, undefined, "/entry/path")).toEqual(sftpDetailFromEntry(entry, "/entry/path"));
  });

  it("lets stat values win for every field", () => {
    const entry = makeEntry({ size: 3, permissions: "rw-r--r--", owner: "entry-owner", group: "entry-group" });
    const merged = mergeSftpDetail(
      entry,
      {
        path: "/srv/actual",
        kind: "directory",
        size: 0,
        modifiedAt: 42,
        permissions: "drwxr-xr-x",
        owner: "root",
        group: "wheel",
      },
      "/entry/path",
    );

    expect(merged).toEqual({
      path: "/srv/actual",
      kind: "directory",
      size: 0,
      modifiedAt: 42,
      permissions: "drwxr-xr-x",
      owner: "root",
      group: "wheel",
    });
  });

  it("maps the sidecar's stat.mode into permissions", () => {
    const merged = mergeSftpDetail(makeEntry(), { mode: "drwxr-xr-x" }, "/x");
    expect(merged.permissions).toBe("drwxr-xr-x");
  });

  it("falls owner/group back to ownerName/groupName from the stat", () => {
    const merged = mergeSftpDetail(
      makeEntry({ owner: "entry-owner", group: "entry-group" }),
      { ownerName: "alice", groupName: "developers" },
      "/x",
    );

    expect(merged.owner).toBe("alice");
    expect(merged.group).toBe("developers");
    expect(merged.ownerName).toBe("alice");
    expect(merged.groupName).toBe("developers");
  });

  it("prefers stat.owner/stat.group over the *Name fields", () => {
    const merged = mergeSftpDetail(
      makeEntry(),
      { owner: "root", ownerName: "alice", group: "wheel", groupName: "developers" },
      "/x",
    );

    expect(merged.owner).toBe("root");
    expect(merged.group).toBe("wheel");
  });

  it("does not let empty stat values wipe entry data and falls back for the path", () => {
    const entry = makeEntry({
      size: 5,
      modifiedAt: 7,
      permissions: "rw-------",
      owner: "entry-owner",
      group: "entry-group",
    });
    const merged = mergeSftpDetail(
      entry,
      { path: "", size: undefined, modifiedAt: undefined, permissions: "", owner: "", group: "" },
      "/fallback",
    );

    expect(merged.path).toBe("/fallback");
    expect(merged.size).toBe(5);
    expect(merged.modifiedAt).toBe(7);
    expect(merged.permissions).toBe("rw-------");
    expect(merged.owner).toBe("entry-owner");
    expect(merged.group).toBe("entry-group");
  });

  it("keeps an explicit null ownerName without erasing the entry owner fallback", () => {
    const merged = mergeSftpDetail(makeEntry({ owner: "entry-owner" }), { ownerName: null }, "/x");
    expect(merged.owner).toBe("entry-owner");
    expect(merged.ownerName).toBeNull();
  });
});

describe("sftpDetailRows", () => {
  const base: SftpDetailInfo = {
    path: "/var/log/syslog",
    kind: "file",
    size: 1536,
    modifiedAt: Date.UTC(2024, 0, 2, 3, 4),
    permissions: "-rw-r--r--",
    owner: "root",
    group: "adm",
  };

  it("returns the rows in path/type/size/modified/permissions/owner/group order", () => {
    const rows = sftpDetailRows(base, t, "en-US");

    expect(rows.map((row) => row.key)).toEqual([
      "path",
      "type",
      "size",
      "modified",
      "permissions",
      "owner",
      "group",
    ]);
    expect(rows.map((row) => row.label)).toEqual([
      "sftpDetails.path",
      "sftpDetails.type",
      "sftpDetails.size",
      "sftpDetails.modified",
      "sftpDetails.permissions",
      "sftpDetails.owner",
      "sftpDetails.group",
    ]);
  });

  it("formats the path, kind, size and modified time", () => {
    const rows = sftpDetailRows(base, t, "en-US");
    const byKey = new Map(rows.map((row) => [row.key, row.value]));

    expect(byKey.get("path")).toBe("/var/log/syslog");
    expect(byKey.get("type")).toBe("sftpDetails.file");
    expect(byKey.get("size")).toBe("1.5 KiB");
    expect(byKey.get("modified")).toBe(
      new Intl.DateTimeFormat("en-US", { dateStyle: "medium", timeStyle: "short" }).format(
        base.modifiedAt,
      ),
    );
    expect(byKey.get("permissions")).toBe("-rw-r--r--");
    expect(byKey.get("owner")).toBe("root");
    expect(byKey.get("group")).toBe("adm");
  });

  it("treats second-based mtime from the sidecar as seconds, not milliseconds", () => {
    const seconds = Math.floor(Date.UTC(2024, 0, 2, 3, 4) / 1000);
    const rows = sftpDetailRows({ path: "/x", kind: "file", modifiedAt: seconds }, t, "en-US");
    const modified = rows.find((row) => row.key === "modified")?.value;
    expect(modified).toBe(
      new Intl.DateTimeFormat("en-US", { dateStyle: "medium", timeStyle: "short" }).format(seconds * 1000),
    );
    expect(modified).not.toContain("1970");
  });

  it("maps every kind to its localized label", () => {
    const kinds: SftpEntryKind[] = ["file", "directory", "symlink", "other"];
    for (const kind of kinds) {
      const rows = sftpDetailRows({ path: "/x", kind }, t, "en-US");
      expect(rows.find((row) => row.key === "type")?.value).toBe(`sftpDetails.${kind}`);
    }
  });

  it("uses an em dash for missing size, modified time, permissions, owner and group", () => {
    const rows = sftpDetailRows({ path: "/var", kind: "directory" }, t, "en-US");
    const byKey = new Map(rows.map((row) => [row.key, row.value]));

    expect(byKey.get("size")).toBe("—");
    expect(byKey.get("modified")).toBe("—");
    expect(byKey.get("permissions")).toBe("—");
    expect(byKey.get("owner")).toBe("—");
    expect(byKey.get("group")).toBe("—");
  });

  it("formats a zero byte file as 0 B", () => {
    const rows = sftpDetailRows({ path: "/empty", kind: "file", size: 0 }, t, "en-US");
    expect(rows.find((row) => row.key === "size")?.value).toBe("0 B");
  });

  it("falls back to ownerName/groupName when owner/group are absent", () => {
    const rows = sftpDetailRows(
      { path: "/x", kind: "file", ownerName: "alice", groupName: "developers" },
      t,
      "en-US",
    );
    const byKey = new Map(rows.map((row) => [row.key, row.value]));

    expect(byKey.get("owner")).toBe("alice");
    expect(byKey.get("group")).toBe("developers");
  });

  it("uses the default locale when locale is empty", () => {
    const rows = sftpDetailRows({ ...base, modifiedAt: 0 }, t, "");
    expect(rows.find((row) => row.key === "modified")?.value).toBe(
      new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(0),
    );
  });
});
