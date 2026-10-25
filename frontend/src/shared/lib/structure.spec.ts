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
  "../../app/App.vue": 8130,
  "../../features/settings/components/SettingsDialog.vue": 929,
  "../../features/completion/lib/completion/fig/figSourceCore.ts": 737,
  "../../features/sftp/components/SftpTreePane.spec.ts": 1323,
};

/** 迁移期方向白名单；W1 结束后必须为空，且每个条目都必须仍然违规。 */
const DIRECTION_ALLOWLIST: string[] = [];

function countLines(text: string): number {
  return text.replace(/\n$/, "").split("\n").length;
}

function isSpec(key: string): boolean {
  return key.endsWith(".spec.ts");
}

export function lineViolations(
  sources: Record<string, string>,
  allow: Record<string, number>,
): string[] {
  const out: string[] = [];
  for (const [key, text] of Object.entries(sources)) {
    const lines = countLines(text);
    const budget = isSpec(key) ? MAX_SPEC_LINES : MAX_SOURCE_LINES;
    const allowed = allow[key] ?? budget;
    if (lines > allowed) out.push(`${key}: ${lines} > ${allowed}`);
    if (allow[key] !== undefined && lines <= budget) {
      out.push(`${key}: allowlist entry is stale (${lines} <= ${budget})`);
    }
  }
  return out;
}

const IMPORT_SPECIFIER = /\b(?:from\s*|import\s*\(\s*|^import\s+)["']([^"']+)["']/gm;

function layerOf(key: string): "shared" | "feature" | null {
  if (key.startsWith("./") || (key.startsWith("../") && !key.startsWith("../../"))) {
    return "shared";
  }
  if (key.startsWith("../../features/")) return "feature";
  return null;
}

export function isUpwardImport(layer: "shared" | "feature", spec: string): boolean {
  return layer === "shared" ? /\/(?:features|app)\//.test(spec) : /\/app\//.test(spec);
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
  });

  it("keeps every source file within its line budget", () => {
    const report = lineViolations(SOURCES, LINE_ALLOWLIST);
    expect(report, `oversized sources:\n${report.join("\n")}`).toEqual([]);
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
    const APP_IMPORT = ["..", "..", "..", "app", "App.vue"].join("/");
    expect(
      directionViolations({ "./x.ts": `import { a } from "${FEATURES_IMPORT}";` }, []),
    ).toEqual([`./x.ts -> ${FEATURES_IMPORT}`]);
    expect(
      directionViolations(
        { "../../features/sftp/lib/x.ts": `import App from "${APP_IMPORT}";` },
        [],
      ),
    ).toEqual([`../../features/sftp/lib/x.ts -> ${APP_IMPORT}`]);
    expect(
      lineViolations({ "./x.ts": `${"x\n".repeat(601)}` }, {}),
    ).toEqual(["./x.ts: 601 > 600"]);
    expect(lineViolations({ "./x.ts": "a\n" }, { "./x.ts": 10 })).toEqual([
      "./x.ts: allowlist entry is stale (1 <= 600)",
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
