import { computed, ref } from "vue";
import { pluginStore } from "../../../../shared/lib/pluginStore";
import { randomUUID } from "../../../../shared/lib/uuid";
import { rpc } from "../../../../shared/lib/rpc";
import { settingsPreferences } from "../../../settings/lib/settingsPreferences";
import {
  mergeShellHistory,
  parseShellHistoryText,
  persistableCommandHistoryBuckets,
  pushCommandHistory,
  sanitizeCommandHistoryBuckets,
  sanitizeCommandHistoryLimit,
  trimCommandHistoryBuckets,
} from "../../lib/panels/commandHistory";
import {
  persistableHistoryMetaBuckets,
  pruneHistoryTimesBuckets,
  recordHistoryMeta,
  recordHistoryTime,
  sanitizeHistoryMetaBuckets,
  sanitizeHistoryTimesBuckets,
  type HistoryMetaRow,
} from "../../lib/panels/historyPanel";

// 命令历史按作用域隔离（scope = 连接 id；本地/串口终端各占固定桶）：↑ history
// 面板/建议/ghost/命令弹窗只读当前作用域的桶，多终端（多连接）历史不再互串
// ——以终端的历史为准。三个 pluginStore 键保持单键（宿主 storage 无列键、
// 按连接动态键不可声明，同 ssh-docker-engine 先例），值内分桶；旧版全局环
// 一次性迁入 local 桶，SSH 连接以各自 shell 历史文件回填（loadRemoteShellHistory）。
const HISTORY_SCOPE_LOCAL = "local";
const HISTORY_SCOPE_SERIAL = "serial";
const COMMAND_HISTORY_KEY = "ssh-command-history";
const COMMAND_HISTORY_TIMES_KEY = "ssh-command-history-times";
const COMMAND_HISTORY_META_KEY = "ssh-command-history-meta";
const EMPTY_HISTORY_RING: string[] = [];
const EMPTY_HISTORY_TIMES: Record<string, number> = {};
const EMPTY_HISTORY_META: Record<string, HistoryMetaRow> = {};
// 远端历史导入尚未成功时(首次拉取赶上会话未就绪/超时),面板下次打开会再试
// 一次;loadRemoteShellHistory 按会话去重,成功过即跳过。
const REMOTE_HISTORY_FETCH_LIMIT = 500;

