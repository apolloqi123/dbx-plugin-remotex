import { computed, ref } from "vue";
import type { SessionInfo } from "../../../features/connections/composables/useSshConnection";
import { createConnectLog } from "../../../features/connections/lib/connectLog";
import { normalizeConnectionPort, normalizeConnectionText } from "../../../features/connections/lib/connectionInfo";
import { initialRdpSessionState, type RdpSessionStateView } from "../../../features/remote-desktop/lib/rdpFrame";
import type { RdpConnectOptions } from "../../../features/remote-desktop/components/RdpConnectDialog.vue";
import type RdpSurface from "../../../features/remote-desktop/components/RdpSurface.vue";
import { toolbarTintStyle } from "../../../shared/lib/toolbarTint";
import type { TerminalConnectionState } from "./useTerminalState";

/** 宿主 host-key 质询（连接卡片/应答动作共用）。 */
export interface HostKeyPrompt {
  challengeId: string;
  operationId: string;
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
}

interface RuntimeEndpoint {
  host?: string;
  port?: number;
}

interface ConnectionSummary {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  color?: string;
  readOnly?: boolean;
  /** 宿主连接表单的 protocol 字段（缺省 ssh）：非 SSH 连接在 openSession 里路由到各自会话。 */
  protocol?: "ssh" | "telnet" | "vnc" | "serial" | "rdp";
  /** 串口连接配置（M32-B，manifest serial_* 字段；缺失时回落连接表单）。 */
  serialPort?: string;
  serialBaud?: number;
  serialDataBits?: "7" | "8";
  serialParity?: "none" | "even" | "odd";
  serialStopBits?: "1" | "2";
  serialBackspace?: "del" | "ctrl_h";
  /** RDP 连接配置（M32-B，manifest rdp_* 字段）。 */
  rdpDomain?: string;
  rdpWidth?: number;
  rdpHeight?: number;
  rdpCertificatePolicy?: "prompt" | "strict" | "accept-temporarily";
  rdpClipboard?: boolean;
}

/**
 * 会话/连接域状态（Task 21b）：SSH 会话本体、连接卡片过渡态、host-key 质询、
 * RDP 会话共享态与宿主连接上下文派生量。
 *
 * 纯本地工厂：依赖的宿主上下文/终端态由 `useWorkbenchContext` 显式注入，
 * 行为（open/attach/close、重连、RDP 生命周期）仍在 App.vue 与各 composable。
 */
