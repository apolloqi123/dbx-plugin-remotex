// settingsPreferences store 单测：默认值、持久化落键、上限归一化、共享权威
// （多个消费方拿到同一 ref）、存储不可用时不抛错。

import { describe, expect, it } from "vitest";
import {
  createSettingsPreferences,
  readFlag,
  SETTINGS_PREFERENCE_KEYS,
  type SettingsPreferenceStorage,
} from "./settingsPreferences";
import { COMMAND_HISTORY_LIMIT, sanitizeCommandHistoryBuckets } from "../../terminal/lib/panels/commandHistory";

class MemoryStorage implements SettingsPreferenceStorage {
  readonly data = new Map<string, string>();
  throwOnRead = false;
  throwOnWrite = false;
  getItem(key: string): string | null {
    if (this.throwOnRead) throw new Error("read blocked");
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    if (this.throwOnWrite) throw new Error("write blocked");
    this.data.set(key, value);
  }
  removeItem(key: string): void {
    if (this.throwOnWrite) throw new Error("write blocked");
    this.data.delete(key);
  }
}

const webgl = {
  load: (storage?: SettingsPreferenceStorage) => storage?.getItem(SETTINGS_PREFERENCE_KEYS.webgl) !== "0",
  persist: (value: boolean, storage?: SettingsPreferenceStorage) => {
    if (value) storage?.removeItem(SETTINGS_PREFERENCE_KEYS.webgl);
    else storage?.setItem(SETTINGS_PREFERENCE_KEYS.webgl, "0");
  },
};

