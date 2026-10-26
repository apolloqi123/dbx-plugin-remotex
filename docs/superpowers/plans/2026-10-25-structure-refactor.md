# 代码层级重构实施计划

> **状态：已完成（2026-10-26）** — T1–T31 全部完成，改动已落在 `refactor/structure-v2`。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** 把 `App.vue` 从 8,620 行降为纯组合根，领域逻辑回到 `features`，底座层依赖方向可被自动守卫约束，最终所有源文件落在行数预算内。

**Architecture:** 保持现有“options 对象 + 返回扁平 API”的 composable 约定，权威状态留在 `App.vue`，行为按终端运行时/IO/装饰/会话/SFTP/工作台六类下沉；模板拆为工具栏、终端舞台、侧栏、对话框宿主。目录上采用 `app`（组合根）→ `features`（领域）→ `shared`（底座）单向依赖，后端 `core` 成为叶子层。

**Tech Stack:** Vue 3 `<script setup>`、Vitest + happy-dom、Vite 8、TypeScript 6、Rust + Cargo、Tauri 宿主桥。

**Spec:** `docs/superpowers/specs/2026-10-25-structure-refactor-design.md`

## Global Constraints

- 行为冻结：不修改运行时行为、RPC 方法名/参数、i18n key、CSS 级联结果、依赖版本。
- 行数预算：非测试源 ≤600 行；spec/测试 ≤1,200 行；`App.vue` ≤600 行。
- 依赖方向：`app → features → shared`；`features ↛ app`；`shared ↛ features/app`；`backend core ↛ app/features/filesystem/sessions`。
- 根目录 `shared/frontend`、`shared/sdk` 是四插件共用 SDK，不移动、不改引用方式。
- `backend/src/filesystem/sftp/tree.rs`、`automation/task_config.rs`、`platform/job_object.rs` 内部不修改。
- `App.vue` 同一时刻只有一个写者；`features/terminal`、`features/sftp` 的文件迁移可并行，但每个文件只能由一个任务处理。
- 每个任务独立提交；不 push、不自行合并。
- 发现功能缺陷只记录，不在本计划内修复。
- 每个任务结束必须运行 `git status --short`，确认只包含本任务文件。

## 常用命令

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run <spec-path>
pnpm --dir frontend test
cargo test --manifest-path backend/Cargo.toml
scripts/test.sh --skip-host
git status --short
```

## 抽取约定（所有 App.vue 抽取任务共用）

1. 新模块使用具名导出 `export function useX(options: {...}) {...}`，返回扁平对象；不使用默认导出。
2. 从 `App.vue` 复制代码块时逐字保留实现与注释，只允许改两类内容：
   - 把原来直接引用的外部符号改为从 `options` 解构；
   - 把需要继续被模板/其他模块使用的符号放进返回值。
3. `App.vue` 只做三件事：删除原代码块、加 import、`const { ... } = useX({ ... })` 解构。模板绑定名不得改变。
4. 不在同一提交里改逻辑；发现逻辑问题时记录到任务备注，继续机械迁移。
5. 迁移后运行 `pnpm --dir frontend typecheck` 与列出的定向测试；类型错误只允许改接线。
6. 若 `App.vue` 行数变化，更新 `frontend/src/shared/lib/structure.spec.ts` 的 `LINE_ALLOWLIST`（只许调低）。

## 文件结构总览

```text
frontend/src/app/
  App.vue
  workbench/state/{useTerminalState,useSessionState,useSftpState,useLayoutState,useDialogState,useWorkbenchContext}.ts
  workbench/composables/{useWorkbenchState,useSplitDivider,useWorkbenchModals,useWorkbenchBootstrap,useWorkbenchPrefs,...}.ts
  workbench/lib/{preferencesMirror,appearanceSync,persistence}.ts
  components/{WorkbenchToolbar,SessionMenu,QuickCommandsMenu,RdpToolbarControls,AppearanceMenu,WorkbenchStage,TerminalStatusOverlays,TransferOverlays,WorkbenchSidePanel,WorkbenchDialogs}.vue

frontend/src/features/terminal/
  lib/{emulator,appearance,decorations,panels,transfer,behavior}/
  composables/{session,io,ui}/
frontend/src/features/sftp/
  lib/{entries,tree,transfer,editor,actions}/
  composables/{listing,tree,transfer,editor,actions}/
frontend/src/shared/            # 不再出现 features/app import

backend/src/sessions/ssh/exec/  # 原 core/exec
backend/src/core/               # 仅依赖 core 内部与 platform
backend/src/structure_tests.rs
```

---

## Wave 0：基线与守卫

### Task 1: 前端结构守卫

**Files:**
- Create: `frontend/src/shared/lib/structure.spec.ts`

**Interfaces:**
- Consumes: 无。
- Produces: `lineViolations(sources, allowlist)`、`directionViolations(sources, allowlist)`、`locationViolations(sources)`，供后续任务复用断言。

- [x] **Step 1: 写失败测试（守卫本体）**

创建 `frontend/src/shared/lib/structure.spec.ts`：

```ts
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
  "../lib/pluginContext.ts",
  "../lib/promptCorpus.spec.ts",
  "../lib/preferencesMirror.ts",
  "../lib/appearanceSync.ts",
  "../lib/appearanceSync.spec.ts",
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
```

- [x] **Step 2: 运行守卫，确认当前基线通过**

Run:

```bash
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

Expected: PASS。若行数/白名单不一致，按实际行数修正 `LINE_ALLOWLIST` 与 `DIRECTION_ALLOWLIST`，不删除断言。

- [x] **Step 3: 记录基线**

Run:

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

把测试文件数与用例数写入提交说明（例如 `frontend: 210 files / 2306 tests`）。

- [x] **Step 4: Commit**

```bash
git add frontend/src/shared/lib/structure.spec.ts
git commit -m "test: add frontend structure guards for size and layering"
```

### Task 2: 后端结构守卫

**Files:**
- Create: `backend/src/structure_tests.rs`
- Modify: `backend/src/main.rs`

**Interfaces:**
- Consumes: 无。
- Produces: `is_test_file(rel)`、`count_lines(text)` 与两条结构测试。

- [x] **Step 1: 写失败测试**

创建 `backend/src/structure_tests.rs`：

```rust
//! Structural guards: file budgets and layer direction.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_SOURCE_LINES: usize = 600;
const MAX_TEST_LINES: usize = 1_200;

const LINE_ALLOWLIST: &[(&str, usize)] = &[
    ("sessions/ssh/connection/open.rs", 718),
    ("sessions/local/runtime.rs", 671),
    ("sessions/ssh/tests.rs", 5535),
    ("features/mcp/tests.rs", 3772),
    ("features/import/tests.rs", 1692),
    ("core/model/manifest_contract_tests.rs", 1397),
];

fn count_lines(text: &str) -> usize {
    text.lines().count()
}

fn is_test_file(rel: &str) -> bool {
    rel.ends_with("tests.rs") || rel.ends_with("_tests.rs") || rel.contains("/tests/")
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("read entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext.to_string_lossy() == "rs") {
            out.push(path);
        }    }
}

#[test]
fn classifies_test_files() {
    assert!(is_test_file("sessions/ssh/tests.rs"));
    assert!(is_test_file("core/model/manifest_contract_tests.rs"));
    assert!(is_test_file("features/mcp/tests/tools.rs"));
    assert!(!is_test_file("sessions/ssh/mod.rs"));
    assert_eq!(count_lines("a\nb\n"), 2);
}

#[test]
fn sources_stay_within_line_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let mut violations = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("relative path")
            .to_string_lossy()
            .replace('\\', "/");
        let lines = count_lines(&fs::read_to_string(&path).expect("read source"));
        let budget = if is_test_file(&rel) { MAX_TEST_LINES } else { MAX_SOURCE_LINES };
        let allowed = LINE_ALLOWLIST
            .iter()
            .find(|(name, _)| *name == rel.as_str())
            .map(|(_, limit)| *limit)
            .unwrap_or(budget);
        if lines > allowed {
            violations.push(format!("{rel}: {lines} > {allowed}"));
        }
        if allowed != budget && lines <= budget {
            violations.push(format!("{rel}: allowlist entry is stale ({lines} <= {budget})"));
        }
    }
    assert!(
        violations.is_empty(),
        "oversized backend sources:\n{}",
        violations.join("\n")
    );
}

#[test]
fn core_does_not_depend_on_upper_layers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let banned = [
        "crate::app::",
        "crate::features::",
        "crate::filesystem::",
        "crate::sessions::",
    ];
    let mut violations = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("read core source");
        for needle in banned {
            if text.contains(needle) {
                violations.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "core layer must stay a leaf:\n{}",
        violations.join("\n")
    );
}
```

- [x] **Step 2: 注册测试模块**

在 `backend/src/main.rs` 的模块声明后加入：

```rust
#[cfg(test)]
mod structure_tests;
```

- [x] **Step 3: 运行，确认行数测试通过、core 测试当前失败**

Run:

```bash
cargo test --manifest-path backend/Cargo.toml structure_tests
```

Expected:
- `sources_stay_within_line_budget` PASS（白名单覆盖现状）。
- `core_does_not_depend_on_upper_layers` FAIL，并列出 ≥22 处 `core → features/sessions/filesystem`。这是 Wave 7 的待办，不得修改断言。

- [x] **Step 4: 临时标记 core 测试为已知失败**

在 `core_does_not_depend_on_upper_layers` 上加：

```rust
#[ignore = "Wave 7 removes core -> upper-layer dependencies"]
```

并在任务备注中记录失败清单。W7 最后一个任务移除 `#[ignore]`。

- [x] **Step 5: Commit**

```bash
git add backend/src/structure_tests.rs backend/src/main.rs
git commit -m "test: add backend structure guards for size and layering"
```

### Task 3: 前端文件迁移工具

**Files:**
- Create: `scripts/refactor/move-file.mjs`

**Interfaces:**
- Consumes: 无。
- Produces: `node scripts/refactor/move-file.mjs <from> <to>`，移动 `frontend/src` 内单文件并重写所有相对 import（含 `?raw`）。

- [x] **Step 1: 实现工具**

创建 `scripts/refactor/move-file.mjs`：

