// i18n 运行时收敛：扁平查找表 + 补充分表不重叠。
//
// 背景：workbench.spec 只比较“合并后的表”，因此 supplemental 里与 base 重复的
// 键（symlink 曾有 6 个）会静默覆盖 base 文案——symlink.editTitle 甚至丢掉了
// 已传入的 {name}。本 spec 直接锁定：base 与 supplemental 的键集合不得重叠，
// 且 workbenchMessage 的扁平查找与既有嵌套/回退语义逐条一致。

import { describe, expect, it } from "vitest";
import {
  messages,
  supplemental,
  workbenchMessage,
  workbenchMessageTable,
  type WorkbenchLocale,
} from "./i18n";

const LOCALES: WorkbenchLocale[] = ["en", "es", "it", "ja", "pt-BR", "zh-CN", "zh-TW"];

function flatten(table: unknown, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  if (!table || typeof table !== "object") return out;
  for (const [key, value] of Object.entries(table as Record<string, unknown>)) {
    const dotted = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object") flatten(value, dotted, out);
    else if (typeof value === "string") out[dotted] = value;
  }
  return out;
}

describe("i18n supplemental 与 base 不重叠", () => {
  it.each(LOCALES)("%s：补充表不得重复 base 键（禁止静默覆盖）", (locale) => {
    const baseKeys = new Set(Object.keys(flatten(messages[locale])));
    const supplementalKeys = Object.keys(supplemental[locale] ?? {});
    const overlaps = supplementalKeys.filter((key) => baseKeys.has(key));
    expect(overlaps, `${locale} 补充分表重复了 base 键`).toEqual([]);
  });

  it("symlink.editTitle 不再被补充表遮蔽：{name} 正常插值", () => {
    expect(workbenchMessage("en", "symlink.editTitle", { name: "data" })).toContain("data");
    expect(workbenchMessage("zh-CN", "symlink.editTitle", { name: "data" })).toContain("data");
  });

  it("已删除的补充分表（symlink/remoteEdit）不再残留键", () => {
    const enSupplemental = supplemental.en ?? {};
    expect(Object.keys(enSupplemental).some((key) => key.startsWith("remoteEdit."))).toBe(false);
    expect(Object.keys(enSupplemental).some((key) => key.startsWith("symlink."))).toBe(false);
  });
});

describe("workbenchMessage 扁平查找语义", () => {
  it("命中 base 文案（嵌套键路径）并做插值", () => {
    const template = messages.en.symlink.editTitle;
    expect(workbenchMessage("en", "symlink.editTitle", { name: "x" })).toBe(
      template.replace("{name}", "x"),
    );
  });

  it("locale 缺键 → 回退 en；en 也缺 → 原样返回 key", () => {
    const esTable = flatten(messages.es);
    const enTable = flatten(messages.en);
    const missingInEs = Object.keys(enTable).find((key) => esTable[key] === undefined && enTable[key] !== "");
    if (missingInEs) {
      expect(workbenchMessage("es", missingInEs)).toBe(enTable[missingInEs]);
    }
    expect(workbenchMessage("es", "definitely.not.a.key")).toBe("definitely.not.a.key");
    expect(workbenchMessage("en", "definitely.not.a.key")).toBe("definitely.not.a.key");
  });

  it("无占位符时原样返回（不触发正则替换）", () => {
    const value = workbenchMessage("en", "pluginName");
    expect(value).toBe(messages.en.pluginName);
    expect(workbenchMessage("en", "pluginName", { ignored: "x" })).toBe(value);
  });

  it("未知占位符保留原样；数字值转字符串", () => {
    const template = "previewDialog.truncated";
    const rendered = workbenchMessage("en", template, { limit: 10, size: 20, unused: "x" });
    expect(rendered).toContain("10");
    expect(rendered).toContain("20");
    expect(rendered).not.toContain("{limit}");
    const unknown = workbenchMessage("en", template, {});
    expect(unknown).toContain("{limit}");
    expect(unknown).toContain("{size}");
  });

  it("locale 归一（zh-HK → zh-TW，未知 → zh-CN）与扁平表一致", () => {
    expect(workbenchMessage("zh-HK", "pluginName")).toBe(messages["zh-TW"].pluginName);
    expect(workbenchMessage("fr-FR", "pluginName")).toBe(messages["zh-CN"].pluginName);
    expect(workbenchMessage("pt_BR", "pluginName")).toBe(messages["pt-BR"].pluginName);
  });

  it("合并表 helper 与运行时输出在全部 locale 上一致（无覆盖时）", () => {
    for (const locale of LOCALES) {
      const table = workbenchMessageTable(locale);
      for (const key of ["pluginName", "symlink.editTitle", "close"]) {
        const expected = table[key];
        if (expected === undefined) continue;
        const rendered = workbenchMessage(locale, key, { name: "n" });
        expect(rendered).toBe(expected.replace(/\{(\w+)\}/g, (_m, name: string) => (name === "name" ? "n" : `{${name}}`)));
      }
    }
  });
});