describe("settingsPreferences store", () => {
  it("默认值：ghost/history/webgl 开，历史上限为默认值", () => {
    const store = createSettingsPreferences({ storage: new MemoryStorage(), webgl });
    expect(store.ghostSuggestEnabled.value).toBe(true);
    expect(store.historyPanelEnabled.value).toBe(true);
    expect(store.webglEnabled.value).toBe(true);
    expect(store.commandHistoryLimit.value).toBe(COMMAND_HISTORY_LIMIT);
  });

  it("从存储水合已有偏好", () => {
    const storage = new MemoryStorage();
    storage.setItem(SETTINGS_PREFERENCE_KEYS.ghostSuggest, "0");
    storage.setItem(SETTINGS_PREFERENCE_KEYS.historyPanel, "0");
    storage.setItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit, "250");
    const store = createSettingsPreferences({ storage, webgl });
    expect(store.ghostSuggestEnabled.value).toBe(false);
    expect(store.historyPanelEnabled.value).toBe(false);
    expect(store.commandHistoryLimit.value).toBe(250);
  });

  it("关闭开关落键 0；重新打开移除键（默认值不落键）", () => {
    const storage = new MemoryStorage();
    const store = createSettingsPreferences({ storage, webgl });
    store.setGhostSuggestEnabled(false);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.ghostSuggest)).toBe("0");
    store.setGhostSuggestEnabled(true);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.ghostSuggest)).toBeNull();

    store.setHistoryPanelEnabled(false);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.historyPanel)).toBe("0");
  });

  it("历史上限归一化到 10..1000 并持久化", () => {
    const storage = new MemoryStorage();
    const store = createSettingsPreferences({ storage, webgl });
    store.setCommandHistoryLimit(5);
    expect(store.commandHistoryLimit.value).toBe(10);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit)).toBe("10");
    store.setCommandHistoryLimit(99999);
    expect(store.commandHistoryLimit.value).toBe(1000);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit)).toBe("1000");
  });

  it("webgl 关闭落键 0、打开移除键", () => {
    const storage = new MemoryStorage();
    const store = createSettingsPreferences({ storage, webgl });
    store.setWebglEnabled(false);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.webgl)).toBe("0");
    expect(store.webglEnabled.value).toBe(false);
    store.setWebglEnabled(true);
    expect(storage.getItem(SETTINGS_PREFERENCE_KEYS.webgl)).toBeNull();
    expect(store.webglEnabled.value).toBe(true);
  });

  it("存储读/写抛错时退化为内存态，绝不外抛", () => {
    const storage = new MemoryStorage();
    storage.throwOnRead = true;
    const store = createSettingsPreferences({ storage, webgl });
    expect(store.ghostSuggestEnabled.value).toBe(true);
    expect(store.commandHistoryLimit.value).toBe(COMMAND_HISTORY_LIMIT);
    storage.throwOnWrite = true;
    expect(() => store.setGhostSuggestEnabled(false)).not.toThrow();
    expect(store.ghostSuggestEnabled.value).toBe(false);
    expect(() => store.setCommandHistoryLimit(30)).not.toThrow();
    expect(store.commandHistoryLimit.value).toBe(30);
  });

  it("共享权威：同一实例的消费方读写同一 ref", () => {
    const store = createSettingsPreferences({ storage: new MemoryStorage(), webgl });
    const consumerA = store.ghostSuggestEnabled;
    const consumerB = store.ghostSuggestEnabled;
    expect(consumerA).toBe(consumerB);
    store.setGhostSuggestEnabled(false);
    expect(consumerA.value).toBe(false);
    expect(consumerB.value).toBe(false);
  });

  it("readFlag：缺键返回 fallback（默认关的偏好也正确），其余仅 \"0\" 为关", () => {
    const storage = new MemoryStorage();
    expect(readFlag(storage, "missing-default-off", false)).toBe(false);
    expect(readFlag(storage, "missing-default-on", true)).toBe(true);
    storage.setItem("on", "1");
    storage.setItem("off", "0");
    expect(readFlag(storage, "on", false)).toBe(true);
    expect(readFlag(storage, "off", false)).toBe(false);
    storage.throwOnRead = true;
    expect(readFlag(storage, "on", false)).toBe(false);
    expect(readFlag(storage, "on", true)).toBe(true);
  });

  it("异步水合后 reloadFromStorage 原地刷新 ref，250 条历史上限不被默认值截断", () => {
    const storage = new MemoryStorage();
    // 模拟 pluginStore.ready 之前创建单例：宿主 storage 缓存为空 → 默认值。
    const store = createSettingsPreferences({ storage, webgl });
    const ghostRef = store.ghostSuggestEnabled;
    const historyRef = store.historyPanelEnabled;
    const webglRef = store.webglEnabled;
    const limitRef = store.commandHistoryLimit;
    expect(store.commandHistoryLimit.value).toBe(COMMAND_HISTORY_LIMIT);

    // 模拟异步水合完成：宿主键此刻才进入存储。
    const history = Array.from({ length: 250 }, (_, index) => `echo ${index}`);
    storage.setItem(SETTINGS_PREFERENCE_KEYS.ghostSuggest, "0");
    storage.setItem(SETTINGS_PREFERENCE_KEYS.historyPanel, "0");
    storage.setItem(SETTINGS_PREFERENCE_KEYS.webgl, "0");
    storage.setItem(SETTINGS_PREFERENCE_KEYS.commandHistoryLimit, "250");
    storage.setItem("ssh-command-history", JSON.stringify({ local: history }));

    store.reloadFromStorage();

    // ref 身份不变（所有消费方即时看到新值），值来自存储。
    expect(store.ghostSuggestEnabled).toBe(ghostRef);
    expect(store.historyPanelEnabled).toBe(historyRef);
    expect(store.webglEnabled).toBe(webglRef);
    expect(store.commandHistoryLimit).toBe(limitRef);
    expect(ghostRef.value).toBe(false);
    expect(historyRef.value).toBe(false);
    expect(webglRef.value).toBe(false);
    expect(limitRef.value).toBe(250);

    // 按重读后的上限水合：250 条全部保留（旧默认 100 会截掉 150 条后回写）。
    const hydrated = sanitizeCommandHistoryBuckets(
      JSON.parse(storage.getItem("ssh-command-history") ?? "null"),
      { legacyScope: "local" },
      store.commandHistoryLimit.value,
    );
    expect(hydrated.local).toHaveLength(250);
    const truncatedByDefault = sanitizeCommandHistoryBuckets(
      JSON.parse(storage.getItem("ssh-command-history") ?? "null"),
      { legacyScope: "local" },
      COMMAND_HISTORY_LIMIT,
    );
    expect(truncatedByDefault.local).toHaveLength(COMMAND_HISTORY_LIMIT);
  });
});