```js
#!/usr/bin/env node
// Moves one frontend source file and rewrites every relative import that points
// at it (and the moved file's own relative imports). Usage is path-based so the
// caller can run it repeatedly for grouped migrations.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const SRC = join(ROOT, "frontend", "src");
const [, , fromArg, toArg] = process.argv;

if (!fromArg || !toArg) {
  console.error("usage: node scripts/refactor/move-file.mjs <from> <to>");
  process.exit(2);
}

const from = resolve(ROOT, fromArg);
const to = resolve(ROOT, toArg);
const insideSrc = (path) => path === SRC || path.startsWith(`${SRC}${sep}`);

if (!insideSrc(from) || !insideSrc(to)) throw new Error("both paths must be inside frontend/src");
if (!existsSync(from)) throw new Error(`missing source: ${from}`);

const SPECIFIER = /(\bfrom\s*|\bimport\s*\(\s*|^\s*import\s+)(["'])(\.[^"']*)\2/gm;

function resolveRelative(importer, spec) {
  const clean = spec.replace(/[?#].*$/, "");
  const base = resolve(dirname(importer), clean);
  const candidates = [base, `${base}.ts`, `${base}.vue`, `${base}.tsx`, join(base, "index.ts")];
  return candidates.find((candidate) => existsSync(candidate) && statSync(candidate).isFile()) ?? null;
}

function rebase(source, oldImporter, newImporter, remap) {
  return source.replace(SPECIFIER, (match, prefix, quote, spec) => {
    const target = resolveRelative(oldImporter, spec);
    if (!target) return match;
    const nextTarget = remap(target);
    if (!nextTarget) return match;
    const clean = spec.replace(/[?#].*$/, "");
    const suffix = spec.slice(clean.length);
    const hadExtension = /\.[a-z]+$/.test(clean);
    let next = relative(newImporter, nextTarget).split(sep).join("/");
    if (!hadExtension) next = next.replace(/\.(ts|vue|tsx)$/, "");
    if (!next.startsWith(".")) next = `./${next}`;
    return `${prefix}${quote}${next}${suffix}${quote}`;
  });
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(ts|vue)$/.test(name)) out.push(path);
  }
  return out;
}

const moved = rebase(
  readFileSync(from, "utf8"),
  dirname(from),
  dirname(to),
  (target) => target,
);

mkdirSync(dirname(to), { recursive: true });
execFileSync("git", ["mv", from, to], { cwd: ROOT });
writeFileSync(to, moved);

let rewritten = 0;
for (const file of walk(SRC)) {
  if (file === to) continue;
  const before = readFileSync(file, "utf8");
  const after = rebase(before, dirname(file), dirname(file), (target) =>
    target === from ? to : null,
  );
  if (after !== before) {
    writeFileSync(file, after);
    rewritten += 1;
  }
}

console.log(`moved ${relative(ROOT, from)} -> ${relative(ROOT, to)}; rewrote ${rewritten} files`);
```

- [x] **Step 2: 用单个无害文件验证工具**

Run:

```bash
node scripts/refactor/move-file.mjs \
  frontend/src/features/sftp/lib/virtualWindow.ts \
  frontend/src/features/sftp/lib/tree/virtualWindow.ts
pnpm --dir frontend typecheck
node scripts/refactor/move-file.mjs \
  frontend/src/features/sftp/lib/tree/virtualWindow.ts \
  frontend/src/features/sftp/lib/virtualWindow.ts
git diff --stat
```

Expected: 两次移动后 `git diff --stat` 为空、`git status --short` 干净；typecheck PASS。若反向移动不是零 diff，修正脚本后重复本步（不要用 `git reset --hard`）。

- [x] **Step 3: Commit**

```bash
git add scripts/refactor/move-file.mjs
git commit -m "chore: add frontend file move codemod"
```

### Task 4: `app/composables` 领域归位

**Files:**
- Move (via Task 3 tool): `frontend/src/app/composables/*`
- Modify: `frontend/src/app/App.vue`（仅 import 路径）
- Modify: `frontend/src/app/hostKeyChallengeProtocol.spec.ts`（若工具未覆盖）

**Interfaces:**
- Consumes: `move-file.mjs`。
- Produces: `app/composables` 只剩工作台级模块；所有领域模块位于对应 `features/<domain>/{composables,lib}`。

- [x] **Step 1: 先建目录**

```bash
mkdir -p frontend/src/features/sftp/composables/{listing,tree,transfer,editor,actions}
mkdir -p frontend/src/features/terminal/composables/{session,io,ui}
mkdir -p frontend/src/features/metrics/composables
mkdir -p frontend/src/features/quick-commands/composables
mkdir -p frontend/src/features/serial/composables
mkdir -p frontend/src/app/workbench/{composables,lib}
```

- [x] **Step 2: 迁移 SFTP 领域模块**

```bash
node scripts/refactor/move-file.mjs frontend/src/app/composables/sftpRemotePaths.ts frontend/src/features/sftp/lib/paths/sftpRemotePaths.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpTreeWorkspace.ts frontend/src/features/sftp/composables/listing/useSftpTreeWorkspace.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpBookmarks.ts frontend/src/features/sftp/composables/actions/useSftpBookmarks.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpRowSelection.ts frontend/src/features/sftp/composables/actions/useSftpRowSelection.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpAttributes.ts frontend/src/features/sftp/composables/actions/useSftpAttributes.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpChmod.ts frontend/src/features/sftp/composables/actions/useSftpChmod.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSftpFolderActions.ts frontend/src/features/sftp/composables/actions/useSftpFolderActions.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useEditorOpenWith.ts frontend/src/features/sftp/composables/editor/useEditorOpenWith.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useEditorSplitPane.ts frontend/src/features/sftp/composables/editor/useEditorSplitPane.ts
```

- [x] **Step 3: 迁移终端/串口/指标/快速命令模块**

```bash
node scripts/refactor/move-file.mjs frontend/src/app/composables/useTerminalMenus.ts frontend/src/features/terminal/composables/ui/useTerminalMenus.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useTerminalSearch.ts frontend/src/features/terminal/composables/ui/useTerminalSearch.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useTerminalTooltip.ts frontend/src/features/terminal/composables/ui/useTerminalTooltip.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/usePromptHints.ts frontend/src/features/terminal/composables/ui/usePromptHints.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWallpaper.ts frontend/src/features/terminal/composables/ui/useWallpaper.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useAiAssistant.ts frontend/src/features/terminal/composables/ui/useAiAssistant.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useShellTabs.ts frontend/src/features/terminal/composables/ui/useShellTabs.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useOnlineSearch.ts frontend/src/features/terminal/composables/ui/useOnlineSearch.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useSerialUpload.ts frontend/src/features/serial/composables/useSerialUpload.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useDockMetrics.ts frontend/src/features/metrics/composables/useDockMetrics.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useQuickCommandPanel.ts frontend/src/features/quick-commands/composables/useQuickCommandPanel.ts
```

- [x] **Step 4: 迁移工作台级模块**

```bash
node scripts/refactor/move-file.mjs frontend/src/app/composables/usePluginEvents.ts frontend/src/app/workbench/composables/usePluginEvents.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/usePluginEvents.spec.ts frontend/src/app/workbench/composables/usePluginEvents.spec.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWorkbenchPrefs.ts frontend/src/app/workbench/composables/useWorkbenchPrefs.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWorkbenchViewPrefs.ts frontend/src/app/workbench/composables/useWorkbenchViewPrefs.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWorkbenchPanels.ts frontend/src/app/workbench/composables/useWorkbenchPanels.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWorkbenchToolbar.ts frontend/src/app/workbench/composables/useWorkbenchToolbar.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useWorkbenchSpawnTabs.ts frontend/src/app/workbench/composables/useWorkbenchSpawnTabs.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useLocalShellPreferences.ts frontend/src/features/terminal/composables/session/useLocalShellPreferences.ts
node scripts/refactor/move-file.mjs frontend/src/app/composables/useNotices.ts frontend/src/app/workbench/composables/useNotices.ts
```

- [x] **Step 5: 类型检查与测试**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

Expected: 全绿。若 `hostKeyChallengeProtocol.spec.ts` 仍指向旧路径，更新其 `?raw` import 到 `./workbench/composables/usePluginEvents.ts?raw`。

- [x] **Step 6: 运行结构守卫**

