import { ref, watch, type Ref } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { ImageAddon } from "@xterm/addon-image";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import {
  resolveTerminalFont,
  type TerminalFontHost,
  type TerminalFontOverride,
} from "../../lib/appearance/terminalFont";
import {
  terminalOptionPatch,
  type TerminalAppearanceState,
} from "../../lib/appearance/terminalAppearance";
import {
  terminalBehaviorOptionPatch,
  type TerminalBehaviorSettings,
} from "../../lib/appearance/terminalBehavior";
import type { TerminalThemeLike } from "../../lib/appearance/terminalScheme";
import {
  attachWebglRenderer,
  syncWebglRenderer,
  type WebglRecoveryOptions,
  type WebglRendererLike,
} from "../../lib/appearance/terminalWebgl";
import { handleOsc52ClipboardWrite } from "../../lib/emulator/terminalOsc";
import {
  cwdFromUserVar,
  parseOsc1337SetUserVar,
  parseOsc777Notify,
  parseOsc9Notification,
} from "../../lib/emulator/terminalOscChannels";
import { installMacWebkitInputFallback } from "../../lib/emulator/terminalWebkitInput";
import type { TerminalCopyCache } from "../../lib/behavior/terminalInteraction";
import { settingsPreferences } from "../../../settings/lib/settingsPreferences";
import { writeClipboardText, type ClipboardDeps } from "../../../../shared/lib/clipboardBridge";

/**
 * 主终端 xterm 实例生命周期：选项合成、addon 装载顺序、宿主编排（paste/
 * wheel/点击定位监听 + ResizeObserver）、尺寸 fit 与 WebGL renderer 开关。
 *
 * 宿主侧（App.vue）仍持有大量直接读取 `terminal` 的调用点与既有的
 * decor/输入/会话接线，故：
 * - 跨模块的值一律经 getter/回调注入（装配顺序在后，setup 期不得求值）；
 * - 创建完成的实例经 `onTerminalCreated` 回填宿主的只读别名，模板绑定名不变；
 * - 装饰引擎（highlight/actionLinks/gutter）在宿主晚于本 composable 装配，
 *   就绪后经 `onReady` 回调挂载，顺序与原 createTerminal 尾部一致。
 */
export interface TerminalInstanceOptions {
  /** 主终端宿主（模板 ref）。 */
  terminalHost: Ref<HTMLElement | undefined>;
  /** 终端 shell 上报的 cwd（OSC 1337 SetUserVar 优先通道写回）。 */
  terminalCwd: Ref<string>;
  /** 沙箱读不到系统剪贴板时的插件视图副本。 */
  terminalCopyCache: TerminalCopyCache;
  /** i18n（t 在宿主 setup 后段定义，故经回调延迟读取）。 */
  t: (key: string) => string;
  showNotice: (message: string) => void;
  /** 宿主剪贴板依赖（沙箱降级链）。 */
  clipboardDeps: () => ClipboardDeps;
  /** 外观/字体：宿主基线 + 用户覆盖，创建时合成一次。 */
  getHostFont: () => TerminalFontHost;
  getFontOverride: () => TerminalFontOverride;
  setFontSize: (size: number) => void;
  getTerminalAppearance: () => TerminalAppearanceState;
  getTerminalBehavior: () => TerminalBehaviorSettings;
  getTerminalTheme: () => TerminalThemeLike;
  registerOscColorQueryHandlers: () => void;
  /** SetUserVar cwd 优先通道的最新上报（输出管线持有）。 */
  setUserVarCwdEvent: (event: { path: string; at: number }) => void;
  /** 事件接线（实现在宿主侧的 composable 里，晚于本实例装配）。 */
  getKeyHandler: () => (event: KeyboardEvent) => boolean;
  getSearchResultsHandler: () => (results: { resultCount: number; resultIndex: number }) => void;
  getOpenTerminalLink: () => (event: MouseEvent, uri: string) => void;
  getPasteHandler: () => (event: ClipboardEvent) => void;
  getWheelHandler: () => (event: WheelEvent) => void;
  getMouseDownHandler: () => (event: MouseEvent) => void;
  getMouseUpHandler: () => (event: MouseEvent) => void;
  getBellHandler: () => () => void;
  /** 主终端键入路由（传输占用/`#` 搜索/行快照与建议刷新，Rule 留在宿主）。 */
  routeTerminalData: (data: string) => void;
  isTerminalMenuOpen: () => boolean;
  isSelectCopyEnabled: () => boolean;
  /** 尺寸/fit 目标：会话 id 随连接切换。 */
  getSshSessionId: () => string | undefined;
  getTelnetSessionId: () => string | undefined;
  getLocalSessionId: () => string | undefined;
  setTrzszTerminalColumns: (cols: number) => void;
  syncSuggestionAnchorsOnSettle: () => void;
  syncGhostAnchorOnSettle: () => void;
  isWallpaperActive: () => boolean;
  /** 终端与 addons 就绪：宿主挂载 highlight/actionLinks/gutter 装饰。 */
  onReady: () => void;
  /** 实例创建完成回填宿主只读别名。 */
  onTerminalCreated: (terminal: Terminal) => void;
}

