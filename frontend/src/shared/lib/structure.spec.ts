import { describe, expect, it } from "vitest";

const SOURCES = import.meta.glob("../../**/*.{ts,vue}", {
  query: "?raw",
  eager: true,
  import: "default",
}) as Record<string, string>;

const MAX_SOURCE_LINES = 600;
const MAX_SPEC_LINES = 1_200;

/** 迁移期白名单：值 = 当前允许的行数；清零后删除。 */
const LINE_ALLOWLIST: Record<string, number> = {
  "./i18n/messages/base/en.ts": 602, // docker filter strings added
};

/** 迁移期方向白名单；W1 结束后必须为空，且每个条目都必须仍然违规。 */
const DIRECTION_ALLOWLIST: string[] = [];

function countLines(text: string): number {
  return text.replace(/\n$/, "").split("\n").length;
}

function isSpec(key: string): boolean {
  return (
    key.endsWith(".spec.ts") ||
    key.endsWith(".test.ts") ||
    key.includes("/testing/")
  );
}

export function lineViolations(
  sources: Record<string, string>,
  allow: Record<string, number>,
): string[] {
  const out: string[] = [];
  for (const [key, text] of Object.entries(sources)) {
    const lines = countLines(text);
    const budget = isSpec(key) ? MAX_SPEC_LINES : MAX_SOURCE_LINES;
    const allowed = allow[key];
    const limit = allowed ?? budget;
    if (lines > limit) out.push(`${key}: ${lines} > ${limit}`);
    if (allowed !== undefined && allowed > budget && lines <= budget) {
      out.push(`${key}: allowlist entry is stale (${lines} <= ${budget})`);
    }
  }
  return out;
}

/** 白名单条目对应的文件已被移动/删除。 */
export function deadLineEntries(
  sources: Record<string, string>,
  allow: Record<string, number>,
): string[] {
  return Object.keys(allow).filter((key) => sources[key] === undefined);
}