```bash
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

Expected: PASS（方向白名单尚未清空）。

- [x] **Step 7: Commit**

```bash
git add frontend/src
git commit -m "refactor(frontend): move domain composables out of the app layer"
```

### Task 5: `shared` 反向依赖清理

**Files:**
- Move: `frontend/src/shared/lib/preferencesMirror.ts` → `frontend/src/app/workbench/lib/preferencesMirror.ts`
- Move: `frontend/src/shared/lib/preferencesMirror.spec.ts` → `frontend/src/app/workbench/lib/preferencesMirror.spec.ts`
- Move: `frontend/src/shared/lib/appearanceSync.ts` → `frontend/src/app/workbench/lib/appearanceSync.ts`
- Move: `frontend/src/shared/lib/appearanceSync.spec.ts` → `frontend/src/app/workbench/lib/appearanceSync.spec.ts`
- Move: `frontend/src/shared/composables/useAppearanceCrossSurfaceSync.ts` → `frontend/src/app/workbench/composables/useAppearanceCrossSurfaceSync.ts`
- Move: `frontend/src/shared/lib/promptCorpus.spec.ts` → `frontend/src/features/terminal/lib/behavior/promptCorpus.spec.ts`
- Create: `frontend/src/shared/lib/connectionText.ts`
- Modify: `frontend/src/shared/lib/pluginContext.ts`
- Modify: `frontend/src/features/connections/lib/connectionInfo.ts`
- Modify: `frontend/src/shared/lib/structure.spec.ts`（清空 `DIRECTION_ALLOWLIST`）

**Interfaces:**
- Consumes: Task 3 工具。
- Produces: `shared` 不再依赖 `features`/`app`；`normalizeConnectionText` 从 `shared/lib/connectionText.ts` 导出，`connectionInfo.ts` 继续 re-export 以保持既有调用点。

- [x] **Step 1: 迁移组合型模块**

```bash
node scripts/refactor/move-file.mjs frontend/src/shared/lib/preferencesMirror.ts frontend/src/app/workbench/lib/preferencesMirror.ts
node scripts/refactor/move-file.mjs frontend/src/shared/lib/preferencesMirror.spec.ts frontend/src/app/workbench/lib/preferencesMirror.spec.ts
node scripts/refactor/move-file.mjs frontend/src/shared/lib/appearanceSync.ts frontend/src/app/workbench/lib/appearanceSync.ts
node scripts/refactor/move-file.mjs frontend/src/shared/lib/appearanceSync.spec.ts frontend/src/app/workbench/lib/appearanceSync.spec.ts
node scripts/refactor/move-file.mjs frontend/src/shared/composables/useAppearanceCrossSurfaceSync.ts frontend/src/app/workbench/composables/useAppearanceCrossSurfaceSync.ts
node scripts/refactor/move-file.mjs frontend/src/shared/lib/promptCorpus.spec.ts frontend/src/features/terminal/lib/behavior/promptCorpus.spec.ts
```

- [x] **Step 2: 下沉 `normalizeConnectionText`**

新建 `frontend/src/shared/lib/connectionText.ts`，逐字迁移 `connectionInfo.ts:11` 的现有实现（保留大小写不敏感的 `"null"`/`"undefined"` 判定与注释）：

```ts
export function normalizeConnectionText(value: unknown): string {
  if (typeof value !== "string") return "";
  const normalized = value.trim();
  return normalized && normalized.toLowerCase() !== "null" && normalized.toLowerCase() !== "undefined" ? normalized : "";
}
```

在 `frontend/src/features/connections/lib/connectionInfo.ts` 中删除原实现，改为：

```ts
export { normalizeConnectionText } from "../../../shared/lib/connectionText";
import { normalizeConnectionText } from "../../../shared/lib/connectionText";
```

在 `frontend/src/shared/lib/pluginContext.ts` 中把 import 改为：

```ts
import { normalizeConnectionText } from "./connectionText";
```

- [x] **Step 3: 清空方向白名单**

把 `structure.spec.ts` 的 `DIRECTION_ALLOWLIST` 改为空数组：

```ts
const DIRECTION_ALLOWLIST: string[] = [];
```

- [x] **Step 4: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

Expected: 全绿；结构守卫无方向违规。

- [x] **Step 5: Commit**

```bash
git add frontend/src
git commit -m "refactor(frontend): remove shared-to-feature dependencies"
```

### Task 6: `features/terminal` 子域分组

**Files:**
- Move: `frontend/src/features/terminal/lib/*` 按子域归位
- Move: `frontend/src/features/terminal/composables/*` 按子域归位
- Modify: `frontend/src/shared/lib/structure.spec.ts`（位置守卫已覆盖，无需改）

**Interfaces:**
- Consumes: Task 3 工具。
- Produces: 目录结构如下（每个文件只属于一组）：

```text
lib/emulator/     terminalAnchor terminalInput terminalInputQueue terminalLineColumns terminalLineModel
                  terminalModeQueries terminalOsc terminalOscChannels terminalPromptHints terminalPromptNewline
                  terminalBackpressure terminalWriteThrottle terminalWebkitInput terminalClickCursor
                  terminalDirectoryTracking terminalProtocolMatrix
lib/appearance/   terminalAppearance terminalFont terminalScheme terminalSchemeCatalog terminalBackground
                  terminalWebgl terminalLayout terminalGutter terminalZoom
lib/decorations/  actionLinksAddon actionLinksMatcher keywordHighlight terminalGhostSuggest terminalQuickSelect
lib/panels/       historyPanel commandHistory terminalCommandMarkers shellTabTitle
lib/transfer/     batchSend terminalTrzsz terminalZmodem
lib/behavior/     terminalHotkeys terminalInteraction dangerousCommands interactivePromptGuard terminalEnterCapture
                  terminalOutputText terminalReconnect terminalSplit promptCorpus.spec
composables/session/  useTerminalTransportWatch useLocalShellPreferences（后续新增 instance/writer）
composables/io/       后续新增 input/keys/clipboard
composables/ui/       useTerminalMenus useTerminalSearch useTerminalTooltip usePromptHints useWallpaper
                      useAiAssistant useShellTabs useOnlineSearch useZmodem useTrzsz useBatchSend
                      useCommandHistory useHistoryPanel useQuickSelect useHighlightRules useCommandDialog
                      useTerminalFontZoom useTerminalBell useTerminalMouse useTerminalAppearance
                      useSplitTerminal useSplitSearch useSplitClipboard useSplitTerminalRenderer
```

- [x] **Step 1: 批量移动 lib**

对上表每个文件执行一次（示例，重复到全部归位）：

```bash
node scripts/refactor/move-file.mjs frontend/src/features/terminal/lib/terminalAnchor.ts frontend/src/features/terminal/lib/emulator/terminalAnchor.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/lib/terminalAnchor.spec.ts frontend/src/features/terminal/lib/emulator/terminalAnchor.spec.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/lib/actionLinksAddon.ts frontend/src/features/terminal/lib/decorations/actionLinksAddon.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/lib/actionLinksMatcher.ts frontend/src/features/terminal/lib/decorations/actionLinksMatcher.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/lib/actionLinksMatcher.spec.ts frontend/src/features/terminal/lib/decorations/actionLinksMatcher.spec.ts
```

每移动 8–10 个文件跑一次：

```bash
pnpm --dir frontend typecheck
```

- [x] **Step 2: 批量移动 composables**

```bash
node scripts/refactor/move-file.mjs frontend/src/features/terminal/composables/useTerminalTransportWatch.ts frontend/src/features/terminal/composables/session/useTerminalTransportWatch.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/composables/useTerminalTransportWatch.spec.ts frontend/src/features/terminal/composables/session/useTerminalTransportWatch.spec.ts
node scripts/refactor/move-file.mjs frontend/src/features/terminal/composables/useSplitTerminal.ts frontend/src/features/terminal/composables/ui/useSplitTerminal.ts
```

其余 UI 型 composable 按接口块归属表移动到 `composables/ui/`；`useLocalShellPreferences` 已在 Task 4 移到 `composables/session/`。

- [x] **Step 3: 全量验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

- [x] **Step 4: 递归化结构守卫的 composable 扫描**

`frontend/src/app/workbench.spec.ts` 的 `import.meta.glob("../**/composables/*.ts", ...)` 改为 `"../**/composables/**/*.ts"`，否则子域分组后终端 composable 会从弹层结构守卫中静默消失。同步检查 `scripts/refactor/move-file.mjs` 打印的 glob 审计清单。

- [x] **Step 5: 写子域边界说明**

新建 `frontend/src/features/terminal/README.md`，用一张表列出 `lib/emulator|appearance|decorations|panels|transfer|behavior` 与 `composables/session|io|ui` 的职责，并写明：跨子域只能经 `lib` 的具名导出，不得直接引用其他子域的内部文件。

- [x] **Step 6: Commit**

```bash
git add frontend/src
git commit -m "refactor(terminal): group modules by subdomain"
```

### Task 7: `features/sftp` 子域分组

**Files:**
- Move: `frontend/src/features/sftp/lib/*` 与 `composables/*` 按子域归位

**Interfaces:**
- Consumes: Task 3 工具。
- Produces:

```text
lib/entries/   sftpEntries sftpFileFilters sftpName sftpErrors
lib/paths/     remotePathInput sftpPathHistory sftpBookmarks sftpRemotePaths
lib/tree/      sftpFileTree virtualWindow dropEntries fileRowKeydown
lib/transfer/  folderUpload folderArchive folderOverwrite sanitizedDownload sftpFolderDownload
               downloadPrefs hostFileDrop hostDragMapping
lib/editor/    editorRules editorTheme textSniff jsonPreview watchEdits
lib/actions/   sftpContextMenu sftpDetails sftpRename
composables/listing/  useSftpNavigation useSftpListLayout useSftpTreeWorkspace useSftpPathHistory
composables/tree/     useTreeVirtualScroll useTreeDetailsCard useTreeFileDrop
composables/transfer/ useUploadChain useUploadDuplicates useFolderUpload useHostFileDrag
                      useSftpDownload useSftpArchive
composables/editor/   useEditorTabs useEditorOpenWith useEditorSplitPane useFilePreview useExternalEdits
composables/actions/  useSftpChmod useSftpDelete useSftpRename useSftpCreateDirectory
                      useSftpAttributes useSftpBookmarks useSftpSelectionClipboard useSymlink
                      useSftpFolderActions useSftpRowSelection useSftpDialogs useSftpDropUpload
```

- [x] **Step 1: 移动 lib 并每批验证**

```bash
node scripts/refactor/move-file.mjs frontend/src/features/sftp/lib/sftpEntries.ts frontend/src/features/sftp/lib/entries/sftpEntries.ts
pnpm --dir frontend typecheck
```

重复到上表全部归位。

- [x] **Step 2: 移动 composables 并每批验证**

```bash
node scripts/refactor/move-file.mjs frontend/src/features/sftp/composables/useSftpNavigation.ts frontend/src/features/sftp/composables/listing/useSftpNavigation.ts
pnpm --dir frontend typecheck
```

表中尚不存在的模块（`useSftpDialogs`、`useSftpDropUpload`、`useSftpPreview`）由 Task 14b 直接创建到目标目录，本任务不移动。

- [x] **Step 3: 全量验证**

