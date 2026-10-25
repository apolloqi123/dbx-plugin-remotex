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
  "../../app/App.vue": 8620,
  "../../features/settings/components/SettingsDialog.vue": 929,
  "../../features/completion/lib/completion/fig/figSourceCore.ts": 737,
  "../../features/sftp/components/SftpTreePane.spec.ts": 1323,
};

/** 迁移期方向白名单；W1 结束后必须为空。 */
const DIRECTION_ALLOWLIST: string[] = [
  "../composables/useAppearanceCrossSurfaceSync.ts",
  "./pluginContext.ts",
  "./promptCorpus.spec.ts",
  "./preferencesMirror.ts",
  "./appearanceSync.ts",
  "./appearanceSync.spec.ts",
];

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

export function directionViolations(
  sources: Record<string, string>,
  allow: string[],
): string[] {
  const out: string[] = [];
  for (const [key, text] of Object.entries(sources)) {
    if (allow.includes(key)) continue;
    const shared = key.startsWith("./") || (key.startsWith("../") && !key.startsWith("../../"));
    const feature = key.startsWith("../../features/");
    if (!shared && !feature) continue;
    for (const match of text.matchAll(IMPORT_SPECIFIER)) {
      const spec = match[1];
      if (shared && /\/(?:features|app)\//.test(spec)) out.push(`${key} -> ${spec}`);
      if (feature && /\/app\//.test(spec)) out.push(`${key} -> ${spec}`);
    }
  }
  return out;
}

export function locationViolations(sources: Record<string, string>): string[] {
  const out: string[] = [];
  for (const key of Object.keys(sources)) {
    if (!key.startsWith("../../features/")) continue;
    const parts = key.slice("../../features/".length).split("/");
    if (parts.length < 2) continue;
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

  it("keeps feature files inside components/composables/lib", () => {
    const report = locationViolations(SOURCES);
    expect(report, `misplaced feature files:\n${report.join("\n")}`).toEqual([]);
  });

  it("detects synthetic violations instead of passing vacuously", () => {
    expect(
      directionViolations({ "./x.ts": 'import { a } from "../features/sftp/lib/a";' }, []),
    ).toEqual(["./x.ts -> ../features/sftp/lib/a"]);
    expect(
      directionViolations(
        { "../../features/sftp/lib/x.ts": 'import App from "../../../app/App.vue";' },
        [],
      ),
    ).toEqual(["../../features/sftp/lib/x.ts -> ../../../app/App.vue"]);
    expect(
      lineViolations({ "./x.ts": `${"x\n".repeat(601)}` }, {}),
    ).toEqual(["./x.ts: 601 > 600"]);
  });
});