const IMPORT_SPECIFIER =
  /(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\.meta\.glob\s*\(\s*|^import\s+)["'`]([^"'`]+)["'`]/gm;

function layerOf(key: string): "shared" | "feature" | null {
  // Vite 以相对本文件的最短路径生成 glob 键：shared 内是 `./` 或单层 `../`，
  // features/app 则是 `../../features/`、`../../app/`。显式 shared 前缀兼容
  // 合成用例与未来键格式变化。
  if (key.startsWith("../../shared/")) return "shared";
  if (key.startsWith("../../features/")) return "feature";
  if (key.startsWith("./") || (key.startsWith("../") && !key.startsWith("../../"))) {
    return "shared";
  }
  return null;
}

const UPWARD_TARGET = /(?:^|\/)(?:features|app)(?:\/|$)/;
const APP_TARGET = /(?:^|\/)app(?:\/|$)/;

export function isUpwardImport(layer: "shared" | "feature", spec: string): boolean {
  const clean = spec.split("?")[0].split("#")[0];
  return layer === "shared" ? UPWARD_TARGET.test(clean) : APP_TARGET.test(clean);
}

/** `<file> -> <spec>` for every upward import in `text`; empty for unguarded files. */
export function upwardImports(key: string, text: string): string[] {
  const layer = layerOf(key);
  if (!layer) return [];
  const out: string[] = [];
  for (const match of text.matchAll(IMPORT_SPECIFIER)) {
    if (isUpwardImport(layer, match[1])) out.push(`${key} -> ${match[1]}`);
  }
  return out;
}

export function directionViolations(
  sources: Record<string, string>,
  allow: string[],
): string[] {
  const out: string[] = [];
  for (const [key, text] of Object.entries(sources)) {
    if (allow.includes(key)) continue;
    out.push(...upwardImports(key, text));
  }
  return out;
}

/** Allowlist keys that are gone from `sources` or no longer import upward. */
export function deadDirectionEntries(
  sources: Record<string, string>,
  allow: string[],
): string[] {
  return allow.filter((key) => {
    const text = sources[key];
    return text === undefined || upwardImports(key, text).length === 0;
  });
}

export function locationViolations(sources: Record<string, string>): string[] {
  const out: string[] = [];
  for (const key of Object.keys(sources)) {
    if (!key.startsWith("../../features/")) continue;
    const parts = key.slice("../../features/".length).split("/");
    if (!["components", "composables", "lib"].includes(parts[1])) out.push(key);
  }
  return out;
}

describe("source structure guards", () => {
  it("scans the shipped frontend sources", () => {
    const files = Object.keys(SOURCES);
    expect(files.length).toBeGreaterThan(100);
    expect(files.some((key) => key.endsWith("app/App.vue"))).toBe(true);
    // glob 不包含导入方自身；用同目录文件确认 shared 子树被扫描到。
    expect(
      files.some((key) => key === "./pluginContext.ts" || key.endsWith("shared/lib/pluginContext.ts")),
    ).toBe(true);
  });

  it("keeps every source file within its line budget", () => {
    const report = lineViolations(SOURCES, LINE_ALLOWLIST);
    expect(report, `oversized sources:\n${report.join("\n")}`).toEqual([]);
  });

  it("keeps every line allowlist entry live", () => {
    const dead = deadLineEntries(SOURCES, LINE_ALLOWLIST);
    expect(dead, `dead line allowlist entries:\n${dead.join("\n")}`).toEqual([]);
  });

  it("keeps imports pointing downward", () => {
    const report = directionViolations(SOURCES, DIRECTION_ALLOWLIST);
    expect(report, `upward imports:\n${report.join("\n")}`).toEqual([]);
  });

  it("keeps every direction allowlist entry live", () => {
    const dead = deadDirectionEntries(SOURCES, DIRECTION_ALLOWLIST);
    expect(dead, `dead direction allowlist entries:\n${dead.join("\n")}`).toEqual([]);
  });

  it("keeps feature files inside components/composables/lib", () => {
    const report = locationViolations(SOURCES);
    expect(report, `misplaced feature files:\n${report.join("\n")}`).toEqual([]);
  });

  it("detects synthetic violations instead of passing vacuously", () => {
    // 用 join 拼接 specifier，避免本文件被扫描时出现字面量的向上导入。
    const FEATURES_IMPORT = ["..", "features", "sftp", "lib", "a"].join("/");
    const FEATURES_DIR = ["..", "..", "features"].join("/");
    const APP_IMPORT = ["..", "..", "..", "app", "App.vue"].join("/");
    const APP_GLOB = ["..", "..", "app", "**", "*.vue"].join("/");
    expect(
      directionViolations({ "./x.ts": `import { a } from "${FEATURES_IMPORT}";` }, []),
    ).toEqual([`./x.ts -> ${FEATURES_IMPORT}`]);
    expect(
      directionViolations(
        { "../../features/sftp/lib/x.ts": `import App from "${APP_IMPORT}";` },
        [],
      ),
    ).toEqual([`../../features/sftp/lib/x.ts -> ${APP_IMPORT}`]);
    // 真实 glob 键形状：shared 文件（`./`、单层 `../`、显式前缀）都必须被检查。
    expect(
      upwardImports("../../shared/lib/x.ts", `import { a } from "${FEATURES_IMPORT}";`),
    ).toEqual([`../../shared/lib/x.ts -> ${FEATURES_IMPORT}`]);
    expect(upwardImports("./x.ts", `import { a } from "${FEATURES_IMPORT}";`)).toEqual([
      `./x.ts -> ${FEATURES_IMPORT}`,
    ]);
    expect(upwardImports("../ui/x.ts", `import { a } from "${FEATURES_IMPORT}";`)).toEqual([
      `../ui/x.ts -> ${FEATURES_IMPORT}`,
    ]);
    // 动态导入 / import.meta.glob 也是向上导入口。
    expect(
      upwardImports("../../shared/lib/x.ts", `const m = import.meta.glob("${APP_GLOB}");`),
    ).toEqual([`../../shared/lib/x.ts -> ${APP_GLOB}`]);
    expect(
      upwardImports(
        "../../features/sftp/lib/x.ts",
        `const m = import("${APP_IMPORT}");`,
      ),
    ).toEqual([`../../features/sftp/lib/x.ts -> ${APP_IMPORT}`]);
    // 扩展名缺省的目录 specifier 同样命中。
    expect(
      upwardImports("../../shared/lib/x.ts", `import { a } from "${FEATURES_DIR}";`),
    ).toEqual([`../../shared/lib/x.ts -> ${FEATURES_DIR}`]);
    // 同名字符串不误报。
    expect(
      upwardImports("../../shared/lib/x.ts", 'import { a } from "./featuresish/a";'),
    ).toEqual([]);
    expect(lineViolations({ "./x.ts": `${"x\n".repeat(601)}` }, {})).toEqual([
      "./x.ts: 601 > 600",
    ]);
    expect(lineViolations({ "./x.ts": "a\n" }, { "./x.ts": 700 })).toEqual([
      "./x.ts: allowlist entry is stale (1 <= 600)",
    ]);
    expect(deadLineEntries({ "./x.ts": "" }, { "./x.ts": 700, "./gone.ts": 700 })).toEqual([
      "./gone.ts",
    ]);
    expect(locationViolations({ "../../features/stray.ts": "" })).toEqual([
      "../../features/stray.ts",
    ]);
    expect(locationViolations({ "../../features/sftp/lib/x.ts": "" })).toEqual([]);
    expect(
      deadDirectionEntries(
        { "./x.ts": 'import { a } from "./y";' },
        ["./x.ts", "./missing.ts"],
      ),
    ).toEqual(["./x.ts", "./missing.ts"]);
  });
});
