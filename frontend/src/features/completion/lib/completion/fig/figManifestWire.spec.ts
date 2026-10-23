// figManifestWire（bundle 去重：语料跨线程数据面）单测：
// - 函数值 → 标记对象，数组长度/字段存在性保持；
// - decode 拒绝非对象载荷；
// - 语义等价：含 generator 函数的语料经数据面往返后，resolve 结果与原语料
//   逐项一致（worker 只跑 resolve；collectGenerators 只在主线程执行）。
// @vitest-environment happy-dom（经 figCompletionSource 引入 parser 产物）

import { describe, expect, it } from "vitest";
import {
  decodeFigManifest,
  encodeFigManifest,
  figArrayHoleMarker,
  figFunctionMarker,
  isFigArrayHoleMarker,
  isFigCallableValue,
  isFigFunctionMarker,
} from "./figManifestWire";
import { createFigCompletionSource } from "./figCompletionSource";
import type { FigSpecManifest, FigSourceRequest } from "./source";
// 全量语料的唯一静态副本（与 figCompletionSource 同一 import）：用于原始
// 语料 vs wire 数据面的逐命令 resolve 对比（next 的稀疏 name 数组回归）。
import generatedManifest from "../../../../../../vendor/fig-specs/spec-manifest.generated";

const baseRequest = (line: string): FigSourceRequest => ({
  line,
  requestId: 11,
  revision: 5,
  sessionId: "sess-wire",
  trigger: "typing",
});

const postProcess = (out: string) => out.split("\n").map((name) => ({ name }));

const fixture: FigSpecManifest = {
  mygen: {
    name: ["mygen"],
    description: "fixture with generator postProcess",
    subcommands: [],
    persistentOptions: [],
    options: [{ name: ["--fast"], description: "fast mode" }],
    args: [
      {
        name: "target",
        generators: [{ script: ["echo", "one"], splitOn: "\n", postProcess }],
      },
    ],
  },
  mydyn: {
    name: ["mydyn"],
    description: "fixture with function loadSpec (dynamic command)",
    // 函数形态 = 动态命令（批次 2 不解析）：数据面标记必须保持同一语义。
    loadSpec: (() => null) as unknown as string,
    subcommands: [],
    options: [],
    args: [],
  },
};

