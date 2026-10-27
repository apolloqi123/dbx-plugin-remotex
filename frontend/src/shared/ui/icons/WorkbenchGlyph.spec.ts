// @vitest-environment happy-dom
// 工具条图标集：每个 glyph 必须落在 24 网格内、至少有一条可见图元，
// 防止新增图标时写成空数组或越界（14px 下越界会被按钮裁切）。
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import WorkbenchGlyph from "./WorkbenchGlyph.vue";
import { WORKBENCH_GLYPHS, type WorkbenchGlyphName } from "./glyphs";

const names = Object.keys(WORKBENCH_GLYPHS) as WorkbenchGlyphName[];

describe("WorkbenchGlyph", () => {
  it("renders every registered glyph with at least one drawable shape", () => {
    expect(names.length).toBeGreaterThanOrEqual(30);
    for (const name of names) {
      const wrapper = mount(WorkbenchGlyph, { props: { name } });
      const svg = wrapper.get("svg");
      expect(svg.attributes("viewBox"), name).toBe("0 0 24 24");
      expect(svg.attributes("aria-hidden"), name).toBe("true");
      const drawables = svg.findAll("path, circle, line");
      expect(drawables.length, `${name} has no drawable shapes`).toBeGreaterThan(0);
    }
  });

  it("keeps every shape inside the 24×24 optical grid", () => {
    const numeric = (value: string | undefined) => Number(value ?? NaN);
    for (const name of names) {
      const wrapper = mount(WorkbenchGlyph, { props: { name } });
      for (const circle of wrapper.findAll("circle")) {
        const cx = numeric(circle.attributes("cx"));
        const cy = numeric(circle.attributes("cy"));
        const r = numeric(circle.attributes("r"));
        expect(cx - r, `${name} circle left`).toBeGreaterThanOrEqual(0);
        expect(cx + r, `${name} circle right`).toBeLessThanOrEqual(24);
        expect(cy - r, `${name} circle top`).toBeGreaterThanOrEqual(0);
        expect(cy + r, `${name} circle bottom`).toBeLessThanOrEqual(24);
      }
      for (const line of wrapper.findAll("line")) {
        for (const key of ["x1", "x2"] as const) {
          const value = numeric(line.attributes(key));
          expect(value, `${name} line ${key}`).toBeGreaterThanOrEqual(0);
          expect(value, `${name} line ${key}`).toBeLessThanOrEqual(24);
        }
        for (const key of ["y1", "y2"] as const) {
          const value = numeric(line.attributes(key));
          expect(value, `${name} line ${key}`).toBeGreaterThanOrEqual(0);
          expect(value, `${name} line ${key}`).toBeLessThanOrEqual(24);
        }
      }
    }
  });
});
