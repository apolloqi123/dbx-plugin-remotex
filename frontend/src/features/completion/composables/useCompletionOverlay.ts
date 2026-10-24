import type { Terminal } from "@xterm/xterm";
import type { CompletionEdit, CompletionItem, CompletionResponse } from "../lib/completion/core/types";
import type { SuggestionAnchor } from "../../../shared/lib/overlayPlacement";
import { onBeforeUnmount, ref, type Ref } from "vue";
import { CompletionController, type CompletionGeneratorChannel } from "../lib/completion/CompletionController";
import { applyEditToText, longestCommonPrefixEdit } from "../lib/completion/core/edit";
import { rankItems } from "../lib/completion/core/ranking";
import { figCompletionSource, serializeFigManifestForWorker } from "../lib/completion/fig/figCompletionSource";
import { GeneratorScheduler } from "../lib/completion/fig/generatorScheduler";
import { resolveCompletionKey, type CompletionKeyboardState } from "../lib/completion/keyboard";
import { withShellBuiltinsSource } from "../lib/completion/shell/shellBuiltins";
import { createEngineRunner } from "../lib/completion/worker/engineRunner";
import { loadCompletionEngine } from "../../../shared/lib/pluginStore";

/** 结构化补全浮层（FIG wave-1 最终架构）：浮层状态、按键消费与唯一来源 =
 *  fig 引擎的 CompletionController 接线（解析→防抖→guard→菜单）。 */