```bash
pnpm --dir frontend test
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

- [x] **Step 4: 写子域边界说明**

新建 `frontend/src/features/sftp/README.md`，列出 `lib/entries|paths|tree|transfer|editor|actions` 与 `composables/listing|tree|transfer|editor|actions` 的职责，并注明 `listing` 是唯一允许发起 `sftp/list` 的位置。

- [x] **Step 5: Commit**

```bash
git add frontend/src
git commit -m "refactor(sftp): group modules by subdomain"
```

---

## Wave 2：App.vue 低风险抽取

### Task 8: 终端装饰引擎

**Files:**
- Create: `frontend/src/features/terminal/composables/ui/useTerminalHighlight.ts`
- Create: `frontend/src/features/terminal/composables/ui/useTerminalActionLinks.ts`
- Create: `frontend/src/features/terminal/composables/ui/useTerminalGutter.ts`
- Create: `frontend/src/features/terminal/composables/ui/useTerminalDecorations.ts`
- Modify: `frontend/src/app/App.vue`（删除三组实现，改为调用）
- Modify: `frontend/src/shared/lib/structure.spec.ts`（调低 App.vue 白名单）

**Interfaces:**
- Consumes: `terminal`/`fitAddon` getter、`terminalHost`、`highlightEnabled`/`compiledHighlightRules`/`highlightDisplayMode`、`gutterSettings`、`actionLinksSettings`、`showActionLinkHintAt`/`hideActionLinkHint`、`handleDockerOpenInTerminal`。
- Produces:
  - `useTerminalHighlight(options)` → `{ attachHighlightRender, detachHighlightRender, rescanHighlightViewport, scheduleHighlightScan }`
  - `useTerminalActionLinks(options)` → `{ actionLinkHint, attachActionLinks, detachActionLinks, handleActionLinkActivate, showActionLinkHintAt, hideActionLinkHint, scheduleActionLinkScan, updateActionLinksSettings }`
  - `useTerminalGutter(options)` → `{ gutterRows, gutterCellHeight, gutterPaneVisible, gutterWidth, gutterVisible, gutterPaneStyle, isGutterActive, attachGutterListeners, detachGutterListeners, scheduleGutterRecompute, resetGutterTimestamps, stampGutterWrittenRows, updateGutterSettings }`
  - `useTerminalDecorations(options)` → 合并上述三者返回值，供 `useTerminalWorkbench` 聚合使用。

- [x] **Step 1: 按抽取约定迁出高亮引擎**

从 `App.vue` 迁出以下符号及 4,624–4,776 的实现：

```text
keywordTextColorActive highlightRenderDisposable highlightDecorationsByRow
highlightDecorationCount highlightScanScheduled highlightLastScanAt highlightPendingRange
clearHighlightDecorations attachHighlightRender detachHighlightRender
rescanHighlightViewport scheduleHighlightScan runHighlightScan scanHighlightRange
watch([compiledHighlightRules, highlightDisplayMode])
```

`HIGHLIGHT_DECORATION_LIMIT`、`HIGHLIGHT_SCAN_MIN_INTERVAL_MS` 一并迁入模块。

- [x] **Step 2: 迁出动作链接引擎**

迁出 4,791–4,989 的 `actionLink*` 状态与函数；`actionLinkHint` 放返回值供模板使用。

- [x] **Step 3: 迁出 gutter**

迁出 4,989–5,160 的 `gutter*` 状态与函数；`GUTTER_SUSPEND_PENDING_BYTES` 一并迁入。

- [x] **Step 4: 写聚合器**

`useTerminalDecorations.ts`：

```ts
export function useTerminalDecorations(options: TerminalDecorationsOptions) {
  const highlight = useTerminalHighlight(options);
  const actionLinks = useTerminalActionLinks(options);
  const gutter = useTerminalGutter(options);
  return { ...highlight, ...actionLinks, ...gutter };
}
```

- [x] **Step 5: 接线**

`App.vue` 中删除原块，加入：

```ts
const {
  attachHighlightRender, detachHighlightRender, rescanHighlightViewport, scheduleHighlightScan,
} = useTerminalHighlight({ terminal: () => terminal, highlightEnabled, compiledHighlightRules, highlightDisplayMode });
const {
  actionLinkHint, attachActionLinks, detachActionLinks, handleActionLinkActivate,
  showActionLinkHintAt, hideActionLinkHint, scheduleActionLinkScan, updateActionLinksSettings,
} = useTerminalActionLinks({ terminal: () => terminal, actionLinksSettings, t });
const {
  gutterRows, gutterCellHeight, gutterPaneVisible, gutterWidth, gutterVisible, gutterPaneStyle,
  isGutterActive, attachGutterListeners, detachGutterListeners, scheduleGutterRecompute,
  resetGutterTimestamps, stampGutterWrittenRows, updateGutterSettings,
} = useTerminalGutter({ terminal: () => terminal, gutterSettings, session });
```

保留模板中同名绑定。

- [x] **Step 6: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/lib/keywordHighlight.spec.ts src/features/terminal/lib/actionLinksMatcher.spec.ts src/features/terminal/lib/terminalGutter.spec.ts
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

Expected: 全绿；`App.vue` 行数下降约 500 行。

- [x] **Step 7: 浏览器冒烟**

运行 `pnpm --dir frontend dev`，打开 `visual.html`，确认：终端输出着色、动作链接按住 Cmd/Ctrl 悬停提示、行号/时间戳开关即时生效、控制台无错误。

- [x] **Step 8: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract terminal decoration engines"
```

### Task 9: 命令标记与剪贴板

**Files:**
- Create: `frontend/src/features/terminal/composables/session/useCommandMarker.ts`
- Create: `frontend/src/features/terminal/composables/io/useTerminalClipboard.ts`
- Modify: `frontend/src/app/App.vue`
- Modify: `frontend/src/shared/lib/structure.spec.ts`

**Interfaces:**
- Consumes: `terminal`/`terminalCopyCache`/`clipboardDeps` 回调、`commandMarkerParser`、`commandMarkerTimer`。
- Produces:
  - `useCommandMarker(options)` → `{ commandMarker, commandMarkerElapsed, commandMarkerDetails, resetCommandMarker, startCommandMarkerTick, stopCommandMarkerTick, applyCommandMarker }`
  - `useTerminalClipboard(options)` → `{ copyTextToClipboard, copyTerminalSelection, pasteTerminal, interceptTerminalPaste, sendConfirmedPaste, confirmRiskyPaste, resolvePasteConfirm, selectAllTerminal, clearTerminal, pasteConfirm }`

- [x] **Step 1: 迁出命令标记**

迁出 1,335–1,348 与 3,200–3,304：`commandMarker`、`commandMarkerElapsed`、`commandMarkerTimer`、`stopCommandMarkerTick`、`startCommandMarkerTick`、`resetCommandMarker`、`applyCommandMarker`、`commandMarkerDetails`。

- [x] **Step 2: 迁出剪贴板与危险粘贴**

迁出 `copyTextToClipboard`、`clipboardDeps`、`copyTerminalSelection`、`pasteTerminal`、`interceptTerminalPaste`、`sendConfirmedPaste`、`pasteConfirmAskChain`、`confirmRiskyPaste`、`resolvePasteConfirm`、`selectAllTerminal`、`clearTerminal` 及 `pasteConfirm` ref。OSC52 handler 仍留在 `createTerminal`，只通过 options 传入 `terminalCopyCache`。

- [x] **Step 3: 接线并验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/lib/terminalInteraction.spec.ts src/features/terminal/lib/dangerousCommands.spec.ts src/features/terminal/lib/terminalCommandMarkers.spec.ts
```

浏览器冒烟：复制选中、右键粘贴、危险命令二次确认、命令标记计时。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract command marker and clipboard logic"
```

### Task 10: 工作台持久化与分隔条

**Files:**
- Create: `frontend/src/app/workbench/composables/useWorkbenchPersistence.ts`
- Create: `frontend/src/app/workbench/composables/useSplitDivider.ts`
- Modify: `frontend/src/app/App.vue`
- Modify: `frontend/src/shared/lib/structure.spec.ts`

**Interfaces:**
- Consumes: 布局 refs（`splitRatio`、`dockerSplitRatio`、`paneOrder`、`paneDirection`、`sftpPaneOpen`、`followDirectory`、`sudoMode`、`visibleColumns`、`sftpShowHidden`、`sort`）、`initialState`、`applyWorkbenchLayoutPrefs`、`writeWorkbenchState`。
- Produces:
  - `useWorkbenchPersistence(options)` → `{ restoreUiState, persistState, writeWorkbenchState, applyWorkbenchLayoutPrefs, initialState }`
  - `useSplitDivider(options)` → `{ startDividerDrag, onDividerKeydown, nudgeSplitDivider }`

- [x] **Step 1: 迁出持久化 2,085–2,269**

将 `initialState`、`applyWorkbenchLayoutPrefs`、`restoreUiState`、`writeWorkbenchState`、`persistThrottle`、`persistState` 与 `WORKBENCH_LAYOUT_KEY` 等常量迁入；`persistState` 依赖的 `session`/`terminalState`/布局 refs 全部走 options。

- [x] **Step 2: 迁出分隔条 5,311–5,401**

迁出 `startDividerDrag`、`DIVIDER_KEY_STEP_PX`、`onDividerKeydown`、`nudgeSplitDivider`；`splitRatio`/`dockerSplitRatio` 仍留在 App，通过 getter/setter 注入。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/app/lib/workbenchLayout.spec.ts src/app/lib/persistThrottle.spec.ts
```

浏览器冒烟：拖拽分隔条、键盘方向键微调、刷新后布局恢复。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract workbench persistence and divider"
```

### Task 11: 模态与焦点仲裁

**Files:**
- Create: `frontend/src/app/workbench/composables/useWorkbenchModals.ts`
- Modify: `frontend/src/app/App.vue`
- Modify: `frontend/src/app/workbench.spec.ts`（结构守卫联合源）

**Interfaces:**
- Consumes: `modalOpenCount` 依赖的弹窗 refs、`pickModalFocusTarget`、`ghostClickGuard`、`closeToolbarPopovers`、键盘动作回调表。
- Produces: `useWorkbenchModals(options)` → `{ modalOpenStates, modalOpenCount, topModalContainer, focusTopModal, onDocumentMouseDownCapture, onDocumentClickCapture, trackStableFocus, onDocumentKeydown }`

- [x] **Step 1: 迁出 6,275–6,535**

迁移 `modalTriggerStack`、`lastStableFocus`、`ghostClickGuard`、三个 document 监听、`modalOpenStates`、`modalOpenCount`、`topModalContainer`、`focusTopModal`、`watch(modalOpenCount)`、`onDocumentKeydown`。键盘分支保留原有顺序与优先级。

- [x] **Step 2: 更新结构守卫**

`frontend/src/app/workbench.spec.ts` 的弹层接入扫描若只读 `App.vue`，把 `appScript` 改为 `App.vue + app/workbench/composables/*.ts` 联合源；断言内容不变。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：设置弹窗 Esc 关闭、焦点回到触发元素、快捷键不与终端输入冲突。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract modal focus and key arbitration"
```

---

## Wave 3：会话生命周期抽取

### Task 12: 本地 shell / Telnet / 串口

**Files:**
- Create: `frontend/src/features/terminal/composables/session/useLocalShellSession.ts`
- Create: `frontend/src/features/telnet/composables/useTelnetWorkbench.ts`
- Create: `frontend/src/features/serial/composables/useSerialWorkbench.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `session` 表、`sendTerminalBytes`、`terminalState`/`terminalError`、`showNotice`/`showError`、`localShellPrefs` 回调。
- Produces:
  - `useLocalShellSession(options)` → `{ localSession, localState, localExitCode, localOpenConfirmOpen, isLocalMode, localShellRestored, lastCwd, recentCommands, startLocalTerminal, closeLocalTerminal, toggleLocalTerminal, requestLocalTerminal, confirmLocalTerminal, restartLocalTerminal, rerunLocalCommand, dismissRestoredLocalShell, reattachLocalSession }`
  - `useTelnetWorkbench(options)` → `{ telnetSession, telnetDialogOpen, telnetState, telnetError, isTelnetMode, telnetTarget, startTelnetSession, closeTelnetSession, startTelnetFromConnection }`
  - `useSerialWorkbench(options)` → `{ serialSession, serialDialogOpen, serialState, serialError, serialBinaryInput, isSerialMode, serialTarget, startSerialSession, closeSerialSession, startSerialFromConnection }`

- [x] **Step 1: 迁出本地 shell（4,057–4,146、4,384–4,500）**

保持 `watch(localOpenConfirmOpen)`、`pendingLocalShellOverride` 与重启链原样。

- [x] **Step 2: 迁出 Telnet（4,146–4,204、4,212–4,224）**

- [x] **Step 3: 迁出串口（4,294–4,334）**

- [x] **Step 4: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：底部 dock 打开本地终端、切换 shell、重启；Telnet 连接/断开；串口连接/断开/二进制输入开关。

