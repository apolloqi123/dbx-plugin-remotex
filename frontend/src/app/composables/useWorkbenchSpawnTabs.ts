import { ref, type Ref } from "vue";
import { WORKBENCH_ID } from "../../shared/lib/pluginIds";
import { randomUUID } from "../../shared/lib/uuid";

/**
 * 宿主 openWorkbench 的「同工作台 spawn 新 tab」入口（新建/复制/命令会话）。
 * 只依赖当前连接上下文与宿主桥，不触碰本地会话与终端管线。
 */
export function useWorkbenchSpawnTabs(options: {
  hostContext: Ref<Record<string, unknown>>;
  connectionId: () => string;
  session: Ref<{ sessionId: string } | undefined>;
  connected: () => boolean;
}) {
  const { hostContext, connectionId, session, connected } = options;

  // 新建会话（同连接再开一个 tab）：context 复制当前一份并换发新 workbenchId——
  // 后端按 workbenchId 开独立 PTY（#41 会话隔离），多个 tab 互不干扰、boot 恢复
  // 各回各的会话。克隆的 workbenchState 摘掉 sessionId/terminalSequence，避免新
  // tab 尝试回附旧会话。forceNew 让宿主跳过"同 connectionId 复用已有 tab"的查重
  // （桥路径不传 connectionId，按钮开的 tab 会互相查重，导致只能多开一个）；
  // 旧宿主忽略第三参，退化为原查重行为。connectionId 显式写入 context：宿主的
  // 重推凭据（reinit re-push）与 hostContext 合并都键在 context.connectionId 上，
  // 不能依赖宿主已把它合进 context（旧宿主没有那层合并）。
  function sessionTabContext(reuseAuthenticatedTransport: boolean, spawnCommand?: string): Record<string, unknown> {
    const context: Record<string, unknown> = {
      ...hostContext.value,
      connectionId: connectionId(),
      workbenchId: randomUUID(),
      reuseAuthenticatedTransport,
      reuseAuthenticatedSessionId: reuseAuthenticatedTransport ? session.value?.sessionId : undefined,
      // WT-4（WezTerm spawn 对标）：命令会话的一次性打开参数。非宿主保留
      // 字段，随 context 原样透传给新工作台，由 sessionTransportReuse 状态
      // 消费；契约见 docs/architecture/PROTOCOL.zh-CN.md「同 transport 命令会话」。
      spawnCommand: spawnCommand?.trim() || undefined,
    };
    const persisted = context.workbenchState;
    if (persisted && typeof persisted === "object") {
      const { sessionId: _sessionId, terminalSequence: _terminalSequence, ...rest } = persisted as Record<string, unknown>;
      context.workbenchState = rest;
    }
    return context;
  }

  function openNewSessionTab() {
    const api = window.dbxPlugin;
    if (!api.openWorkbench || !connectionId()) return;
    // target: "tab"：面板表面（dock iframe）调用时显式指定主工作台 tab 落点；
    // tab 表面调用本就落 tab，旧宿主忽略该选项回落面板条目（可选降级）。
    void api.openWorkbench(WORKBENCH_ID, sessionTabContext(false), { forceNew: true, target: "tab" });
  }

  // 复制会话：新 tab 仍拥有独立 PTY、回放缓冲和 workbenchId，但后端在当前
  // 已认证 SSH transport 上另开 channel，因此堡垒机不会再次发起 MFA。这里不
  // 复制或缓存 OTP；若原 transport 已失效，后端会要求走“新建会话”重新连接。
  function openCopiedSessionTab() {
    const api = window.dbxPlugin;
    if (!api.openWorkbench || !connectionId() || !connected()) return;
    void api.openWorkbench(WORKBENCH_ID, sessionTabContext(true), { forceNew: true, target: "tab" });
  }

  // 命令会话（WT-4，WezTerm spawn 对标）：复制会话的同族入口，channel 启动
  // 动作从 shell 换成执行指定命令（独立 PTY，后端 shell 单引号转义）。弹窗
  // 收命令后 forceNew 开新 tab；共享 transport 引用、MaxSessions 失败可见等
  // 语义全部复用复制会话链路（PROTOCOL「同 transport 命令会话」节）。
  const spawnSessionDialogOpen = ref(false);
  const spawnSessionCommand = ref("");

  function openCommandSessionTab() {
    if (!window.dbxPlugin?.openWorkbench || !connectionId() || !connected()) return;
    spawnSessionCommand.value = "";
    spawnSessionDialogOpen.value = true;
  }

  function confirmCommandSessionTab() {
    const command = spawnSessionCommand.value.trim();
    const api = window.dbxPlugin;
    if (!api?.openWorkbench || !connectionId() || !connected() || !command) return;
    spawnSessionDialogOpen.value = false;
    void api.openWorkbench(WORKBENCH_ID, sessionTabContext(true, command), { forceNew: true, target: "tab" });
  }

  return {
    sessionTabContext,
    openNewSessionTab,
    openCopiedSessionTab,
    spawnSessionDialogOpen,
    spawnSessionCommand,
    openCommandSessionTab,
    confirmCommandSessionTab,
  };
}
