// 指标面板视图层去噪（纯函数，不连 SSH）。
//
// 真机 df/ip 输出里充满伪文件系统（/dev、/sys/fs/cgroup）、docker overlay
// 重复行（同一设备统计重复 N 次）和回环/容器网卡（lo、veth、docker0、
// br-*）——原样渲染会把真实挂载点和物理网卡淹没。这里做视图层过滤/去重/
// 排序，不改后端采集语义。

export interface DiskMountView {
  mount: string;
  filesystem?: string;
  totalBytes?: number;
  usedBytes?: number;
  percentUsed?: number;
}

export interface NetworkInterfaceView {
  name: string;
  rxRate?: number;
  txRate?: number;
}

// 伪文件系统/内存盘：没有独立的块设备容量语义，或只是内核视图。
const PSEUDO_FILESYSTEM =
  /^(tmpfs|devtmpfs|devfs|ramfs|rootfs|squashfs|overlay|aufs|proc|sysfs|cgroup2?|securityfs|pstore|debugfs|tracefs|configfs|fusectl|mqueue|hugetlbfs|nsfs|binfmt_misc|autofs|efivarfs|rpc_pipefs|nfsd|none|fuse\..*)$/i;

// 容器运行时/系统分区上的 bind mount：底层块设备是真实磁盘，但对使用者
// 没有参考价值；EFI 分区同理。macOS 的 /System/Volumes/Data 是真实数据卷，
// 刻意不在列表里。
const NOISE_MOUNT =
  /^\/(boot(?:\/|$)|var\/lib\/(docker|containers|kubelet|lxcfs|snapd)(?:\/|$)|System\/Volumes\/(?:Preboot|VM|Update|iSCPreboot|Recovery|Xcode|xarts)(?:\/|$))/;

// 回环、容器网络、虚拟交换机与链路本地隧道：主机负载视角下恒为噪声。
const NOISE_NETWORK =
  /^(lo\d*|veth|docker|br-|virbr|vnet|cni|flannel|cali|kube|podman|lxc|lxd|tap\d*|tun\d*|gif\d*|stf\d*|awdl\d*|llw\d*|anpi\d*|bridge\d*|p2p\d*|vmenet\d*)/;

export interface MetricsNoiseOptions {
  /** true = 连伪文件系统/虚拟网卡一起展示（「显示全部」开关）。 */
  includeNoise?: boolean;
}

function isNoiseDisk(disk: DiskMountView): boolean {
  // 根文件系统永远保留（容器里本身可能就是 overlay/tmpfs）。
  if (disk.mount === "/") return false;
  const filesystem = (disk.filesystem ?? "").trim();
  if (filesystem && PSEUDO_FILESYSTEM.test(filesystem)) return true;
  return NOISE_MOUNT.test(disk.mount);
}

/**
 * 磁盘行去噪：丢弃零容量行、伪文件系统/容器 bind mount，相同容量指纹的重复行
 * 只留挂载点最短的一条，按占用率降序。`includeNoise` 为「显示全部」开关：
 * 仍丢零容量行与重复指纹，但不隐藏伪文件系统/系统分区。
 */
export function filterDiskMounts<T extends DiskMountView>(disks: T[] | null | undefined, options: MetricsNoiseOptions = {}): T[] {
  if (!Array.isArray(disks)) return [];
  const seen = new Map<string, T>();
  for (const disk of disks) {
    if (!disk || typeof disk.mount !== "string" || !disk.mount) continue;
    if (!(disk.totalBytes && disk.totalBytes > 0)) continue;
    if (!options.includeNoise && isNoiseDisk(disk)) continue;
    // 容量指纹：同一设备被多次挂载（docker overlay2、bind mount）时
    // total/used 完全相同，留挂载点最短的那条（通常是真实挂载点）。
    const fingerprint = `${disk.totalBytes}:${disk.usedBytes ?? 0}`;
    const existing = seen.get(fingerprint);
    if (!existing || disk.mount.length < existing.mount.length) {
      seen.set(fingerprint, disk);
    }
  }
  return [...seen.values()].sort(
    (a, b) => (b.percentUsed ?? 0) - (a.percentUsed ?? 0),
  );
}

/**
 * 网卡行去噪：回环/容器/虚拟交换网卡无论有无流量都隐藏；全部隐藏时返回原
 * 列表（避免空区的困惑，例如只能看到 lo 的受限环境）。`includeNoise` 为
 * 「显示全部」开关：原样返回。
 */
export function filterNetworkInterfaces<T extends NetworkInterfaceView>(
  interfaces: T[] | null | undefined,
  options: MetricsNoiseOptions = {},
): T[] {
  if (!Array.isArray(interfaces)) return [];
  if (options.includeNoise) return interfaces;
  const visible = interfaces.filter((net) => net && typeof net.name === "string" && !NOISE_NETWORK.test(net.name));
  return visible.length > 0 ? visible : interfaces;
}