- [x] **Step 5: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract local, telnet and serial session lifecycles"
```

### Task 13: VNC / RDP 会话接线

**Files:**
- Create: `frontend/src/features/remote-desktop/composables/useVncWorkbench.ts`
- Create: `frontend/src/features/remote-desktop/composables/useRdpWorkbench.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: 已有 `useVncSession`、`useRdpSession`/`useRdpWorkbench` 返回对象、`connectionProtocol`、`showError`。
- Produces:
  - `useVncWorkbench(options)` → `{ startVncFromConnection, vncConnectHandlers }`
  - `useRdpWorkbench(options)` → `{ routeRdpConnection, connectViaRdpDialog, startRdpFromConnection, rdpClosedTitle, rdpScaleMode, rdpClipboardChunks }`

- [x] **Step 1: 迁出 `startVncFromConnection`（4,224–4,238）与其对话框接线**

- [x] **Step 2: 迁出 RDP 路由（3,699–3,717、4,255–4,289）**

`rdpClosedTitle`、`rdpScaleMode` 若已被 `useRdpWorkbench` 管理，则直接复用其返回值，不重复声明。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/remote-desktop/composables/useVncSession.spec.ts
```

浏览器冒烟：VNC/RDP 连接、缩放切换、断线重连提示。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract vnc and rdp workbench wiring"
```

### Task 14: SSH 连接生命周期

**Files:**
- Create: `frontend/src/features/connections/composables/useSshConnection.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminalState`、`terminalError`、`session`、`openRetryAttempt`、`transportReuseState`、`connectLog`、`hostKeyPrompt`、`resolveHostKey`、`requestHostReopenConnection`、`afterSessionConnected` 回调。
- Produces: `useSshConnection(options)` → `{ openSession, attachSession, closeSession, afterSessionConnected, resolveStoredConnectionProtocol, reconnectNow, reconnectPending, resolveHostKey, requestHostReopenConnection }`

- [x] **Step 1: 迁出会话打开/附着/关闭（3,666–4,057）**

保持重试、传输复用、回执日志与错误分类的调用顺序不变。

- [x] **Step 2: 迁出 host reopen 与 host key（4,500–4,598）**

`resolveHostKey` 的 `connection/challenge/resolve` payload 必须逐字保留；`hostKeyChallengeProtocol.spec.ts` 若仍扫描 App.vue，改为扫描新文件。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：连接、断线重连、host key 接受/拒绝、记住主机密钥。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract ssh connection lifecycle"
```

### Task 14b: SFTP 工作区、对话框与拖放上传

**Files:**
- Create: `frontend/src/features/sftp/composables/listing/useSftpWorkspace.ts`
- Create: `frontend/src/features/sftp/composables/actions/useSftpDialogs.ts`
- Create: `frontend/src/features/sftp/composables/transfer/useSftpDropUpload.ts`
- Create: `frontend/src/features/sftp/composables/editor/useSftpPreview.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: 已存在的 `useSftpNavigation`、`useSftpDelete`、`useSftpRename`、`useSftpCreateDirectory`、`useSftpFolderActions`、`useUploadChain`、`useEditorTabs`；`session`、`sudoMode`、`showNotice`/`showError`、`terminalCwd`。
- Produces:
  - `useSftpWorkspace(options)` → `{ entries, currentPath, selectedPath, selectedUris, lastClickedUri, loadingFiles, sftpListingLoaded, sftpHomePath, sftpTreeRoot, sftpTreeRows, sftpTreeEntries, sftpTreeTotalRows, loadedTreeEntries, selectedEntry, selectedEntries, loadDirectory, refreshSftpHomePath, onSftpSelection, commitRenameEntry }`
  - `useSftpDialogs(options)` → `{ deleteTarget, batchDeleteOpen, batchDeleteSubmitting, mkdirTargetDir, newFileTargetDir, newFileDialog, newFileDraft, newFileSubmitting, sftpClipboard, pasteBusy, sftpErrorKey, openNewFileDialog, createNewFile }`
  - `useSftpDropUpload(options)` → `{ dragActive, sftpDragDropTarget, dropUploadPrompt, onDrop, onSftpDragEnter, onSftpDragLeave, resolveDropUpload, onSftpTreeDrop }`
  - `useSftpPreview(options)` → `{ previewOpen, previewEntry, previewEditableAllowed, closePreview, savePreview }`

- [x] **Step 1: 迁出 SFTP 列表/选择状态（874–1,114、5,188–5,254）**

`useSftpWorkspace` 负责组合已有导航/选择/树 composable；列表仍以 `entries/currentPath` 为唯一出口，`loadDirectory` 不复制实现。

- [x] **Step 2: 迁出 SFTP 对话框状态（874–1,114、5,530–5,657）**

`useSftpDialogs` 持有删除/批量/新建/粘贴/错误态；`openNewFileDialog`、`createNewFile` 逐字迁移。

- [x] **Step 3: 迁出本机拖放上传（5,657–5,759）**

`useSftpDropUpload` 负责 `dragenter/dragover/dragleave/drop`、目标目录解析、冲突策略提示与 `useUploadChain` 调用。

- [x] **Step 4: 迁出预览与保存（5,401–5,530）**

- [x] **Step 5: 更新源码扫描守卫**

`frontend/src/app/workbench.spec.ts` 中依赖 `watch(terminalCwd)`、`shouldFollowTabCwd`、`activeTerminalPaneId.value = paneId` 的源码断言改为扫描 `App.vue + features/sftp/composables/listing/useSftpWorkspace.ts` 联合源；断言意图不变。

- [x] **Step 6: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/sftp/composables src/features/sftp/lib
```

浏览器冒烟：目录切换、选中/Shift 多选、新建文件夹/文件、删除/批量删除、重命名、拖本机文件到目录行、冲突策略、预览保存。

- [x] **Step 7: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract sftp workspace, dialogs and drop upload"
```

---

## Wave 4：终端内核抽取

### Task 15: xterm 实例与尺寸

**Files:**
- Create: `frontend/src/features/terminal/composables/session/useTerminalInstance.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminalHost`、`terminalAppearance`、`terminalFontOverride`、`terminalBehavior`、`hostTerminalFontFamily`、`terminalTheme`、`openTerminalLink`、`settingsPreferences`、`wallpaperActive`、装饰挂载回调。
- Produces: `useTerminalInstance(options)` → `{ terminal, fitAddon, searchAddon, webglRenderer, createTerminal, attachTerminalHost, detachTerminalHost, ensureTerminalResizeObserver, scheduleFit, fitTerminalDimensions, settledTerminalDimensions, terminalCursorPosition, setWebglEnabled, webglRecoveryOptions }`

- [x] **Step 1: 迁出 2,269–2,511 与 3,043–3,200**

包括 `createTerminal`、`attachTerminalHost`、`detachTerminalHost`、`ensureTerminalResizeObserver`、`watch(terminalHost)`、`setWebglEnabled`、`scheduleFit`、`fitTerminalDimensions`、`nextAnimationFrame`、`settledTerminalDimensions`、`terminalCursorPosition`。

- [x] **Step 2: 保留扩展点**

`highlightEnabled`/`actionLinksEnabled`/`isGutterActive` 的挂载改为 options 回调 `onReady`，由 App 在 composable 返回后调用，避免循环依赖。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/lib/terminalFont.spec.ts src/features/terminal/lib/terminalAppearance.spec.ts src/features/terminal/lib/terminalWebgl.spec.ts
```

浏览器冒烟：终端首屏渲染、窗口缩放 fit、WebGL 开关、壁纸模式、分屏增删后终端迁移。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract xterm instance lifecycle"
```

### Task 16: 输出管线

**Files:**
- Create: `frontend/src/features/terminal/composables/session/useTerminalWriter.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminal`、`session`、`pendingTerminalFrames`、`replayInFlight`、`terminalTransportWatch`、`commandMarker`、`directoryParser`、`promptNewline`、`terminalWriteThrottle`、`zmodemBusy`。
- Produces: `useTerminalWriter(options)` → `{ writeTerminalOutput, dispatchTerminalOutput, handleBinary, drainTerminalFrames, settleOutputChunk, onOutputGateRelease, outputInFlightBytes }`

- [x] **Step 1: 迁出输出 gate 与写节流（1,622–1,737 中的输出部分）**

- [x] **Step 2: 迁出 3,304–3,632**

`writeTerminalOutput`、`dispatchTerminalOutput`、`handleBinary`、`drainTerminalFrames` 逐字迁移；帧序号 `lastSequence`、`pendingTerminalFrames`、`replayNoProgress` 作为 options 注入的响应式容器与 getter/setter。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/lib/terminalBackpressure.spec.ts src/features/terminal/lib/terminalWriteThrottle.spec.ts src/features/terminal/lib/terminalReconnect.spec.ts
```

浏览器冒烟：长输出不卡顿、断线重连后无重复输出、`ssh-transport-disconnected` 状态帧正确显示。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract terminal output pipeline"
```

### Task 17: 输入、行跟踪与按键路由

**Files:**
- Create: `frontend/src/features/terminal/composables/io/useTerminalInput.ts`
- Create: `frontend/src/features/terminal/composables/io/useTerminalKeys.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminalInputQueue`、`serialInputQueue`、`enterLineTracker`、`overlay` 仲裁函数、历史/ghost/补全状态与动作。
- Produces:
  - `useTerminalInput(options)` → `{ sendTerminalBytes, routeTerminalData, trackPendingInput, captureEnterLine, readEchoTextBeforeCursor, enterLineTracker }`
  - `useTerminalKeys(options)` → `{ handleTerminalKey, priority, terminalOverlayKeyOwners, panelClaims, fourKeyClaims, keyEventClaimKey, overlayOwnsKey }`

- [x] **Step 1: 迁出输入与行跟踪（1,688–1,737、2,894–3,043）**

- [x] **Step 2: 迁出按键路由（2,511–2,894）**

保持 `resolveTerminalInputRoute`、overlay 优先级、历史/补全分支顺序不变。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/lib/terminalInputQueue.spec.ts src/features/terminal/lib/terminalEnterCapture.spec.ts src/features/terminal/lib/terminalHotkeys.spec.ts src/shared/lib/overlayArbiter.spec.ts
```

浏览器冒烟：方向键历史、Tab 补全、Ctrl+F 搜索、远端交互提示时不抢键、分屏内按键落到活动窗格。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract terminal input and key routing"
```

### Task 18: 传输探测与重连接线

**Files:**
- Create: `frontend/src/features/terminal/composables/session/useTerminalReconnect.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminalTransportWatch`、`reconnectPending`、`reconnectAttempt`、`terminalReconnect` helpers、`openSession`。
- Produces: `useTerminalReconnect(options)` → `{ terminalTransportWatch, reconnectPending, reconnectNow, scheduleReconnect, cancelReconnect }`

