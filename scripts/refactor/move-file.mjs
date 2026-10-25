#!/usr/bin/env node
// Moves one frontend source file and rewrites every relative import that points
// at it (and the moved file's own relative imports). Usage is path-based so the
// caller can run it repeatedly for grouped migrations.
//
//   node scripts/refactor/move-file.mjs <from> <to>
//
// Both paths are resolved against the repo root (absolute paths also work) and
// must live inside frontend/src. Non-relative specifiers (`@/...`, packages)
// are never touched.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmdirSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
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
if (!existsSync(from) || !statSync(from).isFile()) throw new Error(`missing file: ${from}`);
if (existsSync(to)) throw new Error(`target already exists: ${to}`);
if (from === to) throw new Error("source and target are the same path");

// Matches `from "x"`, `import("x")` and bare `import "x"` forms; group 3 is the
// specifier. Only relative specifiers (leading dot) are considered.
const SPECIFIER = /(\bfrom\s*|\bimport\s*\(\s*|^\s*import\s+)(["'])(\.[^"']*)\2/gm;
const EXTENSIONS = [".ts", ".tsx", ".vue", ".d.ts", ".mjs", ".js", ".json"];

// macOS/Windows filesystems are case-insensitive, so importers may spell a path
// differently from the on-disk name. Compare case-insensitively there.
const caseInsensitive = process.platform === "darwin" || process.platform === "win32";
const normalize = (path) => (caseInsensitive ? path.toLowerCase() : path);

/** Split `"./x.ts?raw"` into its path part and its `?query`/`#hash` suffix. */
function splitSuffix(spec) {
  const clean = spec.replace(/[?#].*$/, "");
  return { clean, suffix: spec.slice(clean.length) };
}

/** True when the written specifier carries a file extension. */
function hasExtension(clean) {
  return /\.[A-Za-z0-9]+$/.test(basename(clean));
}

/**
 * Resolve a relative specifier to a concrete file on disk, mirroring the
 * bundler's rules (exact path, then known extensions, then `index.*`).
 * Returns null when nothing exists (e.g. virtual modules).
 */
function resolveOnDisk(importerDir, clean) {
  const base = resolve(importerDir, clean);
  const candidates = [base];
  for (const ext of EXTENSIONS) candidates.push(`${base}${ext}`);
  for (const ext of EXTENSIONS) candidates.push(join(base, `index${ext}`));
  for (const candidate of candidates) {
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

/** Build the specifier that points from `importerDir` at `target`. */
function specifierFor(importerDir, target, clean) {
  const extensionless = !hasExtension(clean);
  const explicitIndex = clean.endsWith("/index");
  let next = relative(importerDir, target).split(sep).join("/");
  if (extensionless) {
    next = next.replace(/\.(d\.ts|ts|tsx|vue|mjs|js|json)$/, "");
    // Mirror the original style: `./tree` stays a directory specifier while
    // `./tree/index` keeps its explicit `index` segment.
    if (!explicitIndex && /^index\./.test(basename(target)) && next.endsWith("/index")) {
      next = next.slice(0, -"/index".length);
    }
  }
  if (next === "" || next === ".") return ".";
  if (!next.startsWith(".")) next = `./${next}`;
  return next;
}

/**
 * Rewrite the specifiers of one file.
 * `fromDir` is the directory the existing specifiers resolve against, `toDir`
 * the directory the rewritten specifiers must resolve against (they differ only
 * for the file being moved). `mapTarget` receives the absolute file a specifier
 * currently resolves to and returns the absolute file it must resolve to
 * afterwards, or null to keep the specifier as is.
 */
function rewrite(source, fromDir, toDir, mapTarget, resolveTarget) {
  let hits = 0;
  const nextSource = source.replace(SPECIFIER, (match, prefix, quote, spec) => {
    const { clean, suffix } = splitSuffix(spec);
    const target = resolveTarget(fromDir, clean);
    if (!target) return match;
    const mapped = mapTarget(target);
    if (!mapped) return match;
    hits += 1;
    return `${prefix}${quote}${specifierFor(toDir, mapped, clean)}${suffix}${quote}`;
  });
  return { source: nextSource, hits };
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(ts|vue)$/.test(name)) out.push(path);
  }
  return out;
}

/** `git mv` leaves the old directory behind; drop it when nothing else is in it. */
function pruneEmptyDirs(dir) {
  let current = dir;
  while (current !== SRC && current.startsWith(`${SRC}${sep}`) && readdirSync(current).length === 0) {
    rmdirSync(current);
    current = dirname(current);
  }
}

// 1. The moved file: every relative specifier it contains is re-based from the
//    old directory to the new one. Resolution stays lexical so imports pointing
//    outside frontend/src (vendor bundles, generated manifests) still move
//    correctly even when they have no on-disk extension match.
const movedText = rewrite(
  readFileSync(from, "utf8"),
  dirname(from),
  dirname(to),
  (target) => (normalize(target) === normalize(from) ? to : target),
  (importerDir, clean) => {
    const lexical = resolve(importerDir, clean);
    if (normalize(lexical) === normalize(from)) return from;
    return resolveOnDisk(importerDir, clean) ?? lexical;
  },
);

// 2. Every other file in frontend/src that imported the old path. This runs
//    before the move so the old specifiers still resolve on disk.
const pending = [];
let rewrittenSpecifiers = movedText.hits;
for (const file of walk(SRC)) {
  const before = readFileSync(file, "utf8");
  const after = rewrite(
    before,
    dirname(file),
    dirname(file),
    (target) => (normalize(target) === normalize(from) ? to : null),
    resolveOnDisk,
  );
  if (after.source !== before) {
    pending.push([file, after.source]);
    rewrittenSpecifiers += after.hits;
  }
}

// 3. Move the file, then write the re-based content back in its new home.
mkdirSync(dirname(to), { recursive: true });
execFileSync("git", ["mv", from, to], { cwd: ROOT });
pruneEmptyDirs(dirname(from));
writeFileSync(to, movedText.source, "utf8");
for (const [file, source] of pending) writeFileSync(file, source, "utf8");

console.log(
  `moved ${relative(ROOT, from)} -> ${relative(ROOT, to)}; ` +
    `rewrote ${rewrittenSpecifiers} specifier(s) in ${pending.length} file(s)`,
);
