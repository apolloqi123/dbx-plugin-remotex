// useSettingsPreferences：设置弹窗覆盖的**全局偏好**的模块级唯一权威。
//
// 迁移背景：同一批偏好曾有三份真相——SettingsDialog 组件内自治读写 pluginStore、
// App/组合式函数各持一份内存 ref、再经 emits 水合；webgl 反过来由 App 持有、
// 弹窗经 props/emit 镜像。现在：
// - 状态与持久化只在 store（pluginStore 键唯一写入点）；
// - SettingsDialog、App、useGhostSuggest / useHistoryPanel / useCommandHistory
//   共享同一批 ref，任何一侧写入即时对全部消费方可见；
// - 弹窗的 props/emits 暂保持不变（降低本次改动风险），但 App 侧 emit 处理器
//   已退化为 store 写入的转发，不再维护平行副本。
//
// 存储访问全部 guarded：沙箱/opaque origin 下退化为会话内存态，绝不抛错。

import { ref, type Ref } from "vue";
import { pluginStore } from "../../../shared/lib/pluginStore";
import { COMMAND_HISTORY_LIMIT, sanitizeCommandHistoryLimit } from "../../terminal/lib/panels/commandHistory";
import { loadWebglEnabled, persistWebglEnabled } from "../../terminal/lib/appearance/terminalWebgl";

/** 偏好键（唯一声明点；Dialog 与组合式函数不再各自硬编码）。 */
export const SETTINGS_PREFERENCE_KEYS = {
  ghostSuggest: "ssh-terminal-ghost-suggest",
  historyPanel: "ssh-history-panel-enabled",
  commandHistoryLimit: "ssh-command-history-limit",
  webgl: "ssh-terminal-webgl",
} as const;

export interface SettingsPreferenceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface SettingsPreferencesDeps {
  /** 存储注入（测试/宿主）；缺省 pluginStore。 */
  storage?: SettingsPreferenceStorage;
  /** WebGL 偏好读写（默认值语义 = 不落键），测试可注入。 */
  webgl?: {
    load(storage?: SettingsPreferenceStorage): boolean;
    persist(value: boolean, storage?: SettingsPreferenceStorage): void;
  };
}

export interface SettingsPreferences {
  /** 行内 ghost 自动建议开关（默认开）。 */
  readonly ghostSuggestEnabled: Ref<boolean>;
  /** Warp 式历史面板开关（默认开）。 */
  readonly historyPanelEnabled: Ref<boolean>;
  /** 命令历史环上限（10..1000，默认 100）。 */
  readonly commandHistoryLimit: Ref<number>;
  /** WebGL 渲染加速（默认开；关闭才落键）。 */
  readonly webglEnabled: Ref<boolean>;
  setGhostSuggestEnabled(value: boolean): void;
  setHistoryPanelEnabled(value: boolean): void;
  setCommandHistoryLimit(value: number): void;
  setWebglEnabled(value: boolean): void;
  /**
   * 存储水合完成后原地重读持久化值（启动时 `await pluginStore.ready` 之后调用）。
   *
   * 模块级单例在 import 期创建，此时 pluginStore 的宿主 storage 缓存尚未水合，
   * 首读只能拿到默认值。若不重读，持久化的关闭态/250 条历史上限永远不生效，
   * 历史环还会按默认 100 条截断后回写、丢数据。重读只改 ref.value，不替换
   * ref 本身，因此所有已持有同一 ref 的消费方立即看到新值。
   */
  reloadFromStorage(): void;
}

/** 读取布尔偏好：缺键 → fallback；否则仅 "0" 为关（写入语义见 writeFlag）。 */
export function readFlag(
  storage: SettingsPreferenceStorage,
  key: string,
  fallback: boolean,
): boolean {
  try {
    const raw = storage.getItem(key);
    if (raw === null) return fallback;
    return raw !== "0";
  } catch {
    // 沙箱/opaque origin：读不到按默认值处理。
    return fallback;
  }
}

function writeFlag(storage: SettingsPreferenceStorage, key: string, value: boolean): void {
  try {
    if (value) storage.removeItem(key); // 默认值不落键：默认策略演进时老用户跟随
    else storage.setItem(key, "0");
  } catch {
    // 存储不可用：仅失去记忆，本次会话开关仍然生效。
  }
}

function readWebgl(
  storage: SettingsPreferenceStorage,
  webgl: NonNullable<SettingsPreferencesDeps["webgl"]>,
): boolean {
  try {
    return webgl.load(storage);
  } catch {
    return true; // 存储/探测异常：默认开启（WebGL 不可用时另有渲染回退）
  }
}

export function createSettingsPreferences(deps: SettingsPreferencesDeps = {}): SettingsPreferences {
  const storage = deps.storage ?? pluginStore;
  const webgl = deps.webgl ?? { load: loadWebglEnabled, persist: persistWebglEnabled };

  const ghostSuggestEnabled = ref(readFlag(storage, SETTINGS_PREFERENCE_KEYS.ghostSuggest, true));
  const historyPanelEnabled = ref(readFlag(storage, SETTINGS_PREFERENCE_KEYS.historyPanel, true));
  const webglEnabled = ref(readWebgl(storage, webgl));

  const limitFromStore = (): number => {
    try {
      return sanitizeCommandHistoryLimit(storage.getItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit));
    } catch {
      return COMMAND_HISTORY_LIMIT;
    }
  };
  const commandHistoryLimit = ref(limitFromStore());

  /** 原地重读全部偏好；ref 身份不变，所有消费方即时可见。 */
  function reloadFromStorage(): void {
    ghostSuggestEnabled.value = readFlag(storage, SETTINGS_PREFERENCE_KEYS.ghostSuggest, true);
    historyPanelEnabled.value = readFlag(storage, SETTINGS_PREFERENCE_KEYS.historyPanel, true);
    webglEnabled.value = readWebgl(storage, webgl);
    commandHistoryLimit.value = limitFromStore();
  }

  return {
    ghostSuggestEnabled,
    historyPanelEnabled,
    commandHistoryLimit,
    webglEnabled,
    reloadFromStorage,
    setGhostSuggestEnabled(value: boolean): void {
      ghostSuggestEnabled.value = value;
      writeFlag(storage, SETTINGS_PREFERENCE_KEYS.ghostSuggest, value);
    },
    setHistoryPanelEnabled(value: boolean): void {
      historyPanelEnabled.value = value;
      writeFlag(storage, SETTINGS_PREFERENCE_KEYS.historyPanel, value);
    },
    setCommandHistoryLimit(value: number): void {
      const limit = sanitizeCommandHistoryLimit(value);
      commandHistoryLimit.value = limit;
      try {
        storage.setItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit, String(limit));
      } catch {
        // 存储不可用：本次会话内仍然生效。
      }
    },
    setWebglEnabled(value: boolean): void {
      webglEnabled.value = value;
      try {
        webgl.persist(value, storage);
      } catch {
        // 存储不可用：本次会话内仍然生效。
      }
    },
  };
}

/** 工作台共享的偏好 store（App 与 SettingsDialog 的同一权威）。
 * 注意：本单例在 import 期创建，早于 pluginStore.ready；宿主 storage 水合
 * 完成后必须由 entry（main.ts）调用 `reloadFromStorage()` 再挂载应用。 */
export const settingsPreferences = createSettingsPreferences();