- [x] **Step 1: 迁出 3,632–3,666 与分散的重连定时器状态**

- [x] **Step 2: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/terminal/composables/session/useTerminalTransportWatch.spec.ts src/features/terminal/lib/terminalReconnect.spec.ts
```

- [x] **Step 3: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract terminal reconnect wiring"
```

### Task 18b: 工作台启动、事件订阅与宿主拖拽

**Files:**
- Create: `frontend/src/app/workbench/composables/useWorkbenchBootstrap.ts`
- Create: `frontend/src/app/workbench/composables/useHostDragBridge.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `initialize` 依赖的 open/restore/update 动作、`usePluginEvents` 解码器、`useTerminalTooltip`、`settingsPreferences`、`hostContext`。
- Produces:
  - `useWorkbenchBootstrap(options)` → `{ initialize, findReattachSession, closeOrphanSplitSessions, reattachProtocolSession, syncMcpSettingsMirror, mount, unmount }`
  - `useHostDragBridge(options)` → `{ applyHostDragState, onHostDragMessage }`

- [x] **Step 1: 迁出主机拖拽桥（6,569–6,593）**

保持 `dbx-plugin:file-drag` 消息形状与 `dragActive`/`terminalDragActive` 语义不变。

- [x] **Step 2: 迁出初始化（6,593–6,808）**

`initialize`、`resolveStoredConnectionProtocol` 的启动分支、`findReattachSession`、`closeOrphanSplitSessions`、`reattachProtocolSession` 移入；启动顺序（waitForHostApi → hydrate → open/restore → subscribe）不得调整。

- [x] **Step 3: 迁出事件分派与 MCP 镜像（6,109–6,146、6,808–6,852）**

`handleEvent` 保留 `usePluginEvents` 的解码结果与各分支顺序。

- [x] **Step 4: 迁出挂载/卸载（6,852–6,955）**

`onMounted`/`onBeforeUnmount` 改为模块返回的 `mount`/`unmount`，由 App.vue 调用；清理顺序（计时器 → 订阅 → terminal dispose）保持不变。

- [x] **Step 5: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：冷启动连接、恢复上次会话、重新附着本地/协议会话、宿主拖拽高亮、关闭视图无泄漏（控制台无 error、无重复订阅）。

- [x] **Step 6: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract workbench bootstrap and host drag bridge"
```

---

## Wave 5：模板组件化与 CSS

### Task 19: 工具栏拆分

**Files:**
- Create: `frontend/src/app/components/WorkbenchToolbar.vue`
- Create: `frontend/src/app/components/SessionMenu.vue`
- Create: `frontend/src/app/components/QuickCommandsMenu.vue`
- Create: `frontend/src/app/components/RdpToolbarControls.vue`
- Create: `frontend/src/app/components/AppearanceMenu.vue`
- Move CSS: `frontend/src/app/styles/global/toolbar.css` 中对应选择器 → 各组件 `<style scoped>`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: 现有工具栏 props/actions；每个组件用显式 props + emits，不 inject 全局上下文。
- Produces: `App.vue` 中 `<WorkbenchToolbar>` 替换 6,995–7,453 的工具栏 DOM。

- [x] **Step 1: 先拆 `SessionMenu.vue`**

把 7,093–7,227 的会话菜单（本地/远程/复制/命令会话、shell 选择、环境变量、连接列表）迁入；props 覆盖 `localShells`、`localShellPref`、`localShellEnvPref`、`dockConnections`；emits 覆盖原 `@click` 动作。

- [x] **Step 2: 拆 `QuickCommandsMenu.vue`（7,247–7,336）**

- [x] **Step 3: 拆 `RdpToolbarControls.vue`（7,039–7,092）**

- [x] **Step 4: 拆 `AppearanceMenu.vue` 与剩余 popover**

- [x] **Step 5: 组合 `WorkbenchToolbar.vue`**

外壳、身份区、主操作位、panel 模式左区（7,361–7,405）留在这一层。

- [x] **Step 6: CSS 迁移与验证**

把 `toolbar.css` 中仅作用于这些 DOM 的规则移入各组件 scoped 样式；依赖 `.workbench` 祖先的规则保留在全局。

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器截图对比重构前后的工具栏（1440×900 与 720×900）：布局、间距、下拉位置、z-index 一致。

- [x] **Step 7: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): split workbench toolbar into components"
```

### Task 20: 终端舞台与浮层

**Files:**
- Create: `frontend/src/app/components/WorkbenchStage.vue`
- Create: `frontend/src/app/components/TerminalStatusOverlays.vue`
- Create: `frontend/src/app/components/TransferOverlays.vue`
- Move CSS: `panes.css` 对应规则
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: `terminalHost` ref 通过 `defineExpose`/模板 ref 回传；状态 props 分组。
- Produces: 7,474–8,050 的终端区 DOM 与浮层拆为三组件。

- [x] **Step 1: 拆连接/本地/协议状态浮层（7,708–7,798）**

生成 `TerminalStatusOverlays.vue`，props：`reconnectPending`、`recordingActive`、`recordCountdown`、`localState`、`telnetState`、`serialState`、`vncState`、`rdpState`、`t`。

- [x] **Step 2: 拆传输/回放浮层（7,848–7,980）**

生成 `TransferOverlays.vue`，props：`zmodem*`、`trzsz*`、`serialUpload`、`replayState`。

- [x] **Step 3: 组装 `WorkbenchStage.vue`**

保留 xterm 宿主、壁纸、诊断层、投放遮罩、AI 修复条、命令标记、批量栏状态；通过 `defineExpose({ terminalHost })` 暴露宿主 ref。

- [x] **Step 4: CSS 与验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：本地 shell 退出层、Telnet/串口/VNC/RDP 关闭层、zmodem/trzsz 进度、回放弹层、拖放遮罩。

- [x] **Step 5: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract terminal stage and overlays"
```

### Task 21: 侧栏与对话框宿主

**Files:**
- Create: `frontend/src/app/components/WorkbenchSidePanel.vue`
- Create: `frontend/src/app/components/WorkbenchDialogs.vue`
- Modify: `frontend/src/app/App.vue`
- Modify: `frontend/src/app/workbench.spec.ts`（模板组件解析守卫）

**Interfaces:**
- Consumes: 现有 `SftpWorkspacePanel`、`DockerPanel`、`MetricsPanel`、各对话框组件与状态。
- Produces: 右侧三视图外壳（8,051–8,400）与剩余内联对话框（OTP/导入/预览/录制/审计/拖放上传/目录选择）独立组件。

- [x] **Step 1: 拆 `WorkbenchSidePanel.vue`**

保留 `sftpPane` ref、三视图切换、`@paste/@dragenter/@dragover/@dragleave/@drop` 上抛。

- [x] **Step 2: 拆 `WorkbenchDialogs.vue`**

逐组迁移内联 `<Dialog>`，props 使用 `v-model:open` 风格；不改变 reka 传送门行为。

- [x] **Step 3: 更新结构守卫**

`workbench.spec.ts` 的“模板组件解析守卫”扩展扫描 `app/components/*.vue`，确保新增组件仍能解析图标与导入。

- [x] **Step 4: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：SFTP/Docker/Metrics 切换、SFTP 右键粘贴与文件拖入、各对话框开关、窄视口布局。

- [x] **Step 5: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): extract side panel and dialog host"
```

### Task 21b: 共享状态工厂与工作台聚合器

**Files:**
- Create: `frontend/src/app/workbench/state/useTerminalState.ts`
- Create: `frontend/src/app/workbench/state/useSessionState.ts`
- Create: `frontend/src/app/workbench/state/useSftpState.ts`
- Create: `frontend/src/app/workbench/state/useLayoutState.ts`
- Create: `frontend/src/app/workbench/state/useDialogState.ts`
- Create: `frontend/src/app/workbench/state/useWorkbenchContext.ts`
- Create: `frontend/src/app/workbench/useTerminalWorkbench.ts`
- Create: `frontend/src/app/workbench/useSessionWorkbench.ts`
- Create: `frontend/src/app/workbench/useSftpWorkbench.ts`
- Create: `frontend/src/app/workbench/useShellWorkbench.ts`
- Create: `frontend/src/app/workbench/useWorkbench.ts`
- Modify: `frontend/src/app/App.vue`

**Interfaces:**
- Consumes: Tasks 8–18b 抽取出的所有模块。
- Produces:
  - `useWorkbenchContext()` → `{ terminal, session, sftp, layout, dialogs, appearance, locale, workbenchId }`，每个命名空间是按域创建的 refs/reactive（纯本地工厂，不是全局单例）。
  - `useWorkbench()` → `{ ctx, terminal, sessions, sftp, shell }`，其中四个域对象是聚合器返回的扁平 API。

- [x] **Step 1: 按域迁出状态声明**

从 `App.vue` 搬移声明（保留初始值与消毒器）：

- `useTerminalState`：`terminal`、`fitAddon`、`searchAddon`、`webglRenderer`、`terminalHost`、`terminalState`、`terminalError`、`terminalFontSize`、`terminalFontOverride`、`terminalAppearance`、`terminalBehavior`、`commandMarker`、`commandMarkerElapsed`、`terminalCwd`、`directoryTrackingSupported`、`terminalCopyCache`、`terminalDiag`、`terminalInputQueue`、`serialInputQueue`、`outputGate`。
- `useSessionState`：`session`、`connectionReadOnly`、`connectCancelled`、`connectSucceeded`、`inactiveWaiting`、`connectLogsOpen`、`connectLog`、`hostKeyPrompt`、`rememberHostKey`、本地/Telnet/串口/VNC/RDP 会话 refs、`connectionId`、`connectionProtocol`、`runtimeEndpoint`、`connection`。
- `useSftpState`：`entries`、`currentPath`、`selectedPath`、`selectedUris`、`lastClickedUri`、`loadingFiles`、`sftpPaneOpen`、`rightPanelView`、`sftpListingLoaded`、`sftpHomePath`、`visibleColumns`、`sftpColumnWidths`、`sftpNameWidth`、`sort`、`sftpSearch`、`sftpTypeFilter`、`sftpShowHidden`、`sftpTreeRoot`、`sftpClipboard`、SFTP 对话框 refs。
- `useLayoutState`：`splitRatio`、`dockerSplitRatio`、`paneOrder`、`paneDirection`、`terminalLayout`、`followDirectory`、`sudoMode`。
- `useDialogState`：`settingsOpen`、`auditOpen`、`profilesOpen`、`otpPanelOpen`、`importPanelOpen`、`batchDeleteOpen`、`newFileDialog`、`forwardsOpen`、`sessionMenuOpen`、`ghostKeycapMenuOpen` 等 UI 开关。

依赖这些 refs 的派生 computed 一并迁入对应工厂（`connection`、`connectionIdentity`、`sessionPillText`、`toolbarStyle`、`sftpTreeRows` 等）；设置/偏好适配器（`downloadPrefsAdapter`、`transferPrefsAdapter`、`suggestionPrefsAdapter`、`webglEnabled`）并入 `useWorkbenchPrefs` 的返回对象；`diskUsage` 与指标轮询并入 `useDockMetrics` 的返回对象。

- [x] **Step 2: 组合上下文**

`useWorkbenchContext.ts`：

```ts
import { computed, ref } from "vue";
import { useTerminalState } from "./useTerminalState";
import { useSessionState } from "./useSessionState";
import { useSftpState } from "./useSftpState";
import { useLayoutState } from "./useLayoutState";
import { useDialogState } from "./useDialogState";

