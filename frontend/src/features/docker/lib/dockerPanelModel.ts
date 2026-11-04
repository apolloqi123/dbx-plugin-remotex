// Docker 面板纯逻辑：RPC 载荷归一、状态徽标 class、「在终端打开」命令拼装、
// 动作文案 key 与超时/轮询常量。与 dockerActions/dockerRowMenu 同风格：无 Vue
// 依赖，面板/composable 共用，假数据即可单测。
import type { DockerActionName, DockerActionTarget } from "./dockerActions";

export interface DockerContainer {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  ports: string;
  createdAt: string;
}

export interface DockerInspectSummary {
  id: string;
  name: string;
  image: string;
  status: string;
  running: boolean;
  startedAt: string;
  health: string | null;
  restartPolicy: string;
}

/** kill/rm 等破坏性动作的待确认目标（action 随目标一起带出）。 */
export type DockerConfirmTarget = DockerActionTarget & { action: DockerActionName };

// 动作动词 → 静态 i18n key（i18nKeyReferences.spec 只认字面量 key）。
export const ACTION_LABEL_KEYS: Record<DockerActionName, string> = {
  start: "docker.actionStart",
  stop: "docker.actionStop",
  restart: "docker.actionRestart",
  kill: "docker.actionKill",
  rm: "docker.actionRemove",
};

export const POLL_MS = 10_000;
export const POLL_MAX_FAILURES = 3;
export const LIST_TIMEOUT_MS = 20_000;
export const LOGS_TIMEOUT_MS = 35_000;
export const ACTION_TIMEOUT_MS = 70_000;
export const TAIL_OPTIONS = ["100", "200", "500", "1000"] as const;

export function dockerListPayload(raw: unknown): { available: boolean; needsSudo: boolean; containers: DockerContainer[] } {
  const record = (raw ?? {}) as Record<string, unknown>;
  const rows = Array.isArray(record.containers) ? record.containers : [];
  const containers: DockerContainer[] = [];
  for (const row of rows) {
    if (!row || typeof row !== "object") continue;
    const entry = row as Record<string, unknown>;
    const id = typeof entry.id === "string" ? entry.id : "";
    if (!id) continue;
    containers.push({
      id,
      name: typeof entry.name === "string" ? entry.name : "",
      image: typeof entry.image === "string" ? entry.image : "",
      state: typeof entry.state === "string" ? entry.state : "",
      status: typeof entry.status === "string" ? entry.status : "",
      ports: typeof entry.ports === "string" ? entry.ports : "",
      createdAt: typeof entry.createdAt === "string" ? entry.createdAt : "",
    });
  }
  return {
    available: record.available === true,
    needsSudo: record.needsSudo === true,
    containers,
  };
}

/** 「在终端打开」命令：先检测可用 shell，再 exec；非零退出不触发其他 shell。
 *  CLI 跟随引擎设置（podman 等自定义命令同样可进终端 exec）。 */
export function dockerExecCommand(cli: string, containerId: string): string {
  return `${cli || "docker"} exec -it ${containerId} sh -lc 'for shell in bash zsh fish ash; do if command -v "$shell" >/dev/null 2>&1; then exec "$shell"; fi; done; exec sh'`;
}

/** 状态徽标 class：已知状态着色，未知状态走中性灰。 */
export function dockerStateClass(state: string): string {
  const known = ["running", "exited", "paused", "created", "restarting", "dead", "removing"];
  return known.includes(state) ? `docker-state-${state}` : "docker-state-unknown";
}

export function isDockerContainerRunning(container: Pick<DockerContainer, "state">): boolean {
  return container.state === "running";
}

/**
 * Parse "stopped X ago" from Docker status text and return minutes since stop.
 * Returns null if the status doesn't indicate a stopped container or the time
 * can't be parsed.
 *
 * Examples:
 *   "Exited (0) 2 minutes ago" → 2
 *   "Exited (137) 5 hours ago" → 300
 *   "Up 3 hours" → null (still running)
 */
export function parseStoppedMinutesAgo(status: string): number | null {
  const match = status.match(/Exited\s+\([^)]+\)\s+(\d+)\s+(second|minute|hour|day)s?\s+ago/i);
  if (!match) return null;
  const value = parseInt(match[1], 10);
  const unit = match[2].toLowerCase();
  if (unit === "second") return value / 60;
  if (unit === "minute") return value;
  if (unit === "hour") return value * 60;
  if (unit === "day") return value * 24 * 60;
  return null;
}

/** Filter containers by state. "recent" = running + stopped within 5 minutes. */
export type ContainerFilter = "all" | "running" | "recent" | "stopped";

export function filterContainers(containers: DockerContainer[], filter: ContainerFilter): DockerContainer[] {
  if (filter === "all") return containers;
  if (filter === "running") return containers.filter((c) => c.state === "running");
  if (filter === "stopped") return containers.filter((c) => c.state !== "running");
  // "recent": running + stopped within 5 minutes
  return containers.filter((c) => {
    if (c.state === "running") return true;
    const minutes = parseStoppedMinutesAgo(c.status);
    return minutes !== null && minutes <= 5;
  });
}

/** 字符子序列匹配（模糊搜索兜底）：`wng` 命中 `web-nginx`。 */
function isSubsequence(haystack: string, needle: string): boolean {
  let cursor = 0;
  for (const char of needle) {
    cursor = haystack.indexOf(char, cursor);
    if (cursor === -1) return false;
    cursor += 1;
  }
  return true;
}

/** 名字/镜像/ID 三个标识字段允许子序列模糊；状态/端口/状态文案只做子串。 */
const FUZZY_FIELDS: ReadonlyArray<keyof DockerContainer> = ["name", "id", "image"];
const SUBSTRING_FIELDS: ReadonlyArray<keyof DockerContainer> = ["name", "id", "image", "state", "status", "ports"];

/**
 * 容器搜索：空白分隔的多个词按 AND 组合（`web 8080` 同时命中名字与端口），
 * 大小写不敏感；标识字段先子串后字符子序列（`wng` → `web-nginx`）。
 * 空查询视为全部命中。
 */
export function containerMatchesQuery(container: DockerContainer, query: string): boolean {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return true;
  const substringFields = SUBSTRING_FIELDS.map((key) => container[key].toLowerCase());
  const fuzzyFields = FUZZY_FIELDS.map((key) => container[key].toLowerCase());
  return terms.every(
    (term) =>
      substringFields.some((field) => field.includes(term)) ||
      fuzzyFields.some((field) => isSubsequence(field, term)),
  );
}

/**
 * 列表载荷签名（可用性 + 每行身份字段）。10s 轮询结果没变时复用旧数组，
 * 避免整块列表/详情每轮无谓重渲染（容器多时尤其明显）。
 */
export function dockerListSignature(
  available: boolean,
  needsSudo: boolean,
  containers: DockerContainer[],
): string {
  const rows = containers.map((container) =>
    [container.id, container.name, container.image, container.state, container.status, container.ports, container.createdAt].join("\u0000"),
  );
  return `${available ? 1 : 0}${needsSudo ? 1 : 0}${rows.join("\u0001")}`;
}

/** 在已按状态筛出的列表上叠加搜索；空查询原样返回（同一引用）。 */
export function searchContainers(containers: DockerContainer[], query: string): DockerContainer[] {
  if (!query.trim()) return containers;
  return containers.filter((container) => containerMatchesQuery(container, query));
}
