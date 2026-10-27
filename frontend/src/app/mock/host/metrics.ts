/**
 * 指标域夹具：ssh/metrics（CPU/内存/磁盘/网络/进程 + GPU/NPU 视觉夹具）、
 * ssh/processes/list|kill、sftp/diskUsage。刷新计数与 local/metrics 共用，
 * 让正弦扰动的 sparkline 在两类面板里都持续滚动。
 */

import { UNHANDLED, type Unhandled } from "./shared";
import { fixtureParams } from "./fixture";

let metricsRefreshTick = 0;

/** 每次指标请求自增，供 CPU/网络速率的确定性正弦扰动。 */
export function nextMetricsRefreshTick(): number {
  metricsRefreshTick += 1;
  return metricsRefreshTick;
}

export async function handleMetricsInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/metrics") {
    // ?err=metrics 模拟采集失败：dock 面板信息带走降级占位（CPU/MEM –%、
    // tooltip 携带原因），验证「失败可见」分支而非静默消失。
    if (fixtureParams.get("err") === "metrics") throw new Error("metrics collection failed: exec channel unavailable");
    const totalBytes = 16_573_006_848;
    // used ≈ 10.4 GiB includes the 5.5 GiB page cache, so the donut reads
    // used 4.9 GiB / cache 5.5 GiB / free 5.0 GiB like the reference dashboard.
    const availableBytes = 5_368_709_120;
    const cacheBytes = 5_905_756_160;
    // 网络速率随刷新次数变化（正弦扰动），让 sparkline 曲线肉眼可见地滚动；
    // processes/osId 镜像 §1.5 与既有扩展字段，供 B2 视觉验证。
    const tick = nextMetricsRefreshTick();
    const wave = (base: number, amplitude: number) => Math.max(0, Math.round(base + amplitude * Math.sin(tick / 2)));
    return {
      hostname: "web-01.demo.internal",
      kernel: "6.1.0-18-amd64",
      uptimeSeconds: 1_234_567,
      osId: "ubuntu",
      osPretty: "Ubuntu 22.04.5 LTS",
      cpu: {
        cores: 8,
        percent: 23.4,
        load1: 0.42,
        load5: 0.51,
        load15: 0.48,
        perCore: [
          { index: 0, percent: 6.2 },
          { index: 1, percent: 5.4 },
          { index: 2, percent: 6.4 },
          { index: 3, percent: 4.3 },
          { index: 4, percent: 13.4 },
          { index: 5, percent: 4.9 },
          { index: 6, percent: 10.0 },
          { index: 7, percent: 5.3 },
        ],
      },
      memory: {
        totalBytes,
        availableBytes,
        usedBytes: totalBytes - availableBytes,
        cacheBytes,
        freeBytes: availableBytes,
        swapTotalBytes: 2_147_483_648,
        swapUsedBytes: 0,
      },
      disks: [
        { filesystem: "/dev/sda1", mount: "/", totalBytes: 52_723_200_512, usedBytes: 24_023_981_056, availableBytes: 26_005_927_936, percentUsed: 48, inodeUsePercent: 24 },
        // /data 固定给 87%（>=85 警戒阈值），让 disk-warn 红色进度条始终可被视觉验证。
        { filesystem: "/dev/sdb1", mount: "/data", totalBytes: 105_550_471_168, usedBytes: 91_828_909_916, availableBytes: 13_721_561_252, percentUsed: 87, inodeUsePercent: 55 },
        { filesystem: "tmpfs", mount: "/dev/shm", totalBytes: 8_146_615_296, usedBytes: 0, availableBytes: 8_146_615_296, percentUsed: 0, inodeUsePercent: 0 },
        // 超长挂载点夹具：锁定 disk-row 第一列的省略号截断（回归：长路径曾
        // 外溢压住进度条与数值列，metrics 面板错位严重）。
        { filesystem: "/dev/mapper/vg--main-very--long--logical--volume", mount: "/mnt/data/services/postgres/bind-mounts/very/long/path", totalBytes: 211_100_942_336, usedBytes: 12_000_000_000, availableBytes: 199_100_942_336, percentUsed: 6, inodeUsePercent: 3 },
      ],
      network: [
        { name: "eth0", rxRate: wave(48_000, 40_000), txRate: wave(12_000, 9_000), rxTotal: 123_456_789_012, txTotal: 9_876_543_210 },
        { name: "lo", rxRate: wave(1_200, 800), txRate: wave(1_200, 800), rxTotal: 5_555_555, txTotal: 5_555_555 },
        // 超长网卡名夹具：同上，锁定 disk-row 截断。
        { name: "br-0f17c3a9d2e4-docker-custom-network", rxRate: wave(2_400, 1_800), txRate: wave(1_800, 1_200), rxTotal: 12_345_678, txTotal: 8_765_432 },
      ],
      processes: [
        { pid: 2457, user: "app", cpuPercent: 13.4, memPercent: 4.4, command: "/usr/lib/jvm/java-17-openjdk/bin/java -jar app.jar" },
        { pid: 3917, user: "mongodb", cpuPercent: 10.0, memPercent: 8.3, command: "/usr/bin/mongod --config /etc/mongod.conf" },
        { pid: 812, user: "www-data", cpuPercent: 6.4, memPercent: 3.1, command: "nginx: worker process" },
        { pid: 1317, user: "root", cpuPercent: 1.3, memPercent: 2.5, command: "/usr/bin/dockerd -H fd://" },
        { pid: 2418, user: "redis", cpuPercent: 0.8, memPercent: 2.5, command: "redis-server *:6379" },
        { pid: 1, user: "root", cpuPercent: 0.1, memPercent: 0.4, command: "/sbin/init splash" },
      ],
      // GPU / Ascend NPU 视觉夹具（IMPL_PLAN Task P1-4）：字段与 metrics_gpu.rs
      // 的 JSON 输出一致，供监控卡片渲染、警戒色与空进程态可被视觉验证。
      gpu: {
        available: true,
        gpus: [
          {
            index: 0, uuid: "GPU-7f3a2c11-9b04-e8d2-aa51-6c0f5b19d301", name: "NVIDIA A100-SXM4-40GB",
            driver: "550.54.15", temperature: 67, utilization: 93, memUtil: 81,
            totalMem: 42_949_672_960, usedMem: 35_433_601_024, freeMem: 7_516_071_936,
            powerDraw: 298, powerLimit: 400, fan: 62, pstate: "P0",
            processes: [
              { uuid: "GPU-7f3a2c11-9b04-e8d2-aa51-6c0f5b19d301", pid: 2211, mem: 31_221_622_784, name: "python train_llm.py" },
              { uuid: "GPU-7f3a2c11-9b04-e8d2-aa51-6c0f5b19d301", pid: 2212, mem: 3_921_678_336, name: "python eval.py" },
            ],
          },
          {
            index: 1, uuid: "GPU-7f3a2c11-9b04-e8d2-aa51-6c0f5b19d302", name: "NVIDIA A100-SXM4-40GB",
            driver: "550.54.15", temperature: 41, utilization: 0, memUtil: 0,
            totalMem: 42_949_672_960, usedMem: 1_073_741_824, freeMem: 41_875_931_136,
            powerDraw: 68, powerLimit: 400, fan: 22, pstate: "P8",
            processes: [],
          },
        ],
      },
      npu: {
        available: true,
        cann: "8.0.RC3.beta1",
        devices: [
          {
            deviceKey: "ascend:0:0", npuIndex: 0, chipId: 0, name: "910B4", health: "OK",
            power: 212.4, temperature: 58, aicore: 76, memoryLabel: "hbm",
            usedMem: 22_158_430_208, totalMem: 31_675_758_592,
            processes: [{ pid: 3310, name: "mindspore-train", npuIndex: 0, mem: 21_474_836_480 }],
          },
          {
            deviceKey: "ascend:1:0", npuIndex: 1, chipId: 0, name: "310P3", health: "OK",
            power: 45.1, temperature: 39, aicore: 0, memoryLabel: "memory",
            usedMem: 1_073_741_824, totalMem: 16_856_718_131,
            processes: [],
          },
        ],
      },
    };
  }
  if (method === "ssh/processes/list") {
    // 进程管理表 mock（M13-B）：与 ssh/metrics 的 top-8 行同源，补齐管理表
    // 全字段，并镜像后端 best-effort 的 fdCount / listenPorts 两列——
    // pid 812 展示双端口（可视觉验证排序/去重），pid 1042 两列留空展示占位符。
    return {
      processes: [
        { pid: 1, ppid: 0, user: "root", cpuPercent: 0.1, memPercent: 0.4, etime: "30-04:12:33", state: "S", command: "/sbin/init splash", fdCount: 148, listenPorts: [] },
        { pid: 812, ppid: 811, user: "www-data", cpuPercent: 12.6, memPercent: 3.1, etime: "12-21:05:09", state: "S", command: "nginx: worker process", fdCount: 64, listenPorts: [80, 443] },
        { pid: 1042, ppid: 1040, user: "demo", cpuPercent: 2.4, memPercent: 1.2, etime: "01:23", state: "R", command: "htop", fdCount: null, listenPorts: [] },
        { pid: 2211, ppid: 2209, user: "demo", cpuPercent: 1.8, memPercent: 8.6, etime: "3-02:44:51", state: "S", command: "python train_llm.py --epochs 8", fdCount: 32, listenPorts: [6006, 29500, 29501, 29502, 29503] },
      ],
    };
  }
  if (method === "ssh/processes/kill") {
    // 只记录信号目标，模拟成功路径（不真正改表，刷新仍返回同一份 mock）。
    return {};
  }
  if (method === "sftp/diskUsage") {
    return { filesystem: "/dev/sda1", mount: "/", totalBytes: 52_723_200_512, usedBytes: 24_023_981_056, availableBytes: 26_005_927_936, percentUsed: 48 };
  }
  return UNHANDLED;
}
