import { describe, expect, it } from "vitest";
import { containerMatchesQuery, dockerListSignature, searchContainers, type DockerContainer } from "./dockerPanelModel";

function container(patch: Partial<DockerContainer> = {}): DockerContainer {
  return {
    id: "d4a7c9f1e2b3a1c0d4e5f6",
    name: "web-nginx",
    image: "nginx:1.27",
    state: "running",
    status: "Up 3 days",
    ports: "0.0.0.0:8080->80/tcp",
    createdAt: "2026-09-25 08:15:04 +0000 UTC",
    ...patch,
  };
}

describe("containerMatchesQuery", () => {
  it("matches name, image, id, state, status and ports case-insensitively", () => {
    const row = container();
    expect(containerMatchesQuery(row, "WEB")).toBe(true);
    expect(containerMatchesQuery(row, "redis")).toBe(false);
    expect(containerMatchesQuery(row, "nginx:1.27")).toBe(true);
    expect(containerMatchesQuery(row, "d4a7c9")).toBe(true);
    expect(containerMatchesQuery(row, "RUNNING")).toBe(true);
    expect(containerMatchesQuery(row, "8080")).toBe(true);
  });

  it("supports subsequence fuzz on identity fields", () => {
    const row = container();
    expect(containerMatchesQuery(row, "wng")).toBe(true);
    expect(containerMatchesQuery(row, "wz")).toBe(false);
    expect(containerMatchesQuery(container({ name: "trans-hub_ymt_psbc.1.1te84" }), "ymtpsbc")).toBe(true);
    // 端口/状态只做子串，避免子序列误命中。
    expect(containerMatchesQuery(container({ ports: "0.0.0.0:8080->80/tcp" }), "8080")).toBe(true);
    expect(containerMatchesQuery(container({ ports: "0.0.0.0:8080->80/tcp" }), "8088")).toBe(false);
  });

  it("requires every whitespace-separated term (AND)", () => {
    const row = container();
    expect(containerMatchesQuery(row, "web 8080")).toBe(true);
    expect(containerMatchesQuery(row, "web 6379")).toBe(false);
    expect(containerMatchesQuery(row, "   ")).toBe(true);
  });
});

describe("searchContainers", () => {
  const rows = [
    container(),
    container({ id: "9f8e7d6c5b4a", name: "cache-redis", image: "redis:7-alpine", ports: "127.0.0.1:6379->6379/tcp" }),
    container({ id: "1a2b3c4d5e6f", name: "db-postgres", image: "postgres:16", state: "exited", status: "Exited (0) 2 hours ago", ports: "" }),
  ];

  it("filters rows and keeps order", () => {
    expect(searchContainers(rows, "redis").map((row) => row.name)).toEqual(["cache-redis"]);
    expect(searchContainers(rows, "e").map((row) => row.name)).toEqual(["web-nginx", "cache-redis", "db-postgres"]);
  });

  it("returns the same list for an empty query", () => {
    expect(searchContainers(rows, "   ")).toBe(rows);
  });

  it("returns nothing when no row matches", () => {
    expect(searchContainers(rows, "no-such-container")).toEqual([]);
  });
});

describe("dockerListSignature", () => {
  const rows = [container()];

  it("is stable for identical payloads and changes with identity fields", () => {
    const base = dockerListSignature(true, false, rows);
    expect(dockerListSignature(true, false, rows.map((row) => ({ ...row })))).toBe(base);
    expect(dockerListSignature(true, false, [container({ status: "Up 4 days" })])).not.toBe(base);
    expect(dockerListSignature(false, true, rows)).not.toBe(base);
  });
});
