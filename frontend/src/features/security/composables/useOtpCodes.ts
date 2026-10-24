import { onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { writeClipboardText } from "../../../shared/lib/clipboardBridge";
import { resolveWorkbenchId } from "../../../shared/lib/pluginContext";
import { normalizeConnectionText } from "../../connections/lib/connectionInfo";
import { createTerminalInputQueue } from "../../terminal/lib/terminalInputQueue";
import { normalizeTerminalInputBytes } from "../../terminal/lib/terminalInput";
import {
  parseGenerateResponse,
  parseOtpBindings,
  parseOtpEntries,
  pickSendTargetSession,
  tickCountdown,
  type OtpCodeState,
  type OtpEntryView,
} from "../lib/otpPanel";

// 生成失败后的自动重试退避：连续失败按 5s→10s→30s 封顶阶梯推迟 tick 的
// 自动拉码（手动点击不受退避约束，成功即清零）。没有退避时 sidecar 不可达
// 的条目会每秒重发一次 invoke 并反复刷新错误条。
const RETRY_BACKOFF_STEPS_MS = [5000, 10000, 30000];

/** OTP 面板数据层：条目/绑定列表、TOTP 倒计时与 HOTP 按需取码、复制与发送到
 *  终端输入行（sequenced-input 队列）。tick 定时器与退避计时随组件生命周期
 *  挂载/清理，错误条 6s 后自动消失。 */
export function useOtpCodes(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const { t } = options;

  const loading = ref(true);
  const entries = ref<OtpEntryView[]>([]);
  const bindings = ref<Record<string, string>>({});
  /** 条目 id → 当前验证码状态（TOTP 倒计时 / HOTP 最近一次生成）。 */
  const codes = reactive<Record<string, OtpCodeState>>({});
  const copiedId = ref("");
  const sentId = ref("");
  const actionError = ref("");
  /** 在途生成中的条目 id（reactive Set：驱动按钮 spinner / 禁用态）。 */
  const generating = reactive(new Set<string>());

  let tickTimer = 0;
  const retryBackoffAt: Record<string, number> = {};
  const retryBackoffStep: Record<string, number> = {};

  async function refresh() {
    try {
      const payload = await window.dbxPlugin.invoke("otp/list");
      entries.value = parseOtpEntries(payload);
      bindings.value = parseOtpBindings(payload);
    } catch (cause) {
      showActionError(cause);
    } finally {
      loading.value = false;
    }
  }

  async function generateFor(entry: OtpEntryView) {
    if (generating.has(entry.id)) return;
    generating.add(entry.id);
    try {
      const payload = await window.dbxPlugin.invoke("otp/generate", { entryId: entry.id });
      delete retryBackoffAt[entry.id];
      delete retryBackoffStep[entry.id];
      const state = parseGenerateResponse(payload);
      if (!state) return;
      // reused：本窗口已取过码（code 为 null）——保留旧码置灰展示，倒计时走完
      // 新窗口开启后由 tick 自动重新生成。
      codes[entry.id] = state.reused ? { ...state, code: codes[entry.id]?.code ?? "" } : state;
    } catch (cause) {
      const step = Math.min(retryBackoffStep[entry.id] ?? 0, RETRY_BACKOFF_STEPS_MS.length - 1);
      retryBackoffAt[entry.id] = Date.now() + RETRY_BACKOFF_STEPS_MS[step];
      retryBackoffStep[entry.id] = Math.min((retryBackoffStep[entry.id] ?? 0) + 1, RETRY_BACKOFF_STEPS_MS.length - 1);
      showActionError(cause);
    } finally {
      generating.delete(entry.id);
    }
  }

  async function generateAllTotp() {
    for (const entry of entries.value.filter((candidate) => candidate.otpType === "totp")) {
      await generateFor(entry);
    }
  }

  /** 每秒 tick：TOTP 剩余秒递减，归零即拉新码（新窗口）；失败退避期内跳过
   * 自动拉码，等下一次窗口再试。 */
  function tick() {
    const now = Date.now();
    for (const entry of entries.value) {
      const state = codes[entry.id];
      if (!state || entry.otpType !== "totp") continue;
      if (state.remaining > 0) {
        state.remaining = tickCountdown(state.remaining);
        continue;
      }
      if ((retryBackoffAt[entry.id] ?? 0) > now) continue;
      void generateFor(entry);
    }
  }

  async function copyCode(entry: OtpEntryView) {
    const state = codes[entry.id];
    if (!state?.code) return;
    try {
      await writeClipboardText(state.code);
      copiedId.value = entry.id;
      window.setTimeout(() => {
        if (copiedId.value === entry.id) copiedId.value = "";
      }, 1500);
    } catch (cause) {
      showActionError(cause);
    }
  }

  // 与 App.vue 的键盘通路同协议：8 字节 BE 序号 + 输入字节，经 sendBinary 写入
  // ssh/terminal/in/<sessionId>。面板自持队列实例（后端只用序号回 ack，不做
  // 顺序裁决），写入不带换行，shell 输入行停在原地。
  const inputQueue = createTerminalInputQueue({
    send: (sessionId, payload) => window.dbxPlugin.sendBinary(`ssh/terminal/in/${sessionId}`, payload),
  });

  async function resolveSendTargetSessionId(): Promise<string> {
    const api = window.dbxPlugin;
    const context = api.context ?? (await api.request<Record<string, unknown>>("host.getContext").catch(() => undefined));
    const connectionId = normalizeConnectionText(context?.connectionId);
    const workbenchId = resolveWorkbenchId(context, "");
    const result = await api.invoke<{ sessions?: unknown }>("ssh/sessions/list", {}, { timeoutMs: 10_000 });
    return pickSendTargetSession(result?.sessions, connectionId, workbenchId);
  }

  async function sendCode(entry: OtpEntryView) {
    const state = codes[entry.id];
    if (!state?.code) return;
    try {
      const sessionId = await resolveSendTargetSessionId();
      if (!sessionId) {
        actionError.value = t("otpPanel.send.noSession");
        return;
      }
      inputQueue.enqueue(sessionId, normalizeTerminalInputBytes(new TextEncoder().encode(state.code)));
      sentId.value = entry.id;
      window.setTimeout(() => {
        if (sentId.value === entry.id) sentId.value = "";
      }, 1500);
    } catch (cause) {
      showActionError(cause);
    }
  }

  function showActionError(cause: unknown) {
    actionError.value = cause instanceof Error ? cause.message : String(cause);
    window.setTimeout(() => {
      if (actionError.value === (cause instanceof Error ? cause.message : String(cause))) actionError.value = "";
    }, 6000);
  }

  /** 删除成功后同步本地状态：条目、验证码与按条目的连接绑定一起清掉。 */
  function removeEntry(id: string) {
    entries.value = entries.value.filter((entry) => entry.id !== id);
    delete codes[id];
    const nextBindings = { ...bindings.value };
    for (const [connectionId, entryId] of Object.entries(nextBindings)) {
      if (entryId === id) delete nextBindings[connectionId];
    }
    bindings.value = nextBindings;
  }

  onMounted(() => {
    tickTimer = window.setInterval(tick, 1000);
  });

  onBeforeUnmount(() => {
    if (tickTimer) window.clearInterval(tickTimer);
    tickTimer = 0;
  });

  return {
    loading,
    entries,
    bindings,
    codes,
    copiedId,
    sentId,
    actionError,
    generating,
    refresh,
    generateFor,
    generateAllTotp,
    copyCode,
    sendCode,
    showActionError,
    removeEntry,
  };
}
