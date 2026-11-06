import { describe, expect, it } from "vitest";
import {
  CATEGORY_KEYWORDS,
  categoryMatchesQuery,
  SESSION_CATEGORIES,
  SETTINGS_CATEGORIES,
} from "./settingsNavCategories";

describe("设置导航分类表", () => {
  it("分类 id 唯一且每个分类都有图标与关键词表", () => {
    const ids = SETTINGS_CATEGORIES.map((cat) => cat.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) {
      expect(CATEGORY_KEYWORDS[id].length, `${id} 关键词为空`).toBeGreaterThan(0);
    }
  });

  it("会话级分类都在分类表里", () => {
    const ids = new Set<string>(SETTINGS_CATEGORIES.map((cat) => cat.id));
    for (const id of SESSION_CATEGORIES) {
      expect(ids.has(id), `${id} 不在分类表`).toBe(true);
    }
  });
});

describe("categoryMatchesQuery", () => {
  it("匹配本地化分类名", () => {
    expect(categoryMatchesQuery("appearance", "Appearance", "appear")).toBe(true);
    expect(categoryMatchesQuery("appearance", "外观", "外观")).toBe(true);
  });

  it("匹配关键词表里的设置项而不仅是分类名", () => {
    // 用户搜的是面板内的字段（字号 / WebGL），这些词不在分类名里。
    expect(categoryMatchesQuery("appearance", "Appearance", "font")).toBe(true);
    expect(categoryMatchesQuery("appearance", "外观", "字号")).toBe(true);
    expect(categoryMatchesQuery("terminal", "Terminal", "webgl")).toBe(true);
    expect(categoryMatchesQuery("transfer", "Downloads", "并发")).toBe(true);
  });

  it("不把无关查询算作命中", () => {
    expect(categoryMatchesQuery("appearance", "Appearance", "mcp")).toBe(false);
    expect(categoryMatchesQuery("security", "Security & keys", "wallpaper")).toBe(false);
  });
});
