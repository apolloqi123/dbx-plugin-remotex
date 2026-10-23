import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";

const root = path.dirname(fileURLToPath(import.meta.url));
const temporary = path.join(root, "dist");
const output = path.resolve(root, "../ui");

await build({
  root,
  configFile: false,
  plugins: [vue(), tailwindcss()],
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(root, "src"),
    },
  },
  build: {
    outDir: temporary,
    emptyOutDir: true,
    cssCodeSplit: false,
    assetsInlineLimit: 10 * 1024 * 1024,
    // rolldown（vite 8 内置）：inlineDynamicImports 已弃用，codeSplitting: false
    // 为官方等价项（单 bundle 内联全部 dynamic imports），产物结构不变。
    rollupOptions: { output: { codeSplitting: false } },
  },
});

let html = await fs.readFile(path.join(temporary, "index.html"), "utf8");
const scriptMatch = html.match(/<script[^>]+src="\.\/([^"]+\.js)"[^>]*><\/script>/);
if (!scriptMatch) throw new Error("Vite output did not contain a JavaScript entry");
const script = await fs.readFile(path.join(temporary, scriptMatch[1]), "utf8");
// 内联双层转义：① `</script` 防止提前闭合；② `<!--` 防止 HTML 解析器进入
// script 注释态（highlight.js 语言定义同时含 `<!--` 与 `<script` 字面量，
// 组合起来会让真实闭合标签失效）——`\!` 在 JS 字符串/正则中均等价 `!`，语义不变。
const inlineScript = script.replace(/<\/script/gi, "<\\/script").replace(/<!--/g, "<\\!--");
html = html.replace(scriptMatch[0], () => `<script type="module">${inlineScript}</script>`);

const styleMatch = html.match(/<link[^>]+href="\.\/([^"]+\.css)"[^>]*>/);
if (styleMatch) {
  const style = await fs.readFile(path.join(temporary, styleMatch[1]), "utf8");
  const inlineStyle = style.replace(/<\/style/gi, "<\\/style");
  html = html.replace(styleMatch[0], () => `<style>${inlineStyle}</style>`);
}

if (html.includes(scriptMatch[0])) {
  throw new Error("Self-contained UI still contains the external JavaScript entry tag");
}
if (styleMatch && html.includes(styleMatch[0])) {
  throw new Error("Self-contained UI still contains the external stylesheet tag");
}
if ((html.match(/<script\b/gi) ?? []).length !== 1 || (html.match(/<\/script>/gi) ?? []).length !== 1) {
  throw new Error("Self-contained UI contains an invalid script structure");
}

// 语料唯一性断言（bundle 去重回归门）：fig spec manifest 的静态副本只允许
// 存在一份——曾经主线程静态 import + inline worker 各内嵌一份（ui/index.html
// 多出约 3.7 MB 明文字料）。marker 取自 git spec 的 description，每个语料副本
// 恰好出现一次（含 inline worker 的转义字符串副本）。
const FIG_CORPUS_MARKER = "The number of submodules fetched at the same time";
const corpusCopies = html.split(FIG_CORPUS_MARKER).length - 1;
if (corpusCopies !== 1) {
  throw new Error(
    `Fig spec corpus must be embedded exactly once (found ${corpusCopies}). ` +
      "The inline worker receives the corpus via the init message; do not reintroduce a static manifest import in worker code.",
  );
}

// 体积预算：ui/index.html 为单文件插件 UI，语料/预览组件都内联在此。超预算
// 说明又混入了可延迟加载或重复的大块内容（第一次修复前为 12,684,314 字节）。
const UI_SIZE_BUDGET_BYTES = 9_750_000;
const htmlBytes = Buffer.byteLength(html, "utf8");
if (htmlBytes > UI_SIZE_BUDGET_BYTES) {
  throw new Error(
    `Self-contained UI is ${htmlBytes} bytes, over the ${UI_SIZE_BUDGET_BYTES} byte budget`,
  );
}

await fs.mkdir(output, { recursive: true });
await fs.writeFile(path.join(output, "index.html"), html, "utf8");
console.log(`Wrote self-contained plugin UI to ${path.join(output, "index.html")} (${htmlBytes} bytes)`);