export function useCommandHistory(options: {
  getConnectionId: () => string;
  getSerialSessionId: () => string | undefined;
}) {
  const { getConnectionId, getSerialSessionId } = options;

  // scope 引用后段声明的 connectionId/serialSession——可写 computed 惰性求值，
  // 首读在挂载后，无 TDZ 问题（watch 则必须放 setup 尾部，同 promptHints 先例）。
  const historyScope = computed(
    () => getConnectionId() || (getSerialSessionId() ? HISTORY_SCOPE_SERIAL : HISTORY_SCOPE_LOCAL),
  );
  // 历史显示条数（issue #192）：命令环上限，设置弹窗可配（10..1000，默认 100）。
  // 权威态 = settingsPreferences store（唯一落盘点；设置弹窗与 App 共享同一
  // ref）；本组合式函数在采集口 pushCommandHistory / 水合 sanitize / shell
  // 历史合并 mergeShellHistory 三处穿参，收紧时各桶立即裁剪并落盘。
  const commandHistoryLimit = settingsPreferences.commandHistoryLimit;

  // —— 三键均为单键分桶档（值内 Record<scope, …>）：读档经 *Buckets sanitize
  // （旧版全局档一次性迁入 local 桶），写档逐桶过滤/修剪后整表落盘。——
  function loadCommandHistoryBuckets(): Record<string, string[]> {
    try {
      return sanitizeCommandHistoryBuckets(JSON.parse(pluginStore.getItem(COMMAND_HISTORY_KEY) || "null"), { legacyScope: HISTORY_SCOPE_LOCAL }, commandHistoryLimit.value);
    } catch {
      return {};
    }
  }

  function loadCommandHistoryTimesBuckets(): Record<string, Record<string, number>> {
    try {
      return sanitizeHistoryTimesBuckets(JSON.parse(pluginStore.getItem(COMMAND_HISTORY_TIMES_KEY) || "null"), { legacyScope: HISTORY_SCOPE_LOCAL });
    } catch {
      return {};
    }
  }

  function loadCommandHistoryMetaBuckets(): Record<string, Record<string, HistoryMetaRow>> {
    try {
      return sanitizeHistoryMetaBuckets(JSON.parse(pluginStore.getItem(COMMAND_HISTORY_META_KEY) || "null"), { legacyScope: HISTORY_SCOPE_LOCAL });
    } catch {
      return {};
    }
  }

  const historyBuckets = ref<Record<string, string[]>>(loadCommandHistoryBuckets());
  const historyTimeBuckets = ref<Record<string, Record<string, number>>>(loadCommandHistoryTimesBuckets());
  const historyMetaBuckets = ref<Record<string, Record<string, HistoryMetaRow>>>(loadCommandHistoryMetaBuckets());
  // 当前作用域的活跃视图（可写 computed）：采集/清空/回填照旧写这三个名字，
  // 落点即当前连接的桶；缺桶时返回稳定空对象，避免消费方被新引用反复触发。
  const commandHistory = computed<string[]>({
    get: () => historyBuckets.value[historyScope.value] ?? EMPTY_HISTORY_RING,
    set: (next) => {
      historyBuckets.value = { ...historyBuckets.value, [historyScope.value]: next };
    },
  });
  // 命令 → 最近执行时刻（Warp 式 history 面板相对时间的数据面），与命令环同采集口推进。
  const commandHistoryTimes = computed<Record<string, number>>({
    get: () => historyTimeBuckets.value[historyScope.value] ?? EMPTY_HISTORY_TIMES,
    set: (next) => {
      historyTimeBuckets.value = { ...historyTimeBuckets.value, [historyScope.value]: next };
    },
  });
  // 富元数据映射（批 4d，Warp command search 同位）：命令 → {时长, 退出码}，
  // OSC 633 D 帧写入；并行结构与时间映射同款，随其持久化。
  const commandHistoryMeta = computed<Record<string, HistoryMetaRow>>({
    get: () => historyMetaBuckets.value[historyScope.value] ?? EMPTY_HISTORY_META,
    set: (next) => {
      historyMetaBuckets.value = { ...historyMetaBuckets.value, [historyScope.value]: next };
    },
  });
  // 命令历史浏览态（命令弹窗 ↑↓）；index 为 -1 表示未在浏览历史，作用域切换时作废。
  const commandHistoryIndex = ref(-1);
  const commandHistoryBackup = ref("");

  function applyCommandHistoryLimit(next: number) {
    const limit = sanitizeCommandHistoryLimit(next);
    // 偏好权威 = settingsPreferences store（唯一落盘点；App/设置弹窗共享）。
    settingsPreferences.setCommandHistoryLimit(limit);
    commandHistoryLimit.value = limit;
    // 收紧立即生效：各桶裁到新上限（最新在前，裁最旧），times/meta 以命令环
    // 为准的修剪在随后 persist 中自然跟进。
    historyBuckets.value = trimCommandHistoryBuckets(historyBuckets.value, limit);
    persistCommandHistory();
  }

  function persistCommandHistory() {
    try {
      // 逐桶过滤：疑似内嵌凭据 / 超长 / 多行的命令只留在内存，不写持久层。
      pluginStore.setItem(COMMAND_HISTORY_KEY, JSON.stringify(persistableCommandHistoryBuckets(historyBuckets.value)));
    } catch {
      // localStorage 不可用时命令历史仅保留在内存中。
    }
  }

  function persistCommandHistoryTimes() {
    try {
      pluginStore.setItem(COMMAND_HISTORY_TIMES_KEY, JSON.stringify(pruneHistoryTimesBuckets(historyTimeBuckets.value, historyBuckets.value)));
    } catch {
      // 存储不可用时时间戳仅保留在内存中（面板回退为不显示时间）。
    }
  }

  function persistCommandHistoryMeta() {
    try {
      pluginStore.setItem(COMMAND_HISTORY_META_KEY, JSON.stringify(persistableHistoryMetaBuckets(historyMetaBuckets.value, historyBuckets.value)));
    } catch {
      // 存储不可用时元数据仅保留在内存中（面板回退为不显示时长/退出码）。
    }
  }

  /**
   * 终端命令历史的统一采集口：命令环 + 执行时间映射同步推进、一并持久化。
   * 五个采集点（trackPendingInput 回车行兜底 / OSC 633 E 帧 / 命令弹窗
   * runCommand / 批量发送 / 建议自动执行）共用，保证环与时间映射不漂移。
   */
  function pushTerminalCommandHistory(command: string) {
    commandHistory.value = pushCommandHistory(commandHistory.value, command, commandHistoryLimit.value);
    commandHistoryTimes.value = recordHistoryTime(commandHistoryTimes.value, command, Date.now());
    persistCommandHistory();
    persistCommandHistoryTimes();
  }

  /** OSC 633 D 帧归档：命令 → {时长, 退出码}，合并写（两值分帧到达），随
   *  时间映射同款存续集淘汰 + 持久化。 */
  function recordCommandExecutionMeta(command: string, patch: Partial<HistoryMetaRow>) {
    commandHistoryMeta.value = recordHistoryMeta(commandHistoryMeta.value, command, patch, new Set(commandHistory.value));
    persistCommandHistoryMeta();
  }

  // —— 远端 shell 历史导入(↑ history 面板内容对齐 shell)——插件自采集只覆盖
  // 本会话键入的命令;连接建立后经 ssh/exec(非交互通道,不进终端视图、不回显)
  // 拉取远端历史文件存量(~/.bash_history / ~/.zsh_history,HISTFILE 优先),
  // 解析合并进命令环。成功过即不再拉;失败(超时/受限 shell/无历史文件)时移出
  // 已加载集合,面板下次打开会再试一次——首次拉取赶上会话未就绪也不至于永远
  // 只有本会话采集。命令兼容 BusyBox tail(无 `--` 分隔符)与未设 HISTFILE 的
  // 非交互 shell(~ 展开回落固定文件名)。
  const remoteHistoryLoadedSessions = new Set<string>();

  async function loadRemoteShellHistory(sessionId: string) {
    if (!sessionId || remoteHistoryLoadedSessions.has(sessionId)) return;
    remoteHistoryLoadedSessions.add(sessionId);
    try {
      const command = `for f in "$HISTFILE" ~/.bash_history ~/.zsh_history ~/.zhistory; do [ -f "$f" ] && { tail -n ${REMOTE_HISTORY_FETCH_LIMIT} "$f"; break; }; done`;
      const result = await rpc<{ success: boolean; output?: string; exitCode?: number }>("ssh/exec", {
        sessionId,
        execId: randomUUID(),
        command,
        sudo: false,
      }, { timeoutMs: 15_000 });
      if (!result?.success || !result.output?.trim()) {
        remoteHistoryLoadedSessions.delete(sessionId);
        return;
      }
      const shellLines = parseShellHistoryText(result.output);
      if (!shellLines.length) {
        remoteHistoryLoadedSessions.delete(sessionId);
        return;
      }
      commandHistory.value = mergeShellHistory(commandHistory.value, shellLines, commandHistoryLimit.value);
      persistCommandHistory();
    } catch {
      // 通道不可用/超时:静默降级;移出集合允许面板下次打开时重试。
      remoteHistoryLoadedSessions.delete(sessionId);
    }
  }

  return {
    historyScope,
    commandHistoryLimit,
    applyCommandHistoryLimit,
    commandHistory,
    commandHistoryTimes,
    commandHistoryMeta,
    commandHistoryIndex,
    commandHistoryBackup,
    persistCommandHistory,
    persistCommandHistoryTimes,
    pushTerminalCommandHistory,
    recordCommandExecutionMeta,
    loadRemoteShellHistory,
  };
}
