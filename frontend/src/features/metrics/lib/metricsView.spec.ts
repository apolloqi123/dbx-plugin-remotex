import { describe, expect, it } from "vitest";
import { filterDiskMounts, filterNetworkInterfaces } from "./metricsView";

describe("filterDiskMounts", () => {
  it("drops zero-capacity rows (pseudo filesystems / parse ghosts)", () => {
    const rows = filterDiskMounts([
      { mount: "/", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "(OpenAnolis", totalBytes: 0, usedBytes: 0, percentUsed: 0 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual(["/"]);
  });

  it("hides pseudo filesystems and container bind mounts", () => {
    const rows = filterDiskMounts([
      { mount: "/", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/dev/shm", filesystem: "tmpfs", totalBytes: 8_000, usedBytes: 0, percentUsed: 0 },
      { mount: "/run", filesystem: "tmpfs", totalBytes: 4_000, usedBytes: 200, percentUsed: 5 },
      { mount: "/sys/fs/cgroup", filesystem: "cgroup2", totalBytes: 40_000, usedBytes: 0, percentUsed: 0 },
      { mount: "/snap/core/123", filesystem: "squashfs", totalBytes: 60_000, usedBytes: 60_000, percentUsed: 100 },
      { mount: "/boot", filesystem: "/dev/sda2", totalBytes: 508_000, usedBytes: 147_000, percentUsed: 29 },
      { mount: "/boot/efi", filesystem: "/dev/sda3", totalBytes: 200, usedBytes: 6, percentUsed: 3 },
      { mount: "/var/lib/docker/overlay2/ab3f", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/var/lib/kubelet/pods/x", filesystem: "/dev/sdb1", totalBytes: 90_000, usedBytes: 9_000, percentUsed: 10 },
      { mount: "/data", filesystem: "/dev/sdb1", totalBytes: 100_000, usedBytes: 85_000, percentUsed: 87 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual(["/data", "/"]);
  });

  it("includeNoise shows pseudo filesystems and system partitions again", () => {
    const rows = filterDiskMounts(
      [
        { mount: "/", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
        { mount: "/boot", filesystem: "/dev/sda2", totalBytes: 508_000, usedBytes: 147_000, percentUsed: 29 },
        { mount: "/dev/shm", filesystem: "tmpfs", totalBytes: 8_000, usedBytes: 0, percentUsed: 0 },
      ],
      { includeNoise: true },
    );
    expect(rows.map((r) => r.mount).sort()).toEqual(["/", "/boot", "/dev/shm"]);
  });

  it("keeps the root filesystem even when it is an overlay (containers)", () => {
    const rows = filterDiskMounts([
      { mount: "/", filesystem: "overlay", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual(["/"]);
  });

  it("keeps real network/filesystem mounts and the macOS data volume", () => {
    const rows = filterDiskMounts([
      { mount: "/System/Volumes/Data", filesystem: "/dev/disk3s5", totalBytes: 80_000, usedBytes: 40_000, percentUsed: 50 },
      { mount: "/System/Volumes/Preboot", filesystem: "/dev/disk3s2", totalBytes: 5_000, usedBytes: 100, percentUsed: 2 },
      { mount: "/mnt/nas", filesystem: "192.168.1.7:/export", totalBytes: 900_000, usedBytes: 300_000, percentUsed: 33 },
      { mount: "/media/usb", filesystem: "//router/share", totalBytes: 60_000, usedBytes: 10_000, percentUsed: 17 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual([
      "/System/Volumes/Data",
      "/mnt/nas",
      "/media/usb",
    ]);
  });

  it("dedupes identical capacity fingerprints, keeping the shortest mount", () => {
    const rows = filterDiskMounts([
      { mount: "/srv/app/bind", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/opt/app", filesystem: "/dev/sda1", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual(["/"]);
  });

  it("sorts by usage descending", () => {
    const rows = filterDiskMounts([
      { mount: "/", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/data", totalBytes: 100_000, usedBytes: 85_000, percentUsed: 87 },
      { mount: "/home", totalBytes: 60_000, usedBytes: 6_000, percentUsed: 10 },
    ]);
    expect(rows.map((r) => r.mount)).toEqual(["/data", "/", "/home"]);
  });

  it("keeps distinct mounts on the same filesystem when usage differs", () => {
    const rows = filterDiskMounts([
      { mount: "/", totalBytes: 40_000, usedBytes: 17_000, percentUsed: 44 },
      { mount: "/home", totalBytes: 40_000, usedBytes: 9_000, percentUsed: 23 },
    ]);
    expect(rows).toHaveLength(2);
  });

  it("returns [] for null/garbage input", () => {
    expect(filterDiskMounts(null)).toEqual([]);
    expect(filterDiskMounts([null as never, {} as never])).toEqual([]);
  });
});

describe("filterNetworkInterfaces", () => {
  it("hides loopback and virtual interfaces even when they carry traffic", () => {
    const rows = filterNetworkInterfaces([
      { name: "eth0", rxRate: 10, txRate: 5 },
      { name: "lo", rxRate: 900, txRate: 900 },
      { name: "veth123", rxRate: 30, txRate: 20 },
      { name: "docker0", rxRate: 0, txRate: 0 },
      { name: "br-0f17c3a9d2e4", rxRate: 200, txRate: 150 },
      { name: "virbr0", rxRate: 0, txRate: 0 },
      { name: "tun0", rxRate: 40, txRate: 40 },
      { name: "awdl0", rxRate: 1, txRate: 1 },
    ]);
    expect(rows.map((r) => r.name)).toEqual(["eth0"]);
  });

  it("keeps physical, bonded and user-facing VPN interfaces", () => {
    const rows = filterNetworkInterfaces([
      { name: "en0" },
      { name: "ens5" },
      { name: "enp3s0" },
      { name: "bond0" },
      { name: "eth0.100" },
      { name: "wg0" },
      { name: "tailscale0" },
      { name: "utun3" },
    ]);
    expect(rows.map((r) => r.name)).toEqual(["en0", "ens5", "enp3s0", "bond0", "eth0.100", "wg0", "tailscale0", "utun3"]);
  });

  it("includeNoise shows loopback and virtual interfaces again", () => {
    const rows = filterNetworkInterfaces(
      [
        { name: "eth0" },
        { name: "lo" },
        { name: "docker0" },
      ],
      { includeNoise: true },
    );
    expect(rows.map((r) => r.name)).toEqual(["eth0", "lo", "docker0"]);
  });

  it("falls back to the full list when everything would be hidden", () => {
    const rows = filterNetworkInterfaces([
      { name: "lo", rxRate: 0, txRate: 0 },
      { name: "veth123", rxRate: 0, txRate: 0 },
    ]);
    expect(rows).toHaveLength(2);
  });

  it("returns [] for null input", () => {
    expect(filterNetworkInterfaces(undefined)).toEqual([]);
  });
});
