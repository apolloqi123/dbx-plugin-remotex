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
