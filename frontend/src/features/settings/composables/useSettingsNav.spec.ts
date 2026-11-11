// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { effectScope } from "vue";
import { useSettingsNav } from "./useSettingsNav";
import { SETTINGS_CATEGORIES, SESSION_CATEGORIES } from "../lib/settingsNavCategories";

/** 在独立 scope 里跑 composable，避免 watch 泄漏到其它用例。 */
function withNav(isLocalSession: boolean) {
  const scope = effectScope();
  const nav = scope.run(() =>
    useSettingsNav({ isLocalSession: () => isLocalSession, t: (key) => key }),
  );
  if (!nav) throw new Error("useSettingsNav 未返回");
  return { nav, stop: () => scope.stop() };
}

describe("useSettingsNav 仪表盘序号", () => {
  it("SSH 会话分母为 13，序号与全量表一致", () => {
    const { nav, stop } = withNav(false);
    expect(nav.visibleSettingsCategories.value).toHaveLength(SETTINGS_CATEGORIES.length);
    nav.settingsCategory.value = "mcp";
    expect(nav.activeCategoryCounter.value).toBe(
      `${String(SETTINGS_CATEGORIES.length).padStart(2, "0")} / ${String(SETTINGS_CATEGORIES.length).padStart(2, "0")}`,
    );
    stop();
  });

  it("本地终端隐藏会话级分类后，分子不超过分母", () => {
    // 回归：分子曾按 13 项全量表取下标、分母按可见项算，切到 MCP 会显示“13 / 11”。
    const { nav, stop } = withNav(true);
    const visible = SETTINGS_CATEGORIES.filter((cat) => !SESSION_CATEGORIES.includes(cat.id));
    expect(nav.visibleSettingsCategories.value).toHaveLength(visible.length);

    for (const category of visible) {
      nav.settingsCategory.value = category.id;
      const [numerator, denominator] = nav.activeCategoryCounter.value.split(" / ").map(Number);
      expect(numerator, `${category.id} 分子超分母`).toBeLessThanOrEqual(denominator);
      expect(numerator).toBe(visible.findIndex((cat) => cat.id === category.id) + 1);
      expect(denominator).toBe(visible.length);
    }
    stop();
  });

  it("可见分类数为两位以内时序号补零", () => {
    const { nav, stop } = withNav(true);
    nav.settingsCategory.value = "appearance";
    expect(nav.activeCategoryCounter.value).toBe("01 / 11");
    stop();
  });
});
