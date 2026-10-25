import { ref, type Ref } from "vue";
import {
  clampSuggestionMaxChars,
  clampSuggestionMinChars,
  extractPreferencesMirror,
  loadPreferencesMirror,
  persistPreferencesMirror,
  preferencesMirrorEquals,
  sanitizeConflictPolicy,
  sanitizeTransferCompressMode,
  sanitizeTransferCompressThresholdMib,
  type SftpCompressMode,
} from "../lib/preferencesMirror";
import { clampTransferConcurrency, clampTransferDownloadLimit, clampTransferMaxActive, sanitizeTransferDuplicatePolicy, type TransferDuplicatePolicy } from "../../../features/transfers/lib/transferQueue";
import { sanitizeNameEncoding, type SftpNameEncoding } from "../../../features/sftp/lib/entries/sftpName";
import type { DownloadConflictPolicy } from "../../../features/sftp/lib/transfer/downloadPrefs";
import { sanitizeActionLinksSettings, type ActionLinksSettings } from "../../../features/terminal/lib/decorations/actionLinksMatcher";
import { sanitizeGutterSettings, type GutterSettings } from "../../../features/terminal/lib/appearance/terminalGutter";

// 偏好 localStorage 键（sidecar preferences.json 为权威；localStorage 仅作
// web 浏览器直连场景的同步缓存）。
const DOWNLOAD_DIR_KEY = "ssh-download-directory";
const DOWNLOAD_USE_DEFAULT_KEY = "ssh-download-use-default-dir";
const DOWNLOAD_CONFLICT_KEY = "ssh-download-conflict-policy";
const SFTP_COMPAT_MODE_KEY = "ssh-sftp-compat-mode";
const SFTP_NAME_ENCODING_KEY = "ssh-sftp-name-encoding";
const TRANSFER_COMPRESS_MODE_KEY = "ssh-transfer-compress-mode";
const TRANSFER_COMPRESS_THRESHOLD_KEY = "ssh-transfer-compress-threshold-mib";
const SUGGESTIONS_ENABLED_KEY = "ssh-history-suggestions-enabled";
const SUGGESTIONS_MIN_CHARS_KEY = "ssh-history-suggestion-min-chars";
const SUGGESTIONS_MAX_CHARS_KEY = "ssh-history-suggestion-max-chars";

/**
 * 下载/传输/命令建议/SFTP 兼容/压缩传输偏好与 sidecar preferences 的
 * 读写、镜像与启动水合。权威存储在 sidecar preferences.json（工作台 iframe
 * 是 sandbox="allow-scripts"（opaque origin），localStorage 直接抛
 * SecurityError）；localStorage 仅作 web 浏览器直连场景的同步缓存。
 *
 * 本组偏好状态在 App 早期声明（设置弹窗适配器直接引用 load/persist 函数），
 * 因此 composable 调用点也在早期；后段才声明的偏好域（本地 shell / 高亮 /
 * 背景图 / 在线搜索）经 accessor 惰性接入，水合执行时均已就绪。
 */