export function useCompletionOverlay(options: {
  terminal: () => Terminal | undefined;
  getPendingTerminalInput: () => string;
  commandRunning: Ref<boolean>;
  isTerminalTransferBusy: () => boolean;
  terminalCwd: Ref<string>;
  session: Ref<{ sessionId?: string } | undefined>;
  getSessionId: () => string;
  getLocalSessionId: () => string | undefined;
  sniffTerminalShell: () => "powershell" | "cmd" | null;
  completionAnchor: Ref<SuggestionAnchor | null>;
  readTerminalSuggestionAnchor: () => SuggestionAnchor | null;
  /** 双浮层仲裁（#138 丝滑度 review P0-A）：补全菜单优先时同步收起历史建议
   *  浮层（含取消空结果宽限），由 useCommandSuggestions 注入。 */
  closeSuggestionsOnly: () => void;
  /** 接受候选项的整行替换（Ctrl+U 清行重打）：由 useCommandSuggestions 注入，
   *  构造期仅存引用，调用发生在首次响应之后。 */
  replaceTerminalLine: (nextLine: string) => void;
}) {
  const { terminal: terminalGet, getPendingTerminalInput, commandRunning, isTerminalTransferBusy, terminalCwd, session, getSessionId, getLocalSessionId, sniffTerminalShell, completionAnchor, readTerminalSuggestionAnchor, closeSuggestionsOnly, replaceTerminalLine } = options;

  // 行缓冲/锚点语义与 suggestion* 一致（pendingTerminalInput +
  // readTerminalSuggestionAnchor）。引擎三态存 pluginStore
  // （ssh-completion-engine：fig-safe 默认 / fig / off），SettingsDialog 下拉
  // 自治写入，本处每次调度前直读（无缓存即时生效）。
  // 批次 2-1 接线：真实 figCompletionSource（vendored amazon-q parser + 全量
  // 语料）DEV/生产同源；声明式 generator 位经 GeneratorScheduler → hostClient
  // （completion/execute，目标机执行）异步补齐，静态候选先行（两段渲染）。
  // fig-safe 与 fig 本批次 generator 行为相同；off 时 controller 不调度。
  // 类型不标注冻结接口：collectGenerators 是 impl 上的批次 2-1 第二通道。
  // 批次 2-2 接线：engine runner 包裹同一 source——支持 data-URL worker 的环境
  // 走 worker 线程（resolve 返回 Promise，controller 三重 guard 异步交付），
  // 否则/崩溃两次后永久主线程直跑（§44），调用方无感。
  // 语料去重：manifest 的静态副本只在主线程；worker 经 init 消息接收数据面
  // 副本（serializeFigManifestForWorker），就绪前 resolve 自动走主线程。
  // Windows shell 兜底（用户反馈：cmd/PowerShell 无提示）：runner 外侧再包一层
  // 内建命令 source——fig 语料 pass-through 且提示符判出 PowerShell/cmd 时，
  // 命令名位补 cmdlet/内建命令候选；detect 读 xterm buffer，只能在主线程，
  // 故必须在 runner（worker）外侧包装，worker 内不感知。
  const completionEngineSource = createEngineRunner({
    createSource: () => figCompletionSource,
    workerInit: () => serializeFigManifestForWorker(),
  });
  // inline worker 无 GC 兜底：工作台重挂载（webview 重建/HMR）时旧 runner 的
  // 常驻线程必须显式回收，否则每次挂载泄漏一个 worker。
  onBeforeUnmount(() => completionEngineSource.dispose());
  const completionSource = withShellBuiltinsSource(completionEngineSource, sniffTerminalShell);

  // 声明式 generator 调度：目标取当前会话（ssh 优先，其次本地；串口无可执行
  // 目标 → null 即不执行），cwd 用 OSC 7/633 跟踪值（terminalCwd）。
  const completionScheduler = new GeneratorScheduler({
    // E lane 裁决：runner 只代理冻结接口的 resolve；槽位提取轻量且 generator
    // 执行面在目标机（completion/execute），collect 维持主线程直引单例。
    collect: (request) => figCompletionSource.collectGenerators(request),
    target: () => {
      const sshSessionId = session.value?.sessionId;
      if (sshSessionId) return { kind: "ssh", sessionId: sshSessionId };
      const localSessionId = getLocalSessionId();
      if (localSessionId) return { kind: "local", sessionId: localSessionId };
      return null;
    },
    cwd: () => (terminalCwd.value ? terminalCwd.value : null),
  });
  const completionGeneratorChannel: CompletionGeneratorChannel = {
    slots: (request) => completionScheduler.slots(request),
    run: (slot) => completionScheduler.run(slot),
  };
  const completionOpen = ref(false);
  const completionItems = ref<CompletionItem[]>([]);
  const completionActiveIndex = ref(0);
  // generator 在途占位态（§31）：true 仅表示「无静态候选、动态候选在途」的
  // loading 浮层（零条目占位行，Tab/Enter 放行 shell）；静态候选照常先行。
  const completionLoading = ref(false);
  // 结构化补全输入门：仅 onData 常规键入路径（refreshSuggestionsAfterInput 的
  // guard.show 分支）放行调度。粘贴/快速命令/本地重跑等旁路写入不开浮层
  // （与基线一致）；guard 抑制（alternate screen/跟随程序锁存/历史建议总开关
  // 关）同样关门。controller 在 lineChanged 调度时与 dispatch 前各查一次。
  let completionInputAllowed = false;

  function setCompletionInputAllowed(value: boolean) {
    completionInputAllowed = value;
  }

  function closeCompletionMenu() {
    completionOpen.value = false;
    completionItems.value = [];
    completionActiveIndex.value = 0;
    completionLoading.value = false;
  }

  /** 响应落地：ready（rankItems 排序截断后非空）开浮层；loading（generator
   *  在途且无静态候选）仅当浮层已开时切占位行——不主动开菜单（用户反馈：
   *  generator 位逐键闪出"加载中"悬浮层，属未请求的自动弹出）；pass-through
   *  关。首帧 ready 到达才开浮层，与 Warp/VS Code 的"无候选不弹"一致。 */
  function handleCompletionResponse(response: CompletionResponse) {
    if (response.state === "ready" && response.items.length) {
      completionLoading.value = false;
      openCompletionMenu(response);
    } else if (response.state === "loading" && completionOpen.value) {
      completionLoading.value = true;
      completionItems.value = [];
      completionActiveIndex.value = 0;
      completionAnchor.value = readTerminalSuggestionAnchor();
    } else if (response.state !== "loading") {
      closeCompletionMenu();
    }
  }

  function openCompletionMenu(response: CompletionResponse) {
    const items = rankItems(response.items);
    if (!items.length) {
      closeCompletionMenu();
      return;
    }
    completionItems.value = items;
    completionActiveIndex.value = 0;
    completionAnchor.value = readTerminalSuggestionAnchor();
    completionOpen.value = true;
    // 双浮层仲裁（#138 丝滑度 review P0-A）：worker 模式下补全响应异步到达，
    // 键入时刻的互斥检查（refreshSuggestionsAfterInput 里 completionOpen 为
    // false）已放行历史建议浮层；此处不收会出现两个同锚点浮层叠加到下一次
    // 击键。补全菜单优先（结构化候选信息量更高），同步关掉建议浮层。
    closeSuggestionsOnly();
  }

  /**
   * 结构化补全浮层的按键消费（方案 §21，规则表固化在 keyboard.ts）：
   * ↑↓ 选择、Tab 填充静态候选、Esc 关闭；**Enter 恒定放行 shell 执行当前行**
   * （return false 不消费，回车字节照发 PTY）；generator 动态位置（hint 行）
   * 与 loading 态的 Tab 同样放行——远程 shell 是最后一级 completion provider。
   */
  function handleCompletionKey(event: KeyboardEvent): boolean {
    if (event.type !== "keydown" || !completionOpen.value) return false;
    const items = completionItems.value;
    // loading（generator 在途且无静态候选）时 Tab/Enter 放行 shell
    // （keyboard.ts 规则表 §21：动态/generator 位置或 loading 一律透传）；
    // 静态候选已就位则保持静态键盘模式（active 项可 Tab 接受）。
    const state: CompletionKeyboardState = {
      menuOpen: completionOpen.value,
      hasItems: items.length > 0,
      activeItemKind: items[completionActiveIndex.value]?.kind ?? null,
      loading: completionLoading.value && items.length === 0,
    };
    switch (resolveCompletionKey(state, event.key)) {
      case "accept": {
        const item = items[completionActiveIndex.value];
        if (!item) return true;
        // 批 4b（Warp prefix.rs / shell 语义）：多候选共享同一替换区间时，Tab
        // 先补最长公共前缀并保持菜单打开（refreshCompletionMenu 按新行重算）；
        // 无推进（已到公共边界）或区间不一致才接受高亮项。
        const prefixEdit = longestCommonPrefixEdit(items, getPendingTerminalInput());
        if (prefixEdit) {
          completionController.accept({ ...item, edit: prefixEdit });
          refreshCompletionMenu();
          return true;
        }
        acceptCompletionRow(item);
        return true;
      }
      case "close":
        closeCompletionMenu();
        return true;
      case "next":
        completionActiveIndex.value = (completionActiveIndex.value + 1) % items.length;
        return true;
      case "prev":
        completionActiveIndex.value = (completionActiveIndex.value - 1 + items.length) % items.length;
        return true;
      case "passthrough":
        // Enter 恒执行当前行、Tab 交还 shell（静态候选外的透传面）：同基线，
        // 放行前关闭浮层，避免 shell 自己的补全/执行与浮层叠加。
        closeCompletionMenu();
        return false;
      default:
        return false;
    }
  }

  /** 接受候选项（§24 映射）：item.edit 经 applyEditToText 应用后仍走
   *  replaceTerminalLineWith（整行擦重打机制不变），随后同步刷新候选
   *  （request 即时冲掉挂起的防抖，保持基线的无闪断刷新时序）。 */
  function acceptCompletionRow(item: CompletionItem) {
    if (item.kind === "hint") {
      closeCompletionMenu();
      terminalGet()?.focus();
      return;
    }
    completionController.accept(item);
    refreshCompletionMenu();
    if (!completionOpen.value) terminalGet()?.focus();
  }

  /** 按当前行缓冲重算结构化补全候选：ready 开/刷新浮层，pass-through 关闭
   *  （回落历史建议，由调用方处理）。 */
  function refreshCompletionMenu() {
    completionController.request("typing");
  }

  /** 手动唤起结构化补全菜单（Warp completions 手动键 Ctrl+Space 同位）：显式
   *  请求放行输入门（completionInputAllowed 只在常规键入路径放行，热键路径
   *  必须自开门，否则 request 会被 enabled() 拦下）；抑制门锁存（跟随程序/
   *  历史面板期间）不拦显式动作——用户点名要菜单就给菜单。候选空/引擎 off
   *  由 controller 原样关闭，无副作用。 */
  function openCompletionsManually() {
    completionInputAllowed = true;
    refreshCompletionMenu();
  }

  // CompletionController（lib/completion）：调度中枢。enabled = 引擎三态
  // （off 即关）+ 输入门；session id 取当前会话（无会话空串，guard 兜底）；
  // generators 通道接声明式 generator 调度（两段渲染 + 三重 guard 在 controller）。
  const completionController = new CompletionController({
    source: completionSource,
    sessionId: getSessionId,
    readLine: getPendingTerminalInput,
    // 传输占用（zmodem/trzsz）与命令弹窗执行期间不开浮层（与 ghost 同门）：
    // 输出流里的键入回显不该触发候选请求，更不该把悬浮层盖在输出上。
    enabled: () =>
      loadCompletionEngine() !== "off" &&
      completionInputAllowed &&
      !isTerminalTransferBusy() &&
      !commandRunning.value,
    generators: completionGeneratorChannel,
    onResponse: handleCompletionResponse,
    onAcceptEdit: (edit: CompletionEdit) => {
      // 替换范围由 source 的 CompletionEdit 给出（含引号/转义表面）；越界时
      // applyEditToText 向行界收敛（行漂移防御），整行擦重打机制不变。
      const applied = applyEditToText(getPendingTerminalInput(), edit);
      replaceTerminalLine(applied.text);
    },
  });

  return {
    completionOpen,
    completionItems,
    completionActiveIndex,
    completionAnchor,
    completionLoading,
    completionController,
    closeCompletionMenu,
    refreshCompletionMenu,
    openCompletionsManually,
    acceptCompletionRow,
    handleCompletionKey,
    setCompletionInputAllowed,
  };
}
