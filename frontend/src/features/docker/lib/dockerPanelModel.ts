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
export const TAIL_OPTIONS = ["100", "200", "500", "1000", "2000"] as const;

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