export function useWorkbenchPrefs(options: {
  downloadDirState: Ref<string>;
  downloadUseDefaultState: Ref<boolean>;
  downloadConflictState: Ref<DownloadConflictPolicy>;
  transferConcurrencyState: Ref<number>;
  transferMaxActiveState: Ref<number>;
  transferDownloadLimitState: Ref<number>;
  transferDuplicateState: Ref<TransferDuplicatePolicy>;
  sftpCompatModeState: Ref<boolean>;
  sftpNameEncodingState: Ref<SftpNameEncoding>;
  transferCompressModeState: Ref<SftpCompressMode>;
  transferCompressThresholdState: Ref<number>;
  suggestionsEnabledState: Ref<boolean>;
  suggestionMinCharsState: Ref<number>;
  suggestionMaxCharsState: Ref<number>;
  transferKeys: { concurrency: string; maxActive: string; downloadLimit: string; duplicate: string };
  getLocalShell: () => string;
  setLocalShell: (value: string) => void;
  getLocalShellIntegration: () => boolean;
  setLocalShellIntegration: (value: boolean) => void;
  getLocalShellCwd: () => string;
  setLocalShellCwd: (value: string) => void;
  setLocalShellEnv: (value: Array<{ name: string; value: string }>) => void;
  setActionLinksSettings: (value: ActionLinksSettings) => void;
  setGutterSettings: (value: GutterSettings) => void;
  setWallpaperEnabled: (value: boolean) => void;
  setWallpaperOpacity: (value: number) => void;
  loadWallpaperImage: () => void;
  setCtxSearchEngines: (text: string) => void;
  settingsDialog: Ref<{ setDownloadDirDraft(value: string): void; setDownloadUseDefaultDraft(value: boolean): void } | undefined>;
  getLocalDownloadDir: () => string;
}) {
  const {
    downloadDirState, downloadUseDefaultState, downloadConflictState,
    transferConcurrencyState, transferMaxActiveState, transferDownloadLimitState,
    transferDuplicateState, sftpCompatModeState, sftpNameEncodingState,
    transferCompressModeState, transferCompressThresholdState,
    suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState,
    transferKeys, getLocalShell, setLocalShell, getLocalShellIntegration,
    setLocalShellIntegration, getLocalShellCwd, setLocalShellCwd, setLocalShellEnv,
    setActionLinksSettings, setGutterSettings, setWallpaperEnabled,
    setWallpaperOpacity, loadWallpaperImage, setCtxSearchEngines,
    settingsDialog, getLocalDownloadDir,
  } = options;

  let prefsHydrated: Promise<void> | null = null;

  function loadDownloadDir(): string {
    return downloadDirState.value;
  }

  function persistDownloadDir(value: string) {
    downloadDirState.value = value.trim();
    void syncPrefs();
  }

  function loadDownloadUseDefaultDir(): boolean {
    return downloadUseDefaultState.value;
  }

  function persistDownloadUseDefaultDir(value: boolean) {
    downloadUseDefaultState.value = value;
    void syncPrefs();
  }

  function loadDownloadConflictPolicy(): DownloadConflictPolicy {
    return downloadConflictState.value;
  }

  function persistDownloadConflictPolicy(value: DownloadConflictPolicy) {
    downloadConflictState.value = sanitizeConflictPolicy(value);
    void syncPrefs();
  }

  // 上传并发 / 重复目标策略（P1-5）：设置弹窗经适配器读写，权威态在此。

  function loadSftpCompatMode(): boolean {
    return sftpCompatModeState.value;
  }

  function persistSftpCompatMode(value: boolean) {
    sftpCompatModeState.value = value;
    void syncPrefs();
  }

  function loadSftpNameEncoding(): SftpNameEncoding {
    return sftpNameEncodingState.value;
  }

  function persistSftpNameEncoding(value: SftpNameEncoding) {
    sftpNameEncodingState.value = sanitizeNameEncoding(value);
    void syncPrefs();
  }

  // 压缩传输（M33）：设置弹窗经适配器读写，权威态在此。
  function loadTransferCompressMode(): SftpCompressMode {
    return transferCompressModeState.value;
  }

  function persistTransferCompressMode(value: SftpCompressMode) {
    transferCompressModeState.value = sanitizeTransferCompressMode(value);
    void syncPrefs();
  }

  function loadTransferCompressThreshold(): number {
    return transferCompressThresholdState.value;
  }

  function persistTransferCompressThreshold(value: number) {
    transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(value);
    void syncPrefs();
  }

  // 命令输入建议（P1-1）：设置弹窗经适配器读写，权威态在此。
  function loadSuggestionsEnabled(): boolean {
    return suggestionsEnabledState.value;
  }

  function persistSuggestionsEnabled(value: boolean) {
    suggestionsEnabledState.value = value;
    void syncPrefs();
  }

  function loadSuggestionMinChars(): number {
    return suggestionMinCharsState.value;
  }

  function persistSuggestionMinChars(value: number) {
    suggestionMinCharsState.value = clampSuggestionMinChars(value);
    void syncPrefs();
  }

  function loadSuggestionMaxChars(): number {
    return suggestionMaxCharsState.value;
  }

  function persistSuggestionMaxChars(value: number) {
    suggestionMaxCharsState.value = clampSuggestionMaxChars(value);
    void syncPrefs();
  }

  function cachePrefs() {
    try {
      if (downloadDirState.value) window.localStorage.setItem(DOWNLOAD_DIR_KEY, downloadDirState.value);
      else window.localStorage.removeItem(DOWNLOAD_DIR_KEY);
      // 默认开：只在关闭时落键（"0"），未来默认策略变化时老用户不被钉死。
      if (!downloadUseDefaultState.value) window.localStorage.setItem(DOWNLOAD_USE_DEFAULT_KEY, "0");
      else window.localStorage.removeItem(DOWNLOAD_USE_DEFAULT_KEY);
      if (downloadConflictState.value !== "rename") window.localStorage.setItem(DOWNLOAD_CONFLICT_KEY, downloadConflictState.value);
      else window.localStorage.removeItem(DOWNLOAD_CONFLICT_KEY);
      if (transferConcurrencyState.value !== 3) window.localStorage.setItem(transferKeys.concurrency, String(transferConcurrencyState.value));
      else window.localStorage.removeItem(transferKeys.concurrency);
      if (transferMaxActiveState.value !== 3) window.localStorage.setItem(transferKeys.maxActive, String(transferMaxActiveState.value));
      else window.localStorage.removeItem(transferKeys.maxActive);
      if (transferDownloadLimitState.value !== 0) window.localStorage.setItem(transferKeys.downloadLimit, String(transferDownloadLimitState.value));
      else window.localStorage.removeItem(transferKeys.downloadLimit);
      if (sftpCompatModeState.value) window.localStorage.setItem(SFTP_COMPAT_MODE_KEY, "1");
      else window.localStorage.removeItem(SFTP_COMPAT_MODE_KEY);
      if (sftpNameEncodingState.value !== "auto") window.localStorage.setItem(SFTP_NAME_ENCODING_KEY, sftpNameEncodingState.value);
      else window.localStorage.removeItem(SFTP_NAME_ENCODING_KEY);
      // 压缩传输（M33）：默认 auto/默认 64 时不落键，老用户不被钉死。
      if (transferCompressModeState.value !== "auto") window.localStorage.setItem(TRANSFER_COMPRESS_MODE_KEY, transferCompressModeState.value);
      else window.localStorage.removeItem(TRANSFER_COMPRESS_MODE_KEY);
      if (transferCompressThresholdState.value !== 64) window.localStorage.setItem(TRANSFER_COMPRESS_THRESHOLD_KEY, String(transferCompressThresholdState.value));
      else window.localStorage.removeItem(TRANSFER_COMPRESS_THRESHOLD_KEY);
      // 默认已改为 ask：显式选择 rename 也必须落键，否则下次启动会被默认值覆盖。
      window.localStorage.setItem(transferKeys.duplicate, transferDuplicateState.value);
      if (!suggestionsEnabledState.value) window.localStorage.setItem(SUGGESTIONS_ENABLED_KEY, "0");
      else window.localStorage.removeItem(SUGGESTIONS_ENABLED_KEY);
      if (suggestionMinCharsState.value !== 1) window.localStorage.setItem(SUGGESTIONS_MIN_CHARS_KEY, String(suggestionMinCharsState.value));
      else window.localStorage.removeItem(SUGGESTIONS_MIN_CHARS_KEY);
      if (suggestionMaxCharsState.value !== 64) window.localStorage.setItem(SUGGESTIONS_MAX_CHARS_KEY, String(suggestionMaxCharsState.value));
      else window.localStorage.removeItem(SUGGESTIONS_MAX_CHARS_KEY);
    } catch {
      // opaque origin：缓存跳过，内存态仍支撑本次会话。
    }
  }

  async function syncPrefs() {
    cachePrefs();
    try {
      await window.dbxPlugin.invoke("local/preferences/set", {
        downloadDir: downloadDirState.value,
        downloadUseDefaultDir: downloadUseDefaultState.value,
        downloadConflictPolicy: downloadConflictState.value,
        transfer_concurrency: transferConcurrencyState.value,
        transfer_duplicate_policy: transferDuplicateState.value,
        transfer_max_active: transferMaxActiveState.value,
        transfer_download_limit_kib: transferDownloadLimitState.value,
        sftp_compat_mode: sftpCompatModeState.value,
        sftp_name_encoding: sftpNameEncodingState.value,
        transfer_compress_mode: transferCompressModeState.value,
        transfer_compress_threshold_mib: transferCompressThresholdState.value,
        history_suggestions_enabled: suggestionsEnabledState.value,
        history_suggestion_min_chars: suggestionMinCharsState.value,
        history_suggestion_max_chars: suggestionMaxCharsState.value,
      });
      // 存储迁移批 3：RPC 成功即镜像同一份 14 键子集（云同步载荷；本键无设备
      // 本地字段，localShell 等不在内）。
      persistPreferencesMirror({
        downloadDir: downloadDirState.value,
        downloadUseDefaultDir: downloadUseDefaultState.value,
        downloadConflictPolicy: downloadConflictState.value,
        transfer_concurrency: transferConcurrencyState.value,
        transfer_duplicate_policy: transferDuplicateState.value,
        transfer_max_active: transferMaxActiveState.value,
        transfer_download_limit_kib: transferDownloadLimitState.value,
        sftp_compat_mode: sftpCompatModeState.value,
        sftp_name_encoding: sftpNameEncodingState.value,
        transfer_compress_mode: transferCompressModeState.value,
        transfer_compress_threshold_mib: transferCompressThresholdState.value,
        history_suggestions_enabled: suggestionsEnabledState.value,
        history_suggestion_min_chars: suggestionMinCharsState.value,
        history_suggestion_max_chars: suggestionMaxCharsState.value,
      });
    } catch {
      // 旧 sidecar 无此方法：本次会话内存态兜底（镜像不写，等下次成功）。
    }
  }

  function hydratePrefs(): Promise<void> {
    // Promise 记忆而非布尔：并发调用（onMounted 与本地终端直通分支）共享同一
    // 次加载，直通分支 await 它时偏好保证已就绪。
    prefsHydrated ??= hydratePrefsOnce();
    return prefsHydrated;
  }

  async function hydratePrefsOnce() {
    try {
      downloadDirState.value = window.localStorage.getItem(DOWNLOAD_DIR_KEY)?.trim() || "";
      downloadUseDefaultState.value = window.localStorage.getItem(DOWNLOAD_USE_DEFAULT_KEY) !== "0";
      downloadConflictState.value = sanitizeConflictPolicy(window.localStorage.getItem(DOWNLOAD_CONFLICT_KEY));
      transferConcurrencyState.value = clampTransferConcurrency(window.localStorage.getItem(transferKeys.concurrency) ?? undefined);
      transferDuplicateState.value = sanitizeTransferDuplicatePolicy(window.localStorage.getItem(transferKeys.duplicate));
      transferMaxActiveState.value = clampTransferMaxActive(window.localStorage.getItem(transferKeys.maxActive) ?? undefined);
      transferDownloadLimitState.value = clampTransferDownloadLimit(window.localStorage.getItem(transferKeys.downloadLimit) ?? undefined);
      sftpCompatModeState.value = window.localStorage.getItem(SFTP_COMPAT_MODE_KEY) === "1";
      sftpNameEncodingState.value = sanitizeNameEncoding(window.localStorage.getItem(SFTP_NAME_ENCODING_KEY));
      transferCompressModeState.value = sanitizeTransferCompressMode(window.localStorage.getItem(TRANSFER_COMPRESS_MODE_KEY));
      transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(window.localStorage.getItem(TRANSFER_COMPRESS_THRESHOLD_KEY) ?? undefined);
      suggestionsEnabledState.value = window.localStorage.getItem(SUGGESTIONS_ENABLED_KEY) !== "0";
      suggestionMinCharsState.value = clampSuggestionMinChars(window.localStorage.getItem(SUGGESTIONS_MIN_CHARS_KEY));
      suggestionMaxCharsState.value = clampSuggestionMaxChars(window.localStorage.getItem(SUGGESTIONS_MAX_CHARS_KEY));
    } catch {
      // 同上：等待 sidecar 权威值。
    }
    try {
      const prefs = await window.dbxPlugin.invoke<{
        downloadDir?: unknown;
        downloadUseDefaultDir?: unknown;
        downloadConflictPolicy?: unknown;
        localShell?: unknown;
        localShellIntegration?: unknown;
        action_links_enabled?: unknown;
        action_links_matchers?: unknown;
        terminal_show_line_numbers?: unknown;
        terminal_show_timestamps?: unknown;
        terminal_timestamp_format?: unknown;
        transfer_concurrency?: unknown;
        transfer_duplicate_policy?: unknown;
        transfer_max_active?: unknown;
        transfer_download_limit_kib?: unknown;
        sftp_compat_mode?: unknown;
        sftp_name_encoding?: unknown;
        transfer_compress_mode?: unknown;
        transfer_compress_threshold_mib?: unknown;
        history_suggestions_enabled?: unknown;
        history_suggestion_min_chars?: unknown;
        history_suggestion_max_chars?: unknown;
        ctx_search_engines?: unknown;
        wallpaper_enabled?: unknown;
        wallpaper_opacity?: unknown;
        rdp_experimental_enabled?: unknown;
        localShellCwd?: unknown;
        localShellEnv?: unknown;
      }>("local/preferences/get", {});
      // 背景图本体与偏好同拉（旧 sidecar 无 wallpaper/* 时静默缺席）。
      void loadWallpaperImage();
      if (typeof prefs.downloadDir === "string") downloadDirState.value = prefs.downloadDir.trim();
      if (typeof prefs.downloadUseDefaultDir === "boolean") downloadUseDefaultState.value = prefs.downloadUseDefaultDir;
      if (prefs.downloadConflictPolicy !== undefined) downloadConflictState.value = sanitizeConflictPolicy(prefs.downloadConflictPolicy);
      if (typeof prefs.localShell === "string") setLocalShell(prefs.localShell);
      if (typeof prefs.localShellIntegration === "boolean") setLocalShellIntegration(prefs.localShellIntegration);
      // 启动目录/环境变量：结构不合法静默回落默认（旧 sidecar 无键 = undefined）。
      if (typeof prefs.localShellCwd === "string") setLocalShellCwd(prefs.localShellCwd);
      if (Array.isArray(prefs.localShellEnv)) {
        setLocalShellEnv(prefs.localShellEnv
          .filter((entry): entry is { name: unknown; value: unknown } => typeof entry === "object" && entry !== null && "name" in entry)
          .map((entry) => ({ name: typeof entry.name === "string" ? entry.name : "", value: typeof entry.value === "string" ? entry.value : "" })));
      }
      // 动作链接 / gutter：键位缺省时保持内存默认（功能关闭），不无谓覆写。
      if (prefs.action_links_enabled !== undefined || prefs.action_links_matchers !== undefined) {
        setActionLinksSettings(sanitizeActionLinksSettings({ enabled: prefs.action_links_enabled, matchers: prefs.action_links_matchers }));
      }
      if (prefs.terminal_show_line_numbers !== undefined || prefs.terminal_show_timestamps !== undefined || prefs.terminal_timestamp_format !== undefined) {
        setGutterSettings(sanitizeGutterSettings({
          showLineNumbers: prefs.terminal_show_line_numbers,
          showTimestamps: prefs.terminal_show_timestamps,
          timestampFormat: prefs.terminal_timestamp_format,
        }));
      }
      if (prefs.transfer_concurrency !== undefined) transferConcurrencyState.value = clampTransferConcurrency(prefs.transfer_concurrency);
      if (prefs.transfer_duplicate_policy !== undefined) transferDuplicateState.value = sanitizeTransferDuplicatePolicy(prefs.transfer_duplicate_policy);
      if (prefs.transfer_max_active !== undefined) transferMaxActiveState.value = clampTransferMaxActive(prefs.transfer_max_active);
      if (prefs.transfer_download_limit_kib !== undefined) transferDownloadLimitState.value = clampTransferDownloadLimit(prefs.transfer_download_limit_kib);
      if (prefs.sftp_compat_mode !== undefined) sftpCompatModeState.value = prefs.sftp_compat_mode === true;
      if (prefs.transfer_compress_mode !== undefined) transferCompressModeState.value = sanitizeTransferCompressMode(prefs.transfer_compress_mode);
      if (prefs.transfer_compress_threshold_mib !== undefined) transferCompressThresholdState.value = sanitizeTransferCompressThresholdMib(prefs.transfer_compress_threshold_mib);
      if (prefs.sftp_name_encoding !== undefined) sftpNameEncodingState.value = sanitizeNameEncoding(prefs.sftp_name_encoding);
      if (prefs.history_suggestions_enabled !== undefined) suggestionsEnabledState.value = prefs.history_suggestions_enabled === true;
      if (prefs.history_suggestion_min_chars !== undefined) suggestionMinCharsState.value = clampSuggestionMinChars(prefs.history_suggestion_min_chars);
      if (prefs.history_suggestion_max_chars !== undefined) suggestionMaxCharsState.value = clampSuggestionMaxChars(prefs.history_suggestion_max_chars);
      // 在线搜索引擎表：键缺省保持默认 Google（ctxSearchEngines 解析对空/非法行鲁棒）。
      if (typeof prefs.ctx_search_engines === "string") setCtxSearchEngines(prefs.ctx_search_engines);
      // 背景图偏好：键缺省保持内存默认（关 / 45%）。
      if (typeof prefs.wallpaper_enabled === "boolean") setWallpaperEnabled(prefs.wallpaper_enabled);
      if (prefs.wallpaper_opacity !== undefined) setWallpaperOpacity(Math.min(90, Math.max(10, Math.round(Number(prefs.wallpaper_opacity) || 45))));
      // 存储迁移批 3（IMPL_PLAN_STORAGE_SYNC）：14 个传输/下载偏好键的镜像——
      // store 缺失从 sidecar 播种；漂移（云同步恢复后）则 store 赢：采纳镜像到
      // 状态并推回 sidecar（syncPrefs）。
      const mirrored = loadPreferencesMirror();
      const sidecarView = extractPreferencesMirror(prefs);
      if (mirrored === null) {
        persistPreferencesMirror(sidecarView);
      } else if (!preferencesMirrorEquals(mirrored, sidecarView)) {
        downloadDirState.value = mirrored.downloadDir;
        downloadUseDefaultState.value = mirrored.downloadUseDefaultDir;
        downloadConflictState.value = mirrored.downloadConflictPolicy;
        transferConcurrencyState.value = mirrored.transfer_concurrency;
        transferDuplicateState.value = mirrored.transfer_duplicate_policy;
        transferMaxActiveState.value = mirrored.transfer_max_active;
        transferDownloadLimitState.value = mirrored.transfer_download_limit_kib;
        sftpCompatModeState.value = mirrored.sftp_compat_mode;
        sftpNameEncodingState.value = mirrored.sftp_name_encoding;
        transferCompressModeState.value = mirrored.transfer_compress_mode;
        transferCompressThresholdState.value = mirrored.transfer_compress_threshold_mib;
        suggestionsEnabledState.value = mirrored.history_suggestions_enabled;
        suggestionMinCharsState.value = mirrored.history_suggestion_min_chars;
        suggestionMaxCharsState.value = mirrored.history_suggestion_max_chars;
        void syncPrefs();
      }
      cachePrefs();
    } catch {
      // 旧 sidecar：保留 localStorage 种子或默认。
    }
  }

  // 「使用默认地址」关闭时，下载/导出前弹出目录选择小窗。resolve 语义：
  // { dir, setDefault } = 用户确认（dir 空串 = 默认下载目录；setDefault =
  // 勾选了「将此次目录设为默认地址」）；undefined = 取消本次下载。
  interface DownloadPrompt {
    fileName: string;
    dir: string;
    setDefault: boolean;
    resolve: (result: { dir: string; setDefault: boolean } | undefined) => void;
  }
  const downloadPrompt = ref<DownloadPrompt | null>(null);

  // 单槽弹窗互斥（与 useUploadChain 的重复询问同款）：并发任务同时触发询问时，
  // 后问覆盖槽位会让先问的 promise 永久挂起（该下载/上传静默卡死）。串成链，
  // 后到者的询问等先到者结算完成后再占槽；弹窗 UI 保持一次只问一个。
  let downloadPromptAskChain: Promise<unknown> = Promise.resolve();

  function askDownloadTarget(fileName: string): Promise<{ dir: string; setDefault: boolean } | undefined> {
    const asked = downloadPromptAskChain.then(
      () =>
        new Promise<{ dir: string; setDefault: boolean } | undefined>((resolve) => {
          downloadPrompt.value = {
            fileName,
            dir: loadDownloadDir() || getLocalDownloadDir(),
            setDefault: false,
            resolve,
          };
        }),
    );
    downloadPromptAskChain = asked.catch(() => undefined);
    return asked;
  }

  function resolveDownloadPrompt(result?: { dir: string; setDefault: boolean }) {
    downloadPrompt.value?.resolve(result);
    downloadPrompt.value = null;
  }

  // 勾选「设为默认地址」后的闭环回写：目录成为新默认 + 自动打开
  // 「使用默认地址」开关（设置页草稿同步，弹窗开着也能立即看到）。
  function applyChosenDirAsDefault(dir: string) {
    const normalized = dir.trim();
    if (normalized) {
      persistDownloadDir(normalized);
      settingsDialog.value?.setDownloadDirDraft(normalized);
    }
    persistDownloadUseDefaultDir(true);
    settingsDialog.value?.setDownloadUseDefaultDraft(true);
  }

  // 「浏览」按钮打开应用内目录选择器（FolderPickerDialog，sidecar 列本机
  // 目录）：沙箱 iframe 没有目录选择 API，原生系统对话框有窗口层级问题。
  // target 记录选中值回填到哪——下载询问弹窗还是设置页草稿。
  const folderPickerTarget = ref<"prompt" | "settings" | null>(null);

  function onFolderPicked(path: string) {
    if (folderPickerTarget.value === "prompt" && downloadPrompt.value) {
      downloadPrompt.value.dir = path;
    } else if (folderPickerTarget.value === "settings") {
      settingsDialog.value?.setDownloadDirDraft(path);
    }
    folderPickerTarget.value = null;
  }

  // 「询问我」冲突策略：目标目录已有同名文件时弹确认（自动重命名/覆盖/取消）。
  interface DownloadConflictPrompt {
    fileName: string;
    path: string;
    resolve: (choice: "rename" | "overwrite" | undefined) => void;
  }
  const downloadConflictPrompt = ref<DownloadConflictPrompt | null>(null);

  let downloadConflictAskChain: Promise<unknown> = Promise.resolve();

  function askDownloadConflict(fileName: string, path: string) {
    const asked = downloadConflictAskChain.then(
      () =>
        new Promise<"rename" | "overwrite" | undefined>((resolve) => {
          downloadConflictPrompt.value = { fileName, path, resolve };
        }),
    );
    downloadConflictAskChain = asked.catch(() => undefined);
    return asked;
  }

  function resolveDownloadConflict(choice: "rename" | "overwrite" | undefined) {
    downloadConflictPrompt.value?.resolve(choice);
    downloadConflictPrompt.value = null;
  }

  // 落盘前的冲突解析：返回传给后端的冲突模式，undefined = 用户取消。
  // rename/overwrite 策略直接放行；ask 仅在确实撞名时打断，预检失败不阻断。
  async function resolveDownloadConflictFor(dir: string, fileName: string): Promise<"rename" | "overwrite" | undefined> {
    const policy = loadDownloadConflictPolicy();
    if (policy !== "ask") return policy;
    const targetDir = dir || loadDownloadDir() || getLocalDownloadDir();
    if (!targetDir) return "rename";
    try {
      const probe = await window.dbxPlugin.invoke<{ exists: boolean; path: string }>("local/fs/exists", { dir: targetDir, name: fileName });
      if (!probe.exists) return "rename";
      return await askDownloadConflict(fileName, probe.path);
    } catch {
      return "rename";
    }
  }

  return {
    loadDownloadDir,
    persistDownloadDir,
    loadDownloadUseDefaultDir,
    persistDownloadUseDefaultDir,
    loadDownloadConflictPolicy,
    persistDownloadConflictPolicy,
    loadSftpCompatMode,
    persistSftpCompatMode,
    loadSftpNameEncoding,
    persistSftpNameEncoding,
    loadTransferCompressMode,
    persistTransferCompressMode,
    loadTransferCompressThreshold,
    persistTransferCompressThreshold,
    loadSuggestionsEnabled,
    persistSuggestionsEnabled,
    loadSuggestionMinChars,
    persistSuggestionMinChars,
    loadSuggestionMaxChars,
    persistSuggestionMaxChars,
    syncPrefs,
    hydratePrefs,
    downloadPrompt,
    resolveDownloadPrompt,
    askDownloadTarget,
    applyChosenDirAsDefault,
    folderPickerTarget,
    onFolderPicked,
    downloadConflictPrompt,
    askDownloadConflict,
    resolveDownloadConflict,
    resolveDownloadConflictFor,
  };
}