export function useWorkbenchContext() {
  const hostContext = ref<Record<string, unknown>>({});
  const appearance = ref(resolveAppearance());
  const locale = ref("zh-CN");
  const fallbackWorkbenchId = randomUUID();
  const workbenchId = computed(() => resolveWorkbenchId(hostContext.value, fallbackWorkbenchId));
  return {
    hostContext,
    appearance,
    locale,
    workbenchId,
    terminal: useTerminalState(),
    session: useSessionState(),
    sftp: useSftpState(),
    layout: useLayoutState(),
    dialogs: useDialogState(),
  };
}
```

`fallbackWorkbenchId` 必须继续走 `lib/uuid` shim（issue #77）；`randomUUID`、`resolveAppearance`、`resolveWorkbenchId` 的 import 随迁。

- [x] **Step 3: 写四个域聚合器**

每个聚合器只做“调用已抽取模块 + 返回扁平 API”，不写实现：

```ts
export function useTerminalWorkbench(ctx: WorkbenchContext) {
  const instance = useTerminalInstance(ctx.terminal);
  const writer = useTerminalWriter({ ...ctx.terminal, terminal: instance.terminal });
  const input = useTerminalInput({ ...ctx.terminal, terminal: instance.terminal });
  const keys = useTerminalKeys({ ...ctx.terminal, ...instance, ...input, terminal: instance.terminal });
  const decorations = useTerminalDecorations({ ...ctx.terminal, terminal: instance.terminal });
  return { ...instance, ...writer, ...input, ...keys, ...decorations };
}
```

`useSessionWorkbench`、`useSftpWorkbench`、`useShellWorkbench` 同构：只做调用与合并，不写实现。

- [x] **Step 4: 顶层聚合**

`useWorkbench.ts`：

```ts
export function useWorkbench() {
  const ctx = useWorkbenchContext();
  return {
    ctx,
    terminal: useTerminalWorkbench(ctx),
    sessions: useSessionWorkbench(ctx),
    sftp: useSftpWorkbench(ctx),
    shell: useShellWorkbench(ctx),
  };
}
```

- [x] **Step 5: App.vue 改为单入口**

```ts
const wb = useWorkbench();
```

模板组件按域接收 `wb.terminal` / `wb.sessions` / `wb.sftp` / `wb.shell`，不再直接引用上百个 ref。

- [x] **Step 6: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：与 Task 21 相同清单，重点确认启动、终端输入输出、SFTP 操作、设置与分屏。

- [x] **Step 7: 更新源码扫描守卫**

`frontend/src/shared/lib/uuidCallSites.spec.ts` 的 “keeps App.vue importing the shim” 用例改为断言 `useWorkbenchContext.ts` 引入 shim；裸调用扫描保持不变。

- [x] **Step 8: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): add workbench state factories and domain aggregators"
```

### Task 22: App.vue 收口

**Files:**
- Modify: `frontend/src/app/App.vue`
- Modify: `frontend/src/shared/lib/structure.spec.ts`

**Interfaces:**
- Consumes: 前面所有 composable、状态工厂与组件。
- Produces: `App.vue` ≤600 行，只含 `useWorkbench()` 单入口、外壳布局与生命周期 hooks。

- [x] **Step 1: 收敛剩余装配**

把仍留在 `App.vue` 的 options 对象提取为 `app/workbench/lib/wiring.ts` 的构造函数（纯数据/回调打包）；删除已迁入 `state/` 的重复声明。`App.vue` 只保留 `const wb = useWorkbench()`、`onMounted(wb.shell.mount)`、`onBeforeUnmount(wb.shell.unmount)` 与外壳模板。

- [x] **Step 2: 移除 App.vue 白名单**

从 `LINE_ALLOWLIST` 删除 `"../../app/App.vue"`。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(app): reduce App.vue to a composition root"
```

---

## Wave 6：前端剩余超限文件

### Task 23: SettingsDialog

**Files:**
- Modify: `frontend/src/features/settings/components/SettingsDialog.vue`
- Create: `frontend/src/features/settings/composables/useSettingsSave.ts`
- Create: `frontend/src/features/settings/components/{SettingsNav,SettingsFooter}.vue`（按现有分区补足）

**Interfaces:**
- Consumes: 现有 pane 组件与共享草稿状态。
- Produces: `SettingsDialog.vue` ≤600 行；保存链、导航、脏状态判定各自独立。

- [x] **Step 1: 迁出保存链**

把 `save()` 及其校验/逐项写入逻辑迁到 `useSettingsSave`，返回 `{ save, saving, saveError, dirty }`。

- [x] **Step 2: 迁出导航与外壳**

导航列表、底部按钮、提示条拆为组件；共享草稿仍由 `SettingsDialog` 持有。

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

浏览器冒烟：设置各分区切换、修改后保存、取消回滚、错误提示。

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "refactor(settings): split settings dialog orchestrator"
```

### Task 24: figSourceCore

**Files:**
- Modify: `frontend/src/features/completion/lib/completion/fig/figSourceCore.ts`
- Create: `frontend/src/features/completion/lib/completion/fig/figCorpus.ts`
- Create: `frontend/src/features/completion/lib/completion/fig/figQuery.ts`

**Interfaces:**
- Consumes: 现有 vendored parser 与生成语料。
- Produces: 三个 ≤600 行模块；`figSourceCore.ts` 保留冻结接缝 `FigCompletionSource` 的实现。

- [x] **Step 1: 按职责搬迁**

- 语料装载/索引 → `figCorpus.ts`
- 查询、过滤、排序 → `figQuery.ts`
- `FigCompletionSource` 组合层留在 `figSourceCore.ts`

- [x] **Step 2: 验证**

```bash
pnpm --dir frontend typecheck
pnpm --dir frontend exec vitest run src/features/completion
```

- [x] **Step 3: Commit**

```bash
git add frontend/src
git commit -m "refactor(completion): split fig source core"
```

### Task 25: SftpTreePane 测试拆分

**Files:**
- Modify: `frontend/src/features/sftp/components/SftpTreePane.spec.ts`
- Create: `frontend/src/features/sftp/components/SftpTreePane.details.spec.ts`
- Create: `frontend/src/features/sftp/components/sftpTreePaneFixture.ts`

**Interfaces:**
- Consumes: 现有夹具函数 `makeEntry`/`makeNode`/`row`/`mountPane`/`dispatchDrag`。
- Produces: 行交互 spec 与详情卡/拖放 spec 各自 ≤1,200 行；共享夹具独立。

- [x] **Step 1: 提取夹具**

把 `makeEntry`、`makeNode`、`row`、`PaneProps`、`mountPane`、`dispatchDrag`、`flushMicrotasks` 移到 `sftpTreePaneFixture.ts`。

- [x] **Step 2: 拆分用例**

- 行点击/键盘/改名/选中态 → `SftpTreePane.spec.ts`
- Info 详情卡 + 拖放目标/1s 展开 → `SftpTreePane.details.spec.ts`

- [x] **Step 3: 验证**

