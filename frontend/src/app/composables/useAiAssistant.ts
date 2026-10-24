import { computed, ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import {
  aiBridgeAvailable,
  aiCompletionAvailable,
  aiRecommendationsAvailable,
  clearAiRecommendations,
  generateAiText,
  listAiModels,
  openAiConversation,
  pickDefaultAiModel,
  setAiRecommendations,
} from "../../features/agent/lib/aiBridge";
import { TailCapture } from "../../features/agent/lib/aiCapture";
import { parseAiResultText } from "../../features/agent/lib/aiResultParse";
import { buildAiAssistRequest, buildAiFixPrompt, buildAiFixRequest } from "../../features/agent/lib/aiRequests";
import { loadAiSettings, saveAiSettings, sanitizeAiSettings, type AiSettings } from "../../features/agent/lib/aiSettings";
import { shouldOfferAiFix } from "../../features/agent/lib/aiFix";
import type { Osc633StreamUpdates } from "../../features/terminal/lib/terminalCommandMarkers";
import { prepareAiOutputSnapshot, stripAnsiEscapes } from "../../shared/lib/outputRedaction";
import { confirmDialog } from "../../shared/lib/confirmDialog";

export interface AiFixBarState {
  command: string;
  exitCode: number;
  output: string;
  anchorY: number;
}

/**
 * AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）：设置权威值、修复条
 * 状态、执行期输出采集与推荐位。三条体验都只「发送快照到宿主 AI 面板」——
 * AI 生成内容一律不回写 PTY、不代执行（执行权在人）；插件零密钥，能力缺失
 * 全链路可见降级（aiBridgeOk 各调用点新鲜探测）。
 *
 * 终端/补全/命令标记均在 setup 后段创建，一律经 getter 惰性读取；`t`、
 * locale 等后声明值同样走 getter，规避声明顺序。`#` 命令搜索（aiSearch）
 * 留在 App——模板直接消费其解构态。
 */
export function useAiAssistant(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  getLocale: () => string;
  getConnectionId: () => string;
  getTerminalCwd: () => string;
  isLocalMode: () => boolean;
  getLocalShellPref: () => string;
  getLocalShells: () => ReadonlyArray<{ program: string; isDefault: boolean }>;
  isApplePlatform: () => boolean;
  sniffShell: () => string | null;
  terminal: () => Terminal | undefined;
  commandMarker: () => { command: string; active: boolean };
  suggestionViewportHeight: () => number;
  readSuggestionAnchor: () => { y: number } | null;
  replaceLine: (command: string) => void;
}) {
  const { t, showNotice } = options;

  // 设置权威值 pluginStore（单键 JSON），内存态供门控与设置页回显；bridge
  // 能力位挂载后再探一次（桥 bootstrap 与 UI 挂载同批完成），运行时调用点
  // 各做新鲜探测（aiBridgeOk），不受挂载时序影响。
  const aiSettingsState = ref<AiSettings>(loadAiSettings());
  const aiBridgeReady = ref(false);
  // 修复条状态：command/exitCode/output 已是脱敏后快照（触发时一次成型），
  // anchorY 为触发光标行顶（相对 terminal-host），bottom 样式按它落位。
  const aiFixBar = ref<AiFixBarState | null>(null);
  // 执行期输出采集（仅 fix 开启时有意义；采集与解码器随会话复位）。
  const aiOutputCapture = new TailCapture();
  let aiOutputDecoder = new TextDecoder("utf-8", { fatal: false });

  function aiBridgeOk(): boolean {
    return aiBridgeAvailable(window.dbxPlugin);
  }

  function updateAiSettings(patch: Partial<AiSettings>) {
    aiSettingsState.value = sanitizeAiSettings({ ...aiSettingsState.value, ...patch });
    saveAiSettings(aiSettingsState.value);
  }

  function resetAiFixConsent() {
    updateAiSettings({ fixConsent: false });
    showNotice(t("aiSettings.resetConsentDone"));
  }

  /** AI 上下文公共位（多语言 / 多系统 / 多 shell，Warp AI 对齐批）：locale 让
   *  LLM 以终端用户语言回答；shell 语义是命令生成的关键——本地终端取 shell
   *  偏好（basename）与平台口径，SSH 只在提示符采样判出 PowerShell/cmd 时带
   *  （复用补全的采样，不为 AI 另写探测；远端 OS 刻意不发探测命令，AI 从输出
   *  推断——诚实省略优于猜测）。 */
  function aiContextExtras() {
    const extras: Record<string, unknown> = { locale: options.getLocale() };
    const connectionId = options.getConnectionId();
    const cwd = options.getTerminalCwd();
    if (connectionId) extras.connectionId = connectionId;
    if (cwd) extras.cwd = cwd;
    if (options.isLocalMode()) {
      const shellProgram = options.getLocalShellPref() || options.getLocalShells().find((entry) => entry.isDefault)?.program;
      if (shellProgram) extras.shell = shellProgram.split(/[\\/]/).pop();
      const platform = typeof navigator !== "undefined" ? navigator.platform.toLowerCase() : "";
      extras.os = options.isApplePlatform() ? "macos" : platform.includes("win") ? "windows" : "linux";
    } else {
      const sniffed = options.sniffShell();
      if (sniffed) extras.shell = sniffed;
    }
    return extras;
  }

  // —— 推荐位（P2，host.ai.setRecommendations）：修复条在场时向宿主 AI 面板
  // 推一张「修复上次失败命令」卡（≤5 张、随工作台存续）；清空点与新命令开始/
  // 条消失/会话复位对齐，best-effort 不阻塞主链路。
  let fixRecommendationPushed = false;

  function pushFixRecommendation() {
    const bar = aiFixBar.value;
    const api = window.dbxPlugin;
    if (!bar || fixRecommendationPushed || !aiRecommendationsAvailable(api)) return;
    fixRecommendationPushed = true;
    void setAiRecommendations(api, {
      context: { ...aiContextExtras(), command: bar.command, exitCode: bar.exitCode, kind: "ai-fix" },
      items: [{ id: "ssh-ai-fix", label: t("aiFix.recommendation"), prompt: `Fix this failed command: ${bar.command} (exit code ${bar.exitCode})`, order: 0 }],
    }).catch(() => undefined);
  }

  function clearFixRecommendation() {
    if (!fixRecommendationPushed) return;
    fixRecommendationPushed = false;
    void clearAiRecommendations(window.dbxPlugin).catch(() => undefined);
  }

  /** 本地白名单分诊（ssh/alert/triage 既有 RPC，best-effort）：category +
   *  白名单建议命令作 prompt 提示位，帮 LLM 收窄误诊面；失败静默跳过。 */
  async function aiFixTriageHint(output: string): Promise<string | undefined> {
    if (!output.trim()) return undefined;
    try {
      const view = await window.dbxPlugin.invoke<{ category?: string; suggestions?: Array<{ command?: string }> }>("ssh/alert/triage", { payload: output });
      const commands = (view?.suggestions ?? []).map((item) => item?.command).filter((c): c is string => typeof c === "string" && c.length > 0).slice(0, 3);
      if (!view?.category && !commands.length) return undefined;
      return [`category=${view?.category ?? "unknown"}`, ...(commands.length ? [`whitelisted suggestions: ${commands.join("; ")}`] : [])].join(", ");
    } catch {
      return undefined;
    }
  }

  /** 修复条发起：优先直连生成（aiCompletion 能力位，桌面运行时）——列模型 →
   *  挑默认 → generateText → **用户显式确认后回填输入行**（不回车不执行，
   *  红线不变）；直连不可用/失败/取消回退面板会话（ask/agent 由设置 opt-in）。
   *  首次发送过快照预览确认（记「不再询问」）。 */
  async function aiFixFromBar() {
    const bar = aiFixBar.value;
    if (!bar) return;
    const api = window.dbxPlugin;
    if (!aiBridgeOk()) {
      showNotice(t("aiFix.unavailable"));
      return;
    }
    if (!aiSettingsState.value.fixConsent) {
      const message = `${t("aiFix.confirmBody", { command: bar.command, code: String(bar.exitCode) })}\n\n${bar.output}`;
      const accepted = await confirmDialog(message);
      if (!accepted) return;
      updateAiSettings({ fixConsent: true });
    }
    if (aiCompletionAvailable(api)) {
      try {
        const model = pickDefaultAiModel(await listAiModels(api));
        if (!model) throw new Error("no configured ai model");
        const triageHint = await aiFixTriageHint(bar.output);
        const prompt = buildAiFixPrompt({ command: bar.command, exitCode: bar.exitCode, output: bar.output, triageHint });
        const result = (await generateAiText(api, { configId: model.configId, model: model.model, prompt })).trim();
        // Warp 同款默认插入（2026-10-02 体验反馈，与 `#` 路径一致）：生成即回填
        // 光标处，不弹回填确认——回填不等于执行，回车权仍在用户；Why 行走通知。
        const parsed = parseAiResultText(result);
        if (!parsed) throw new Error("empty ai response");
        aiFixBar.value = null;
        clearFixRecommendation();
        options.replaceLine(parsed.command);
        showNotice(parsed.why || t("aiFix.filledNotice"));
        return;
      } catch {
        // 直连失败（无模型/宿主确认取消/供应商错误已泛化）→ 回退面板会话。
      }
    }
    const sent = await openAiConversation(api, buildAiFixRequest({ command: bar.command, exitCode: bar.exitCode, output: bar.output, context: aiContextExtras(), agentMode: aiSettingsState.value.agentMode })).catch(() => false);
    if (sent) {
      aiFixBar.value = null;
      clearFixRecommendation();
    }
    showNotice(t(sent ? "aiFix.sentNotice" : "aiFix.unavailable"));
  }

  /** 无选区时取 buffer 尾部 n 行（可读文本；发送前统一过脱敏管线）。 */
  function readTerminalTailLines(lines: number): string {
    const term = options.terminal();
    if (!term) return "";
    try {
      const buffer = term.buffer.active;
      const start = Math.max(0, buffer.length - lines);
      const rows: string[] = [];
      for (let y = start; y < buffer.length; y += 1) rows.push(buffer.getLine(y)?.translateToString(true) ?? "");
      return rows.join("\n").replace(/\s+$/, "");
    } catch {
      return "";
    }
  }

  /** ai-assist 热键：选中文本优先，无选区取屏幕尾部 40 行；快照过脱敏管线后
   *  发起 ask 会话。空上下文也可发（通用协助）。 */
  async function aiAssistFromTerminal() {
    if (!aiBridgeOk()) {
      showNotice(t("aiAssist.unavailable"));
      return;
    }
    const term = options.terminal();
    const selection = term?.hasSelection() ? (term.getSelection() ?? "") : "";
    const raw = selection || readTerminalTailLines(40);
    const sent = await openAiConversation(window.dbxPlugin, buildAiAssistRequest({ query: "", selection: prepareAiOutputSnapshot(raw), context: aiContextExtras(), agentMode: aiSettingsState.value.agentMode })).catch(() => false);
    showNotice(t(sent ? "aiAssist.sentNotice" : "aiAssist.unavailable"));
  }

  /** 修复条样式：底边贴触发光标行顶（宿主净高/锚点不可测时回退 40px，clamp ≥2px）。 */
  const aiFixBarStyle = computed(() => {
    const bar = aiFixBar.value;
    if (!bar) return undefined;
    const hostHeight = options.suggestionViewportHeight() || 0;
    const bottom = hostHeight > 0 && bar.anchorY >= 0 ? Math.max(2, hostHeight - bar.anchorY + 2) : 40;
    return { bottom: `${bottom}px` };
  });

  /** AI 修复条触发（Warp Fix-with-AI 同位，在 writeTerminalOutput 的采集之后
   *  调用——快进快出命令 E/C/D 同 chunk，快照必须含同 chunk 正文）：非零退出 +
   *  门干净 + 非预期非零命令（test/grep 豁免，lib/aiFix）→ 以当前快照出条；
   *  anchorY 取当前光标行顶。 */
  function maybeOfferAiFix(updates: Osc633StreamUpdates) {
    const exitCode = updates.lastExitCode;
    const command = options.commandMarker().command;
    if (typeof exitCode !== "number" || exitCode === 0 || !command) return;
    if (
      !shouldOfferAiFix(
        { bridgeAvailable: aiBridgeOk(), fixEnabled: aiSettingsState.value.fix, alternateActive: options.terminal()?.buffer.active.type === "alternate" },
        exitCode,
        command,
      )
    ) {
      return;
    }
    aiFixBar.value = {
      command,
      exitCode,
      output: prepareAiOutputSnapshot(aiOutputCapture.raw()),
      anchorY: options.readSuggestionAnchor()?.y ?? -1,
    };
    pushFixRecommendation();
  }

  /** 执行期输出采集：解码器 {stream:true} 保证多字节跨 chunk 连续；仅在
   *  执行期 chunk 或含退出码的收尾 chunk 入缓冲（与迁移前判定一致）。 */
  function captureOutputChunk(data: Uint8Array, activeOrFinishing: boolean) {
    if (!activeOrFinishing) return;
    aiOutputCapture.append(stripAnsiEscapes(aiOutputDecoder.decode(data, { stream: true })));
  }

  /** 新命令边界（OSC 633 E 帧）：采集缓冲清零 + 旧修复条让位 + 推荐卡撤下。 */
  function beginAiCommand() {
    aiOutputCapture.reset();
    aiFixBar.value = null;
    clearFixRecommendation();
  }

  /** 会话切换/断开：修复条/推荐卡/采集缓冲复位，解码器重建（避免跨会话的
   *  多字节残态）。 */
  function resetAiFixForSession() {
    aiFixBar.value = null;
    clearFixRecommendation();
    aiOutputCapture.reset();
    aiOutputDecoder = new TextDecoder("utf-8", { fatal: false });
  }

  return {
    aiSettingsState,
    aiBridgeReady,
    aiFixBar,
    aiBridgeOk,
    updateAiSettings,
    resetAiFixConsent,
    aiContextExtras,
    aiFixFromBar,
    aiAssistFromTerminal,
    aiFixBarStyle,
    maybeOfferAiFix,
    captureOutputChunk,
    beginAiCommand,
    resetAiFixForSession,
    clearFixRecommendation,
  };
}