export function useSessionState(options: {
  hostContext: { readonly value: Record<string, unknown> };
  appearance: { readonly value: { readonly colorScheme: "light" | "dark" } };
  terminalState: { readonly value: TerminalConnectionState };
}) {
  const session = ref<SessionInfo>();
  // 连接卡片：用户取消（在途 open 无法中止，仅切换 UI 态并在 promise 落地后回收
  // 孤儿会话）、Show logs 展开态与连接尝试日志（环形 200 条，跨尝试保留历史）。
  const connectCancelled = ref(false);
  // 连接成功后立即揭示终端；success 标记只用于非阻塞的视觉状态。
  const connectSucceeded = ref(false);
  const connectLogsOpen = ref(false);
  const connectLog = createConnectLog();
  const hostKeyPrompt = ref<HostKeyPrompt>();
  const rememberHostKey = ref(true);
  // 批量发送命令条（Electerm quick-command bar 风格）：常驻贴在终端底部，回车
  // 生效只读门禁（来自 ssh/sessions/list 行的 readOnly：表单 read_only ∥
  // 宿主标准 read_only）。后端门禁为权威来源，前端据此禁用写操作。
  const connectionReadOnly = ref(false);

  // RDP 会话（nyaterm-parity P3-4）：与 SSH/本地/Telnet/串口/VNC 同款互斥展示，
  // 画面走 RdpSurface 画布（帧从 rdp/frame/{id} 解码），状态经 lib/rdpFrame 的
  // 纯 reducer 折叠（connecting/connected/reconnecting/closed + errorKind），
  // 断线重连展示手动 rdp/reconnect 出口。
  const rdpSession = ref<{ sessionId: string; host: string; port: number } | null>(null);
  const rdpDialogOpen = ref(false);
  // RDP 实验门控的权威在后端（rdp/start 前置检查 local/preferences 的
  // rdp_experimental_enabled，直接 RPC 也无法绕过）；M32-A 移除工具条入口后
  // 前端不再持有内存门副本。
  const rdpState = ref<RdpSessionStateView>(initialRdpSessionState());
  const rdpScaleMode = ref<RdpConnectOptions["scaleMode"]>("fit");
  const rdpSurface = ref<InstanceType<typeof RdpSurface> | null>(null);
  // 超大剪贴板的分片拼接缓冲（sidecar C2 修复：>8 MiB 的 JSON 事件会被 SDK
  // 上限静默丢弃，后端按 chunkIndex/chunkTotal 分片发送；sessionId 隔离防串话）。
  const rdpClipboardChunks = new Map<string, { parts: string[]; received: number }>();
  const isRdpMode = computed(() => rdpSession.value !== null);
  const rdpTarget = computed(() => (rdpSession.value ? `${rdpSession.value.host}:${rdpSession.value.port}` : ""));

  const connectionId = computed(() => normalizeConnectionText(options.hostContext.value.connectionId));
  const restored = computed(() => options.hostContext.value.restored === true);
  const runtimeEndpoint = computed<RuntimeEndpoint>(() => {
    const value = options.hostContext.value.runtime;
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    const raw = value as Record<string, unknown>;
    return {
      host: normalizeConnectionText(raw.host),
      port: normalizeConnectionPort(raw.port),
    };
  });
  const connection = computed<ConnectionSummary>(() => {
    const value = options.hostContext.value.connection;
    // 宿主下发的 connection 是配置对象（mockDbxHost 同契约）；对象之外一律视为缺失。
    if (!value || typeof value !== "object" || Array.isArray(value)) return {} as ConnectionSummary;
    const raw = value as Record<string, unknown>;
    // 宿主把插件表单状态放在嵌套的 external_config 里（生命周期推送同构：
    // model.rs RuntimeEndpoint 即从 connection.external_config 读 protocol）；
    // 顶层拍平与嵌套两级都读，顶层优先。
    const ext = raw.external_config && typeof raw.external_config === "object" && !Array.isArray(raw.external_config)
      ? (raw.external_config as Record<string, unknown>)
      : {};
    const field = (key: string): unknown => raw[key] ?? ext[key];
    // serial/rdp 连接配置（M32-B）：manifest select 存的是字符串值，逐项
    // 白名单归一，非法/缺失回落各 startXXXSession 的默认值。
    const resolution = typeof field("rdp_resolution") === "string" ? (field("rdp_resolution") as string).match(/^(\d{3,4})x(\d{3,4})$/) : null;
    const protocolRaw = typeof field("protocol") === "string" ? (field("protocol") as string) : undefined;
    return {
      name: normalizeConnectionText(raw.name),
      host: normalizeConnectionText(raw.host),
      port: normalizeConnectionPort(raw.port),
      username: normalizeConnectionText(raw.username),
      color: normalizeConnectionText(raw.color),
      readOnly: raw.readOnly === true,
      protocol: protocolRaw === "telnet" || protocolRaw === "vnc" || protocolRaw === "serial" || protocolRaw === "rdp" ? protocolRaw : "ssh",
      serialPort: normalizeConnectionText(field("serial_port")),
      serialBaud: [9600, 19200, 38400, 57600, 115200, 230400].includes(Number(field("serial_baud"))) ? Number(field("serial_baud")) : undefined,
      serialDataBits: field("serial_data_bits") === "7" ? "7" : field("serial_data_bits") === "8" ? "8" : undefined,
      serialParity: field("serial_parity") === "even" || field("serial_parity") === "odd" ? (field("serial_parity") as "even" | "odd") : field("serial_parity") === "none" ? ("none" as const) : undefined,
      serialStopBits: field("serial_stop_bits") === "1" ? "1" : field("serial_stop_bits") === "2" ? "2" : undefined,
      serialBackspace: field("serial_backspace") === "ctrl_h" ? "ctrl_h" : field("serial_backspace") === "del" ? "del" : undefined,
      rdpDomain: normalizeConnectionText(field("rdp_domain")),
      rdpWidth: resolution ? Number(resolution[1]) : undefined,
      rdpHeight: resolution ? Number(resolution[2]) : undefined,
      rdpCertificatePolicy: field("rdp_certificate_policy") === "strict"
        ? ("strict" as const)
        : field("rdp_certificate_policy") === "accept-temporarily"
          ? ("accept-temporarily" as const)
          : field("rdp_certificate_policy") === "prompt"
            ? ("prompt" as const)
            : undefined,
      rdpClipboard: field("rdp_clipboard") === undefined ? undefined : field("rdp_clipboard") === true || field("rdp_clipboard") === "true",
    };
  });
  // sidecar 查询到的权威协议（connection/protocol）：真实宿主推给 webview 的
  // connection 上下文缺 external_config.protocol，openSession 路由时查询结果必须
  // 写回这里，协议路由与连接级门控才有可靠的事实源——只用于当次路由而不落状态的
  // 话，isRdpConnection 在真实宿主恒为 ssh 形态，SSH 专属入口全部漏出。
  // 连接切换（宿主改推另一连接）即失效，避免把上一连接的协议带给下一个连接。
  const storedConnectionProtocol = ref("");
  // 连接协议（对标 Tabby profile）：sidecar 暂存的权威协议 > 宿主连接表单的
  // protocol 字段，缺省 ssh。非 SSH 连接由 openSession 路由到各自的会话视图，
  // 不建立 SSH 会话。
  const connectionProtocol = computed<"ssh" | "telnet" | "vnc" | "serial" | "rdp">(() => {
    const protocol = storedConnectionProtocol.value || connection.value.protocol;
    return protocol === "telnet" || protocol === "vnc" || protocol === "serial" || protocol === "rdp" ? protocol : "ssh";
  });
  // RDP 连接（宿主表单 protocol=rdp）：SSH/SFTP/主机运维整类能力在 RDP 连接上
  // 无处生效，工具条、面板与覆盖层按连接协议整体隐藏（而非置灰）——与 localUiMode
  // 的"local 模式隐藏 SSH 专属动作"惯例一致。区别于 isRdpMode（仅 RDP 会话激活
  // 期），连接级门控让断开态下残留的 SSH 入口也不再出现。并集 isRdpMode 兜住
  // 对话框直连路径（不经 openSession 查询，表单协议无从知晓）与会话已激活、
  // 协议查询尚未返回的窗口期——这两段里 SSH 入口同样一处都不该出现。
  const isRdpConnection = computed(() => connectionProtocol.value === "rdp" || isRdpMode.value);
  const canWrite = computed(() => !connection.value.readOnly && !connectionReadOnly.value);
  const connected = computed(() => options.terminalState.value === "connected" && !!session.value);
  // 连接色染色按主题分级（light 压低 alpha 保 muted 文字 AA 对比度，P2-4）。
  const toolbarStyle = computed(() => toolbarTintStyle(connection.value.color, options.appearance.value.colorScheme));

  return {
    session,
    connectCancelled,
    connectSucceeded,
    connectLogsOpen,
    connectLog,
    hostKeyPrompt,
    rememberHostKey,
    connectionReadOnly,
    rdpSession,
    rdpDialogOpen,
    rdpState,
    rdpScaleMode,
    rdpSurface,
    rdpClipboardChunks,
    isRdpMode,
    rdpTarget,
    connectionId,
    restored,
    runtimeEndpoint,
    connection,
    storedConnectionProtocol,
    connectionProtocol,
    isRdpConnection,
    canWrite,
    connected,
    toolbarStyle,
  };
}

export type SessionState = ReturnType<typeof useSessionState>;