```bash
pnpm --dir frontend exec vitest run src/features/sftp/components
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

- [x] **Step 4: Commit**

```bash
git add frontend/src
git commit -m "test(sftp): split tree pane specs"
```

---

## Wave 7：后端

### Task 26: `core/exec` 迁入 SSH

**Files:**
- Move: `backend/src/core/exec/` → `backend/src/sessions/ssh/exec/`
- Modify: `backend/src/core/mod.rs`
- Modify: `backend/src/sessions/ssh/mod.rs`
- Modify: 所有 `crate::core::exec` 引用

**Interfaces:**
- Consumes: 无。
- Produces: `crate::sessions::ssh::exec::*` 与原导出保持一致。

- [x] **Step 1: 移动目录并改模块声明**

```bash
git mv backend/src/core/exec backend/src/sessions/ssh/exec
```

`core/mod.rs` 删除 `pub(crate) mod exec;`；`sessions/ssh/mod.rs` 加入 `pub(crate) mod exec;`。

- [x] **Step 2: 批量替换路径**

```bash
grep -rl 'crate::core::exec' backend/src --include='*.rs' | xargs perl -pi -e 's/crate::core::exec/crate::sessions::ssh::exec/g'
grep -rl 'pub(in crate::core::exec)' backend/src --include='*.rs' | xargs perl -pi -e 's/pub\(in crate::core::exec\)/pub(in crate::sessions::ssh::exec)/g'
```

- [x] **Step 3: 验证**

```bash
cargo test --manifest-path backend/Cargo.toml
```

Expected: 全绿。`filesystem` 暂时引用 `crate::sessions::ssh::exec::shell_quote`，Task 27 会迁回 core。

- [x] **Step 4: Commit**

```bash
git add backend/src
git commit -m "refactor(backend): move remote exec pipeline under ssh sessions"
```

### Task 27: `core` 类型下沉

**Files:**
- Create: `backend/src/core/shell_quote.rs`
- Create: `backend/src/core/model/name_encoding.rs`
- Create: `backend/src/core/preferences/transfer.rs`
- Create: `backend/src/core/preferences/env.rs`
- Create: `backend/src/core/model/connection/triggers.rs`
- Create: `backend/src/core/keys/decode_error.rs`
- Modify: `backend/src/core/mod.rs`、`core/preferences/mod.rs`、`core/model/mod.rs`、`core/model/connection.rs`、`core/keys/mod.rs`
- Modify: `filesystem/sftp/name.rs`、`features/transfer/compress/policy.rs`、`features/automation/triggers/types.rs`、`sessions/local/env.rs`、`sessions/ssh/auth/private_key.rs`、`filesystem/sftp/ext.rs`、`filesystem/sftp/copy/*`

**Interfaces:**
- Consumes: Task 26 后的模块布局。
- Produces: `core` 不再出现上层 crate 路径；原调用点通过 re-export 保持不变。

- [x] **Step 1: 下沉 `shell_quote`**

新建 `core/shell_quote.rs`：

```rust
//! Shell quoting helpers shared by remote command builders.

/// Single-quote shell escaping for embedding a path in a remote command.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}
```

删除 `sessions/ssh/exec/quote.rs`，把 `exec/mod.rs` 的 `mod quote; pub use quote::*;` 改为 `pub use crate::core::shell_quote::shell_quote;`；`filesystem` 引用改为 `crate::core::shell_quote::shell_quote`。

- [x] **Step 2: 下沉 `NameEncoding`**

把 `filesystem/sftp/name.rs` 的 `NameEncoding` 枚举与 `parse/as_str` 迁到 `core/model/name_encoding.rs`；在 `name.rs` 顶部 `pub use crate::core::model::name_encoding::NameEncoding;`。

- [x] **Step 3: 下沉压缩策略**

把 `CompressPolicy`、`THRESHOLD_MIB_DEFAULT`、`THRESHOLD_MIB_MAX`、`WORTHWHILE_PERCENT`、`WEAK_CPU_PARALLELISM` 迁到 `core/preferences/transfer.rs`；`features/transfer/compress/policy.rs` 改为 `pub use crate::core::preferences::transfer::{...}`。

- [x] **Step 4: 下沉触发器类型**

把 `features/automation/triggers/types.rs` 整体迁到 `core/model/connection/triggers.rs`；`core/model/connection.rs` 的字段直接引用 `triggers::TriggersConfig`；`features/automation/triggers/mod.rs` re-export core 类型。

- [x] **Step 5: 下沉环境变量清洗**

把 `USER_ENV_MAX_PAIRS`、`sanitize_env_pair` 与相关上限常量迁到 `core/preferences/env.rs`；`sessions/local/env.rs` re-export。

- [x] **Step 6: 下沉私钥错误文案**

把 `private_key_decode_failure` 与 `looks_like_public_key` 迁到 `core/keys/decode_error.rs`；`sessions/ssh/mod.rs` 继续 `pub(crate) use crate::core::keys::decode_error::private_key_decode_failure;`。

同时补齐新模块声明：`core/mod.rs` 加 `pub(crate) mod shell_quote;`；`core/model/mod.rs` 加 `pub(crate) mod name_encoding;`；`core/preferences.rs` 加 `mod env;` 与 `mod transfer;`（并按现有 `pub use` 风格导出）；`core/model/connection.rs` 加 `mod triggers;`；`core/keys.rs` 加 `mod decode_error;`。

- [x] **Step 7: 解除 core 守卫 ignore**

删除 Task 2 中 `core_does_not_depend_on_upper_layers` 的 `#[ignore]`，运行：

```bash
cargo test --manifest-path backend/Cargo.toml structure_tests
cargo test --manifest-path backend/Cargo.toml
```

Expected: 全绿，core 无上层引用。

- [x] **Step 8: Commit**

```bash
git add backend/src
git commit -m "refactor(backend): make core a leaf layer"
```

### Task 28: 拆分 `open.rs`

**Files:**
- Move: `backend/src/sessions/ssh/connection/open.rs` → `connection/open/{mod,pty,register,reader,error}.rs`
- Modify: `backend/src/sessions/ssh/connection/mod.rs`

**Interfaces:**
- Consumes: 现有 `open_connection` 调用方（保持不变）。
- Produces: `connection::open::open_connection` 签名不变；子模块 `pub(super)`。

- [x] **Step 1: 建目录并移动**

```bash
mkdir -p backend/src/sessions/ssh/connection/open
git mv backend/src/sessions/ssh/connection/open.rs backend/src/sessions/ssh/connection/open/mod.rs
```

- [x] **Step 2: 按职责搬移函数**

- `pty.rs`：PTY/通道请求/窗口尺寸
- `register.rs`：会话注册、转发/keepalive 启动
- `reader.rs`：读循环与消息分派
- `error.rs`：错误映射与超时文案

每搬一组运行 `cargo test --manifest-path backend/Cargo.toml sessions::ssh`。

- [x] **Step 3: 验证**

```bash
cargo test --manifest-path backend/Cargo.toml
```

- [x] **Step 4: Commit**

```bash
git add backend/src
git commit -m "refactor(ssh): split connection open into focused modules"
```

### Task 29: 拆分 `local/runtime.rs`

**Files:**
- Move: `backend/src/sessions/local/runtime.rs` → `local/runtime/{mod,lifecycle,io,control}.rs`
- Modify: `backend/src/sessions/local/mod.rs`

**Interfaces:**
- Consumes: 现有 runtime 公开函数与 `use runtime::*`。
- Produces: 对外签名不变。

- [x] **Step 1: 建目录并移动**

```bash
mkdir -p backend/src/sessions/local/runtime
git mv backend/src/sessions/local/runtime.rs backend/src/sessions/local/runtime/mod.rs
```

- [x] **Step 2: 按职责搬移**

- `lifecycle.rs`：spawn/reap/退出态
- `io.rs`：stdin/stdout 泵与帧编码
- `control.rs`：resize/signal/关闭

- [x] **Step 3: 验证**

```bash
cargo test --manifest-path backend/Cargo.toml
```

- [x] **Step 4: Commit**

```bash
git add backend/src
git commit -m "refactor(local): split local terminal runtime"
```

### Task 30: 后端测试文件拆分

**Files:**
- Move: `sessions/ssh/tests.rs` → `sessions/ssh/tests/{mod,connection,transfer,tree,terminal,forwarding,security}.rs`
- Move: `features/mcp/tests.rs` → `features/mcp/tests/{mod,tools,safety,connections,bridge}.rs`
- Move: `features/import/tests.rs` → `features/import/tests/{mod,ssh_config,stream,apply}.rs`
- Move: `core/model/manifest_contract_tests.rs` → `core/model/manifest_contract/{mod,form,fields,triggers,localization}.rs`
- Modify: 对应 `mod.rs` / 模块声明

**Interfaces:**
- Consumes: 现有测试函数与 helper。
- Produces: 每个测试文件 ≤1,200 行；测试数量不减少。

- [x] **Step 1: 记录测试数量**

```bash
cargo test --manifest-path backend/Cargo.toml 2>&1 | tail -5
```

记录通过数，最终必须 ≥ 该值。

- [x] **Step 2: 拆 SSH 测试**

按主题搬运，helper（`auth_failure`、`method_set`、`test_timeout` 等）放 `mod.rs` 并 `pub(super) use`。每拆完一组运行：

```bash
cargo test --manifest-path backend/Cargo.toml sessions::ssh
```

- [x] **Step 3: 拆 MCP / import / manifest 测试**

同样按主题搬运；`state()`、`zip_entry_names` 等 helper 放各自 `mod.rs`。

- [x] **Step 4: 验证全量**

```bash
cargo test --manifest-path backend/Cargo.toml
```

Expected: 通过数不低于 Step 1，`structure_tests::sources_stay_within_line_budget` PASS。

- [x] **Step 5: Commit**

```bash
git add backend/src
git commit -m "test(backend): split oversized test modules by scenario"
```

---

## Wave 8：收口与交付

### Task 31: 清空白名单、全量验证与打包

**Files:**
- Modify: `frontend/src/shared/lib/structure.spec.ts`
- Modify: `backend/src/structure_tests.rs`
- Modify: `manifest.json`、`backend/Cargo.toml`、`backend/Cargo.lock`、`CHANGELOG.md`

**Interfaces:**
- Consumes: 所有前置任务。
- Produces: 无白名单的结构守卫、可安装的 `.dbxp`。

- [x] **Step 1: 清空白名单**

两个守卫的 `LINE_ALLOWLIST` 改为空；确认无 stale 项。

- [x] **Step 2: 跑结构守卫**

```bash
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
cargo test --manifest-path backend/Cargo.toml structure_tests
```

Expected: 全绿且无 allowlist。

- [x] **Step 3: 全量门禁**

```bash
scripts/test.sh --skip-host
```

Expected: 后端测试、前端 typecheck/test/build、dead-code、MCP/本地 smoke 全部通过。

- [x] **Step 4: 浏览器全流程冒烟**

```bash
pnpm --dir frontend dev
```

打开 `visual.html`，逐项确认并截图：

- 主终端输入/输出、复制粘贴
- 新标签页、分屏、嵌套分屏、活动窗格切换
- `#` AI 搜索
- SFTP 选中/展开/右键菜单/拖拽上传/1s 悬停展开
- 设置各分区保存
- Docker 面板上下分屏
- 窄视口（720px）工具栏不溢出
- 控制台无新增 error

- [x] **Step 5: 升版与打包**

版本从 `0.7.9-beta.28` 升到 `0.7.9-beta.29`，同步 `manifest.json`、`backend/Cargo.toml`、`backend/Cargo.lock`；CHANGELOG 写双语条目（结构重构、行为不变）。

```bash
scripts/build.sh
ls -l dist/io.dbx.sshx-0.7.9-beta.29-darwin-arm64.dbxp
shasum -a 256 dist/io.dbx.sshx-0.7.9-beta.29-darwin-arm64.dbxp
```

- [x] **Step 6: Commit**

```bash
git add frontend/src/shared/lib/structure.spec.ts backend/src/structure_tests.rs manifest.json backend/Cargo.toml backend/Cargo.lock CHANGELOG.md
git commit -m "refactor: enforce source structure budgets and release beta.29"
```

- [x] **Step 7: 报告**

报告：每波提交哈希、最终结构守卫结果、全量测试数量、`.dbxp` 路径/大小/SHA-256。等待用户安装验证；不 push、不自行合并。

---

## 自检清单

- [x] 每个 spec 目标都有对应任务：`App.vue` 逻辑（Tasks 8–18b、21b–22）、模板（19–21）、目录归位（4–7）、`shared` 越界（5）、SFTP 工作区（14b）、后端 `core` 叶子（26–27）、后端大文件（28–30）、其余前端超限（23–25）、守卫与打包（1–2、31）。
- [x] 每个任务都有独立验证命令和提交点。
- [x] 没有 TBD/TODO/“稍后实现”占位。
- [x] 接口名在各任务间一致：`useWorkbenchContext`、`useWorkbench`、`useWorkbenchPersistence`、`useSplitDivider`、`useWorkbenchModals`、`useWorkbenchBootstrap`、`useHostDragBridge`、`useTerminalInstance`、`useTerminalWriter`、`useTerminalInput`、`useTerminalKeys`、`useTerminalReconnect`、`useCommandMarker`、`useTerminalClipboard`、`useTerminalHighlight`、`useTerminalActionLinks`、`useTerminalGutter`、`useSshConnection`、`useLocalShellSession`、`useTelnetWorkbench`、`useSerialWorkbench`、`useVncWorkbench`、`useRdpWorkbench`、`useSftpWorkspace`、`useSftpDialogs`、`useSftpDropUpload`、`useSftpPreview`。
