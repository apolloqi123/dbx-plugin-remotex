/** 内存 fixture 树：路径感知的 sftp/list 与写操作（无条件生效，无开关参数）。
 *  可视化夹具专用；真实宿主语义由 sidecar 负责。 */
export interface MockNode {
  name: string;
  kind: "directory" | "file";
  size: number;
  modifiedAt: number;
  permissions: string;
  children?: MockNode[];
}

let mockStamp = 1786262400;
function nextMockStamp() {
  mockStamp += 3600;
  return mockStamp;
}

function mockDir(name: string, children: MockNode[] = []): MockNode {
  return { name, kind: "directory", size: 0, modifiedAt: nextMockStamp(), permissions: "0755", children };
}

function mockFile(name: string, size: number, permissions = "0644"): MockNode {
  return { name, kind: "file", size, modifiedAt: nextMockStamp(), permissions };
}

const mockTree: MockNode = mockDir("/", [
  mockDir("home", [
    mockDir("demo", [
      mockDir(".config"),
      mockDir("projects"),
      // 大目录夹具(issue #151):2000 个文件锁定虚拟滚动的窗口化行为——
      // 滚动到底可达、行高与 30px 常数一致、spacer 总高与真实内容一致。
      mockDir(
        "bigdir",
        Array.from({ length: 2000 }, (_, index) => mockFile(`file-${String(index + 1).padStart(4, "0")}.txt`, index * 137 + 11)),
      ),
      mockFile("deploy.sh", 2481, "0755"),
      mockFile("docker-compose.yml", 8192),
      mockFile("server.log", 741248),
      // 超长名称夹具：锁定 SFTP 文件列表/侧栏/预览标题在极端宽度下的省略号
      // 截断（回归：长名曾把行布局挤错位）。
      mockDir("a-very-long-directory-name-that-easily-overflows-narrow-side-panels"),
      mockFile("release-artifact-bundle-2026-09-15-final-signed-verification-report.pdf", 999_999),
    ]),
  ]),
  mockDir("etc", [mockFile("hosts", 221), mockDir("nginx", [mockFile("nginx.conf", 1264)])]),
  mockDir("tmp"),
  mockDir("var", [mockDir("log", [mockFile("syslog", 15432)])]),
  mockDir("root"),
]);

function normalizeMockPath(path: string): string {
  let value = (path || "/").trim() || "/";
  if (!value.startsWith("/")) value = `/${value}`;
  value = value.replace(/\/{2,}/g, "/");
  return value === "/" ? value : value.replace(/\/+$/, "");
}

function findMockNode(path: string): MockNode | null {
  const normalized = normalizeMockPath(path);
  if (normalized === "/") return mockTree;
  let node: MockNode = mockTree;
  for (const segment of normalized.slice(1).split("/")) {
    const next = node.children?.find((child) => child.name === segment);
    if (!next) return null;
    node = next;
  }
  return node;
}

function mockParentAndName(path: string): { parent: MockNode | null; name: string } {
  const normalized = normalizeMockPath(path);
  const index = normalized.lastIndexOf("/");
  const parentPath = index <= 0 ? "/" : normalized.slice(0, index);
  return { parent: findMockNode(parentPath), name: normalized.slice(index + 1) };
}

function mockEntryOf(node: MockNode, parentPath: string) {
  return {
    name: node.name,
    uri: `sftp:${parentPath === "/" ? "" : parentPath}/${node.name}`,
    kind: node.kind,
    ...(node.kind === "file" ? { size: node.size } : {}),
    modifiedAt: node.modifiedAt,
    permissions: node.permissions,
  };
}

function mockList(path: string) {
  const parentPath = normalizeMockPath(path);
  const node = findMockNode(parentPath);
  if (!node || node.kind !== "directory") throw new Error(`sftp: no such directory: ${parentPath}`);
  return (node.children ?? [])
    .map((child) => mockEntryOf(child, parentPath))
    .sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === "directory" ? -1 : 1));
}

function mockWriteEntry(path: string, node: MockNode): { success: true } {
  const { parent, name } = mockParentAndName(path);
  if (!parent || parent.kind !== "directory" || !name || parent.children?.some((child) => child.name === name)) {
    throw new Error(`sftp: cannot write ${normalizeMockPath(path)}`);
  }
  parent.children!.push(node);
  return { success: true };
}

// 编辑器 sftp/write 落盘的内存副本（按路径）：sftp/read 优先回读，保证
// 可视化夹具里「保存 → 再打开」闭环。
const fileContents = new Map<string, Uint8Array>();

// 日志着色夹具：server.log / syslog 放真实形态的日志（时间戳、级别、键值、
// JSON 行、堆栈帧），供编辑器日志高亮与预览走查使用。
const DEMO_APP_LOG = [
  "2026-10-29T09:41:02.118Z INFO  server booting pid=4821 env=production",
  "2026-10-29T09:41:02.204Z INFO  listening on 0.0.0.0:8080",
  '2026-10-29T09:41:03.551Z WARN  upstream=10.0.0.8 latency=612ms "slow response"',
  "2026-10-29T09:41:04.002Z INFO  GET /health 200 in 2ms",
  "2026-10-29T09:41:05.771Z ERROR upstream connect timeout after 3 tries url=https://api.example.com/health",
  "2026-10-29T09:41:05.772Z DEBUG retry 1/3 in 250ms",
  '{"level":"info","msg":"request done","status":200,"ok":true}',
  "Traceback (most recent call last):",
  '  File "/srv/app/main.py", line 42, in handler',
  "    return upstream.fetch()",
  "TimeoutError: upstream did not respond in 5s",
  "2026-10-29T09:41:09.001Z INFO  graceful shutdown signal received",
].join("\n");
const DEMO_SYSLOG = [
  "Oct 29 09:41:02 web-1 sshd[4821]: Accepted publickey for deploy from 10.0.0.8 port 51234 ssh2",
  "Oct 29 09:41:06 web-1 systemd[1]: Started nginx.service - A high performance web server.",
  "Oct 29 09:41:11 web-1 kernel: [ 1234.567890] eth0: link up, 1000 Mbps, full duplex",
  "Oct 29 09:41:14 web-1 cron[9012]: (root) CMD (/usr/local/bin/backup.sh)",
].join("\n");

fileContents.set("/home/demo/server.log", new TextEncoder().encode(`${DEMO_APP_LOG}\n`));
fileContents.set("/var/log/syslog", new TextEncoder().encode(`${DEMO_SYSLOG}\n`));

// 树节点 size 与种子内容对齐（列表里显示的大小 = 双击预览拿到的字节数）。
for (const path of ["/home/demo/server.log", "/var/log/syslog"]) {
  const node = findMockNode(path);
  const bytes = fileContents.get(path);
  if (node && bytes) node.size = bytes.byteLength;
}

export const mockFileSystem = {
  dir: mockDir,
  file: mockFile,
  list: mockList,
  writeEntry: mockWriteEntry,
  findNode: findMockNode,
  normalizePath: normalizeMockPath,
  parentAndName: mockParentAndName,
  writeContents(path: string, bytes: Uint8Array) {
    fileContents.set(path, bytes);
  },
  readContents(path: string): Uint8Array | undefined {
    return fileContents.get(path);
  },
};