export function useTerminalInstance(options: TerminalInstanceOptions) {
  const {
    terminalHost,
    terminalCwd,
    terminalCopyCache,
    t,
    showNotice,
    clipboardDeps,
    getHostFont,
    getFontOverride,
    setFontSize,
    getTerminalAppearance,
    getTerminalBehavior,
    getTerminalTheme,
    registerOscColorQueryHandlers,
    setUserVarCwdEvent,
    getKeyHandler,
    getSearchResultsHandler,
    getOpenTerminalLink,
    getPasteHandler,
    getWheelHandler,
    getMouseDownHandler,
    getMouseUpHandler,
    getBellHandler,
    routeTerminalData,
    isTerminalMenuOpen,
    isSelectCopyEnabled,
    getSshSessionId,
    getTelnetSessionId,
    getLocalSessionId,
    setTrzszTerminalColumns,
    syncSuggestionAnchorsOnSettle,
    syncGhostAnchorOnSettle,
    isWallpaperActive,
    onReady,
    onTerminalCreated,
  } = options;

  let terminal: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let searchAddon: SearchAddon | undefined;
  let osc52Disposable: { dispose(): void } | undefined;
  // WT-2：OSC 通知/SetUserVar 白名单通道（OSC 9 / 777 / 1337）的 disposable：
  // createTerminal 挂一次，终端销毁统一释放。解析与防御上限在 lib/terminalOscChannels.ts。
  let oscFeedDisposables: { dispose(): void }[] = [];
  let terminalPasteHandler: ((event: ClipboardEvent) => void) | undefined;
  let terminalWheelHandler: ((event: WheelEvent) => void) | undefined;
  // 点击定位光标（iTerm2 风格）：按下位置记忆 + 松开时判定“原地点击”。
  let terminalMouseDownHandler: ((event: MouseEvent) => void) | undefined;
  let terminalMouseUpHandler: ((event: MouseEvent) => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let disposeInput: { dispose(): void } | undefined;
  let disposeWebkitInputFallback: (() => void) | undefined;
  let disposeSelectionCopy: { dispose(): void } | undefined;
  let disposeTerminalBell: { dispose(): void } | undefined;
  let resizeTimer = 0;

  // 终端 WebGL 渲染加速：偏好权威 = settingsPreferences store（与设置弹窗
  // 共享同一 ref + 唯一落盘点），默认开；WebGL 不可用（headless/无 context）
  // 时静默回退 DOM 渲染。只有主终端长期挂 renderer；回放弹窗保持 DOM 渲染，
  // GIF 导出在导出期间给离屏终端临时挂载（取像素依赖 canvas），导出完随终端
  // dispose 释放 context。
  const webglEnabled = settingsPreferences.webglEnabled;
  const webglRenderer = ref<WebglRendererLike | null>(null);
  // WebGL 重建的销毁钩子与存活标志：context loss 后的延迟重建必须能在终端
  // dispose 时被取消，并释放 onContextLoss 订阅——否则 1s 定时器会在销毁后
  // 对已 dispose 的 Terminal 新建并 loadAddon 一个 context（每页 context 有限）。
  let webglCleanup: (() => void) | undefined;
  let terminalAlive = false;
  // GPU 重置/驱动切换后有限次重建 renderer（Tabby 同款策略）：成功经
  // onRecovered 回填引用，偏好已关闭则放弃重建，预算耗尽静默留在 DOM 渲染。
  function webglRecoveryOptions(): WebglRecoveryOptions<WebglRendererLike> {
    return {
      onRecovered: (addon) => {
        webglRenderer.value = addon;
      },
      enabled: () => webglEnabled.value,
      alive: () => terminalAlive,
      onCleanup: (cleanup) => {
        webglCleanup = cleanup;
      },
    };
  }

  /** 主终端宿主级监听：分屏/收起分屏会重建 #main 插槽 DOM，绑定必须随宿主迁移。 */
  function attachTerminalHost(host: HTMLElement) {
    if (terminalPasteHandler) host.addEventListener("paste", terminalPasteHandler, true);
    if (terminalWheelHandler) host.addEventListener("wheel", terminalWheelHandler, { passive: false, capture: true });
    if (terminalMouseDownHandler) host.addEventListener("mousedown", terminalMouseDownHandler);
    if (terminalMouseUpHandler) host.addEventListener("mouseup", terminalMouseUpHandler);
    ensureTerminalResizeObserver().observe(host);
  }

  function detachTerminalHost(host: HTMLElement) {
    if (terminalPasteHandler) host.removeEventListener("paste", terminalPasteHandler, true);
    if (terminalWheelHandler) host.removeEventListener("wheel", terminalWheelHandler, true);
    if (terminalMouseDownHandler) host.removeEventListener("mousedown", terminalMouseDownHandler);
    if (terminalMouseUpHandler) host.removeEventListener("mouseup", terminalMouseUpHandler);
    resizeObserver?.unobserve(host);
  }

  // 终端宿主尺寸变化（窗口缩放/命令条让位/分屏）时：fit 重排行数之外，还要
  // 重读建议浮层锚点——光标行/列的像素位置随行高与滚动变化，浮层开着时
  // 停在旧位置会错位或越过新边界（review 第二批：resize 主动重定位）。
  function ensureTerminalResizeObserver(): ResizeObserver {
    resizeObserver ??= new ResizeObserver(() => {
      scheduleFit();
      syncSuggestionAnchorsOnSettle();
      syncGhostAnchorOnSettle();
    });
    return resizeObserver;
  }

  function createTerminal() {
    if (!terminalHost.value || terminal) return;
    // 用户设置优先、未设置跟随宿主（issue #31）：合成一次，字号/字体族同步生效；
    // 重连/新开终端都经此路径拿到最终值。
    const font = resolveTerminalFont(getFontOverride(), getHostFont());
    const optionPatch = terminalOptionPatch(getTerminalAppearance().settings);
    setFontSize(font.fontSize);
    const behaviorPatch = terminalBehaviorOptionPatch(getTerminalBehavior());
    terminal = new Terminal({
      convertEol: false,
      cursorBlink: optionPatch.cursorBlink,
      // 默认细竖线（bar）：块状光标在宽字距下显得笨重，竖线更接近常规输入框观感。
      // 具体形态由外观设置覆盖（样式/闪烁/失焦态三档）。
      cursorStyle: optionPatch.cursorStyle,
      cursorInactiveStyle: optionPatch.cursorInactiveStyle,
      fontFamily: font.fontFamily,
      fontSize: font.fontSize,
      fontWeight: optionPatch.fontWeight,
      fontWeightBold: optionPatch.fontWeightBold,
      lineHeight: optionPatch.lineHeight,
      letterSpacing: optionPatch.letterSpacing,
      drawBoldTextInBrightColors: optionPatch.drawBoldTextInBrightColors,
      minimumContrastRatio: optionPatch.minimumContrastRatio,
      // 行为类选项（对标 Tabby「Terminal」页）：回滚行数默认与既有硬编码一致。
      scrollback: behaviorPatch.scrollback,
      scrollOnUserInput: behaviorPatch.scrollOnUserInput,
      wordSeparator: behaviorPatch.wordSeparator,
      ignoreBracketedPasteMode: behaviorPatch.ignoreBracketedPasteMode,
      macOptionIsMeta: behaviorPatch.macOptionIsMeta,
      // SearchAddon 的 highlight decorations 走 proposed API，缺这一项会在
      // findNext/registerDecoration 时直接抛 "allowProposedApi option"。
      allowProposedApi: true,
      theme: getTerminalTheme(),
    });
    onTerminalCreated(terminal);
    fitAddon = new FitAddon();
    searchAddon = new SearchAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(searchAddon);
    // 链接点击处理器自持：既做「需按住修饰键才可点」的门禁，也复刻 addon 默认的
    // 反制反向标签劫持（开空白窗 → 清 opener → 导航），不给安全打折扣。
    terminal.loadAddon(new WebLinksAddon(getOpenTerminalLink()));
    terminal.open(terminalHost.value);
    // 对标 electerm 的终端体验增强（须在 open 之后挂载）：
    // - Unicode 11 宽度表：emoji/新版 CJK 符号按两列计宽，旧宽度表会错位对齐；
    // - 内联图像（sixel + iTerm2 OSC 1337）：imgcat/chafa/htop 图表可渲染，
    //   像素上限与 electerm 同款 32MiB。
    // 连字 addon（@xterm/addon-ligatures）暂不引入：其 opentype.js 依赖走 Node
    // 内置模块，Electron（electerm）可用，本插件的沙箱 iframe 模块求值即崩，
    // 上游尚无浏览器安全构建。
    terminal.loadAddon(new Unicode11Addon());
    terminal.unicode.activeVersion = "11";
    terminal.loadAddon(new ImageAddon({ pixelLimit: 33_554_432 }));
    // 主题色解析失败时颜色查询的兜底应答（深色系常规值，仅在宿主下发非法颜色时触达）。
    registerOscColorQueryHandlers();
    osc52Disposable = terminal.parser.registerOscHandler(52, (data) =>
      handleOsc52ClipboardWrite(data, (text) => {
        // 远端主动写剪贴板同样进插件视图副本，供沙箱宿主的右键粘贴降级。
        terminalCopyCache.set(text);
        return writeClipboardText(text, clipboardDeps());
      }),
    );
    // WT-2（WezTerm 对标）：OSC 通知与 SetUserVar 白名单通道。xterm 内核对未知
    // OSC 静默吞掉不渲染不回显（terminalProtocolMatrix.spec.ts 实测锚点），要
    // 消费必须在 parser 层挂 handler。xterm 6 对同一 OSC id 的多 handler 按
    // 「后注册先执行」派发、true 即截停：本组 handler 只对白名单 payload 返回
    // true，其余返回 false 落回 ImageAddon（OSC 1337 内联图像）等既有通道。
    const showTerminalOscNotice = (title: string, body: string) => {
      // 通知来自远端会话，正文原样透出（已截断/压行）；无标题时补七语来源标题，
      // 与 SFTP/下载等同面通知区分开。
      showNotice(`${title || t("terminalOsc.defaultTitle")}: ${body}`);
    };
    oscFeedDisposables = [
      // OSC 9（iTerm2 growl 风格）：payload 即正文。
      terminal.parser.registerOscHandler(9, (data) => {
        const notice = parseOsc9Notification(data);
        if (notice) showTerminalOscNotice(notice.title, notice.body);
        return notice !== null;
      }),
      // OSC 777：仅消费 `notify;TITLE;BODY` 类（kitty/urxvt 约定），其余种类不认识
      // 不消费（白名单语义）。
      terminal.parser.registerOscHandler(777, (data) => {
        const notice = parseOsc777Notify(data);
        if (notice) showTerminalOscNotice(notice.title, notice.body);
        return notice !== null;
      }),
      // OSC 1337 SetUserVar（iTerm2 shell 集成）：cwd 元数据走目录跟随优先通道，
      // cd 即刻跟随、不等提示符时刻的 OSC 7；其余 user-var 只确认消费不动作
      // （WezTerm user-var-changed 事件的等价消费面，后续可在此扩展）。
      terminal.parser.registerOscHandler(1337, (data) => {
        const userVar = parseOsc1337SetUserVar(data);
        if (!userVar) return false;
        const cwd = cwdFromUserVar(userVar.name, userVar.value);
        if (cwd) {
          setUserVarCwdEvent({ path: cwd, at: Date.now() });
          terminalCwd.value = cwd;
        }
        return true;
      }),
    ];
    terminal.attachCustomKeyEventHandler(getKeyHandler());
    searchAddon.onDidChangeResults(getSearchResultsHandler());
    disposeInput = terminal.onData(routeTerminalData);
    // xterm.js 6.1 still drops rapid direct commits on macOS WKWebView when an
    // IME reports printable keys as keyCode=229 (#5887/#6045/#6144 upstream).
    // The adapter runs before xterm's hidden textarea listeners and routes only
    // single-byte text outside real composition through the same PTY path.
    // 右键菜单打开时暂停直写捕获（P2-8）：菜单操作不该漏进 PTY。
    disposeWebkitInputFallback = installMacWebkitInputFallback({ terminal, onData: (data) => {
      if (isTerminalMenuOpen()) return;
      routeTerminalData(data);
    } });
    // 选中复制（可在设置里关闭）：选择一变化即静默写入剪贴板，不弹提示。
    // 系统剪贴板写链可能整体失败（沙箱 iframe），插件视图副本必须照记——
    // 右键粘贴在宿主读链断掉时靠它兜底。
    disposeSelectionCopy = terminal.onSelectionChange(() => {
      if (!isSelectCopyEnabled() || !terminal?.hasSelection()) return;
      const selection = terminal.getSelection();
      terminalCopyCache.set(selection);
      void writeClipboardText(selection, clipboardDeps()).catch(() => undefined);
    });
    // 终端响铃（对标 Tabby「Terminal → Sound」）：xterm 6.x 移除了 bellStyle，
    // 只在每次响铃时抛 onBell，因此「关闭 / 视觉 / 听觉」三态只能由这里自行实现。
    disposeTerminalBell = terminal.onBell(getBellHandler());
    // 捕获阶段的 paste 监听：拦截 Ctrl+V 之外的所有粘贴路径（浏览器右键菜单等），
    // 统一走风险确认后再写入终端。
    const interceptTerminalPaste = getPasteHandler();
    const handleTerminalWheel = getWheelHandler();
    const handleTerminalMouseDown = getMouseDownHandler();
    const handleTerminalMouseUp = getMouseUpHandler();
    terminalPasteHandler = (event) => interceptTerminalPaste(event);
    terminalWheelHandler = (event) => handleTerminalWheel(event);
    terminalMouseDownHandler = (event) => handleTerminalMouseDown(event);
    terminalMouseUpHandler = (event) => handleTerminalMouseUp(event);
    attachTerminalHost(terminalHost.value);
    terminalAlive = true;
    if (webglEnabled.value && !isWallpaperActive()) {
      webglRenderer.value = attachWebglRenderer(terminal, () => new WebglAddon(), webglRecoveryOptions());
    }
    // 装饰挂载（highlight/actionLinks/gutter）由宿主 onReady 执行：三个装饰
    // 引擎在宿主里晚于本 composable 装配，直接调用会踩循环依赖。
    onReady();
    scheduleFit();
  }

  // 布局树结构变化（首次分屏/嵌套分屏/收起分屏/重置）时 #main 插槽会被 Vue
  // 重建：已打开的 xterm 根节点留在被移除的旧宿主里，新宿主是空的——主窗格
  // （上/左）因此空白且无法输入。把终端节点迁到新宿主，重挂宿主级监听并重新
  // fit；分屏页签由各自的 SplitTerminalPane 自行创建，不受影响。
  watch(
    terminalHost,
    (host, previous) => {
      if (previous && previous !== host) detachTerminalHost(previous);
      if (!host || !terminal?.element) return;
      if (terminal.element.parentElement !== host) host.appendChild(terminal.element);
      if (host === previous) return;
      attachTerminalHost(host);
      // 迁 DOM 后 WebGL/Canvas 画布不会自行重绘；刷新一次再交给 fit 校正行长。
      if (terminal.rows > 0) terminal.refresh(0, terminal.rows - 1);
      scheduleFit();
    },
    { flush: "post" },
  );

  // 设置开关即时生效：开=挂 renderer（失败静默回退 DOM），关=dispose。
  // 偏好写入统一走 settingsPreferences（唯一落盘点；弹窗共享同一 ref）。
  function setWebglEnabled(next: boolean) {
    settingsPreferences.setWebglEnabled(next);
    if (!terminal) return;
    if (!next) {
      // 关闭时先取消可能挂起的重建并释放 context-loss 订阅，再 dispose renderer。
      webglCleanup?.();
      webglCleanup = undefined;
    }
    webglRenderer.value = syncWebglRenderer(terminal, next, webglRenderer.value, () => new WebglAddon(), webglRecoveryOptions());
  }

  function scheduleFit() {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      if (!terminal || !fitAddon || !terminalHost.value?.clientWidth || !terminalHost.value.clientHeight) return;
      try {
        fitAddon.fit();
        // trzsz 进度条按终端列宽渲染（filter 内部文本进度条虽未启用，列宽保持同步）。
        setTrzszTerminalColumns(terminal.cols);
        const sshSessionId = getSshSessionId();
        const telnetSessionId = getTelnetSessionId();
        const localSessionId = getLocalSessionId();
        if (telnetSessionId) {
          // Telnet NAWS：sidecar 发 SB NAWS 子协商，尽力而为。
          void window.dbxPlugin.notify("telnet/resize", { sessionId: telnetSessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
        } else if (localSessionId) {
          void window.dbxPlugin.notify("local/terminal/resize", { sessionId: localSessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
        } else if (sshSessionId) {
          void window.dbxPlugin.notify("ssh/terminal/resize", { sessionId: sshSessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => undefined);
        }
      } catch {
        // The iframe can briefly be detached while DBX switches tabs.
      }
    }, 20);
  }

  /** Synchronous fit for spawn-time sizing: returns the real panel cols/rows.
   * Sessions MUST be spawned at this size — spawning at xterm's 80×24 default
   * and fitting right after makes ConPTY/PSReadLine repaint the prompt at the
   * new geometry (a second prompt line with a blank gap), which users read as
   * broken rendering. fit() throws when the iframe is detached mid-tab-switch;
   * fall back to the current terminal dims in that case. */
  function fitTerminalDimensions(): { cols: number; rows: number } {
    if (!terminal || !fitAddon) return { cols: 120, rows: 32 };
    try {
      if (!terminalHost.value?.clientWidth || !terminalHost.value.clientHeight) {
        return { cols: terminal.cols, rows: terminal.rows };
      }
      fitAddon.fit();
      setTrzszTerminalColumns(terminal.cols);
    } catch {
      // The iframe can briefly be detached while DBX switches tabs.
    }
    return { cols: terminal.cols, rows: terminal.rows };
  }

  function nextAnimationFrame(): Promise<void> {
    return new Promise((resolve) => requestAnimationFrame(() => resolve()));
  }

  /** Spawn-size gate for local shells. ConPTY repaints the whole screen buffer
   * on every changed resize, and one racing the shell's opening output renders
   * as a duplicated prompt/banner. On a dock "+" fresh tab the webview can
   * still be settling when the start request fires — zero-size host (fit then
   * falls back to xterm's default 80×24) or a pending web font swap (changes
   * cell metrics) — so the spawn size would differ from the panel's steady
   * state and the later real fit becomes a cross-geometry repaint. Poll frames
   * until two consecutive fits agree on a laid-out host; after maxWaitMs give
   * up with the last sample (never blocks boot indefinitely). */
  async function settledTerminalDimensions(maxWaitMs = 1500): Promise<{ cols: number; rows: number }> {
    const deadline = Date.now() + maxWaitMs;
    try {
      // Wait out the font swap with half the budget so the sample loop below
      // keeps its share; document.fonts can be missing in older webviews.
      await Promise.race([
        document.fonts?.ready ?? Promise.resolve(),
        new Promise((resolve) => window.setTimeout(resolve, maxWaitMs / 2)),
      ]);
    } catch {
      // Font readiness is best-effort; the consecutive-fit samples still guard.
    }
    let last: { cols: number; rows: number } | null = null;
    while (Date.now() < deadline) {
      await nextAnimationFrame();
      const host = terminalHost.value;
      if (!host?.clientWidth || !host.clientHeight) {
        // Not laid out yet — a fit here would silently measure nothing.
        last = null;
        continue;
      }
      const dims = fitTerminalDimensions();
      if (last && dims.cols === last.cols && dims.rows === last.rows) return dims;
      last = dims;
    }
    return last ?? fitTerminalDimensions();
  }

  /** The terminal's on-screen cursor (1-based) at spawn time. ConPTY's opening
   * CSI 6n asks where the cursor is and starts the shell there
   * (PSEUDOCONSOLE_INHERIT_CURSOR), so the sidecar must answer with this — and
   * the answer decides where the banner/prompt is drawn: the viewport size (what
   * the sidecar used to send) inherits the last row and re-serializes the screen
   * mid-banner, which is the stray first glyph users see. cursorY is
   * viewport-relative (0 = the row at baseY) and cursorX is the column, so both
   * need the +1 to become the 1-based position conhost expects. */
  function terminalCursorPosition(): { cursorRow: number; cursorCol: number } {
    const buffer = terminal?.buffer.active;
    if (!buffer) return { cursorRow: 1, cursorCol: 1 };
    return { cursorRow: buffer.cursorY + 1, cursorCol: buffer.cursorX + 1 };
  }

  /** 终端销毁：释放本实例持有的 timer/observer/parser/输入与铃铛 handler，
   * 最后 dispose 终端本体（宿主在其之前先摘装饰与宿主级监听）。 */
  function disposeTerminalInstance() {
    // 先标记终端已销毁并取消 WebGL 重建：挂起的 1s 重建若在 terminal.dispose()
    // 之后触发，会对已销毁的 Terminal 新建并 loadAddon 一个 WebGL context。
    terminalAlive = false;
    webglCleanup?.();
    webglCleanup = undefined;
    webglRenderer.value = null;
    window.clearTimeout(resizeTimer);
    resizeObserver?.disconnect();
    osc52Disposable?.dispose();
    osc52Disposable = undefined;
    for (const disposable of oscFeedDisposables) disposable.dispose();
    oscFeedDisposables = [];
    disposeInput?.dispose();
    disposeWebkitInputFallback?.();
    disposeWebkitInputFallback = undefined;
    disposeSelectionCopy?.dispose();
    disposeTerminalBell?.dispose();
    disposeTerminalBell = undefined;
    terminal?.dispose();
  }

  return {
    terminal: () => terminal,
    fitAddon: () => fitAddon,
    searchAddon: () => searchAddon,
    webglRenderer,
    createTerminal,
    attachTerminalHost,
    detachTerminalHost,
    ensureTerminalResizeObserver,
    scheduleFit,
    fitTerminalDimensions,
    settledTerminalDimensions,
    terminalCursorPosition,
    setWebglEnabled,
    webglRecoveryOptions,
    disposeTerminalInstance,
  };
}