describe("figManifestWire 编解码", () => {
  it("函数值 → 标记对象，数据字段与数组长度保持不变", () => {
    const decoded = decodeFigManifest(encodeFigManifest(fixture));
    expect(Object.keys(decoded)).toEqual(["mygen", "mydyn"]);
    expect(decoded.mydyn).toMatchObject({
      name: ["mydyn"],
      loadSpec: figFunctionMarker(),
    });
    const generators = (decoded.mygen as { args: Array<{ generators: unknown[] }> }).args[0].generators;
    expect(generators).toHaveLength(1);
    expect(isFigFunctionMarker((generators[0] as { postProcess: unknown }).postProcess)).toBe(true);
    expect((generators[0] as { script: string[] }).script).toEqual(["echo", "one"]);
  });

  it("isFigCallableValue：真实函数与标记都按函数形态处理，普通数据不误判", () => {
    expect(isFigCallableValue(postProcess)).toBe(true);
    expect(isFigCallableValue(figFunctionMarker())).toBe(true);
    expect(isFigFunctionMarker(figFunctionMarker())).toBe(true);
    expect(isFigCallableValue("git")).toBe(false);
    expect(isFigCallableValue(null)).toBe(false);
    expect(isFigFunctionMarker({ __figWireFunction: true, extra: 1 })).toBe(false);
    expect(isFigFunctionMarker({ __figWireFunction: "yes" })).toBe(false);
    // 洞标记与函数标记互不误判。
    expect(isFigArrayHoleMarker(figArrayHoleMarker())).toBe(true);
    expect(isFigArrayHoleMarker(figFunctionMarker())).toBe(false);
    expect(isFigFunctionMarker(figArrayHoleMarker())).toBe(false);
    expect(isFigArrayHoleMarker(null)).toBe(false);
    expect(isFigArrayHoleMarker({ __figWireArrayHole: "yes" })).toBe(false);
  });

  it("decode 拒绝非对象 / 数组载荷（协议错误交由调用方兜底）", () => {
    expect(() => decodeFigManifest("null")).toThrow();
    expect(() => decodeFigManifest("[]")).toThrow();
    expect(() => decodeFigManifest('"manifest"')).toThrow();
  });

  it("语义等价：数据面往返后 resolve 输出与原语料一致", () => {
    const original = createFigCompletionSource({ manifest: fixture });
    const wire = createFigCompletionSource({ manifest: decodeFigManifest(encodeFigManifest(fixture)) });
    for (const line of ["mygen ", "mygen --f", "mygen --fast ", "mydyn ", "mydyn x", "unknown "]) {
      expect(wire.resolve(baseRequest(line))).toEqual(original.resolve(baseRequest(line)));
    }
  });

  it("稀疏 name 数组：洞保持为洞，不注册名为 null 的选项", () => {
    // next dev/start 的 hostPortOptions 真实形态：`["-H", , "--hostname"]`。
    const names: string[] = [];
    names[0] = "-H";
    names[2] = "--hostname";
    expect(1 in names).toBe(false);
    const sparseFixture: FigSpecManifest = {
      sparse: {
        name: ["sparse"],
        subcommands: [],
        options: [{ name: names, description: "sparse names" }],
        args: [],
      },
    };

    const decoded = decodeFigManifest(encodeFigManifest(sparseFixture));
    const decodedNames = (decoded.sparse as { options: Array<{ name: string[] }> }).options[0].name;
    expect(decodedNames).toHaveLength(3);
    expect(decodedNames[0]).toBe("-H");
    expect(decodedNames[2]).toBe("--hostname");
    expect(1 in decodedNames).toBe(false);
    expect(decodedNames).not.toContain(null);

    const original = createFigCompletionSource({ manifest: sparseFixture });
    const wire = createFigCompletionSource({ manifest: decoded });
    const originalResponse = original.resolve(baseRequest("sparse -"));
    const wireResponse = wire.resolve(baseRequest("sparse -"));
    expect(wireResponse).toEqual(originalResponse);
    const labels = (wireResponse?.items ?? []).map((item) => item.label);
    expect(labels).toEqual(expect.arrayContaining(["-H", "--hostname"]));
    expect(labels).not.toContain("null");
  });

  it("全量语料（724 commands）原始 vs wire resolve 一致，不产出 null 选项", () => {
    const manifest = generatedManifest as unknown as FigSpecManifest;
    const names = Object.keys(manifest);
    expect(names).toHaveLength(724);
    const original = createFigCompletionSource({ manifest });
    const wire = createFigCompletionSource({ manifest: decodeFigManifest(encodeFigManifest(manifest)) });
    const mismatches: string[] = [];
    for (const name of names) {
      const line = `${name} `;
      if (JSON.stringify(wire.resolve(baseRequest(line))) !== JSON.stringify(original.resolve(baseRequest(line)))) {
        mismatches.push(name);
      }
    }
    expect(mismatches).toEqual([]);
    // 回归点：next dev 的 -H/--hostname 稀疏数组。
    const nextDev = wire.resolve(baseRequest("next dev -"));
    const nextDevLabels = (nextDev?.items ?? []).map((item) => item.label);
    expect(nextDevLabels).toEqual(expect.arrayContaining(["-H", "--hostname"]));
    expect(nextDevLabels).not.toContain("null");
  });

  it("collectGenerators 只在主线程执行：数据面副本不产出可调用 postProcess", () => {
    const original = createFigCompletionSource({ manifest: fixture });
    const wire = createFigCompletionSource({ manifest: decodeFigManifest(encodeFigManifest(fixture)) });
    const originalSlots = original.collectGenerators(baseRequest("mygen "));
    expect(originalSlots).toHaveLength(1);
    expect(typeof originalSlots[0].postProcess).toBe("function");
    // worker 侧不使用槽位；即便调用也不得抛错（postProcess 缺省 = 片段即候选）。
    expect(() => wire.collectGenerators(baseRequest("mygen "))).not.toThrow();
  });
});
