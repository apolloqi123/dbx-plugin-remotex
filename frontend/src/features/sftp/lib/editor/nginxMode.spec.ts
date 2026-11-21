// Nginx 着色回归：legacy nginx 模式把路径里的 `/*`（`modules/*.conf`）当成
// C 注释起点，之后整篇都被标成注释。nginx 只有词首 `#` 是注释。
import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { HighlightStyle, ensureSyntaxTree } from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { nginxLanguage } from "./nginxMode";

const style = HighlightStyle.define([
  { tag: tags.comment, class: "cm-test-comment" },
  { tag: tags.keyword, class: "cm-test-keyword" },
  { tag: tags.variableName, class: "cm-test-variable" },
  { tag: tags.string, class: "cm-test-string" },
  { tag: tags.number, class: "cm-test-number" },
]);

function ranges(text: string, extension: Extension = nginxLanguage()) {
  const state = EditorState.create({ doc: text, extensions: [extension] });
  const tree = ensureSyntaxTree(state, text.length, 10_000);
  expect(tree).not.toBeNull();
  const out: Array<{ text: string; classes: string[] }> = [];
  highlightTree(tree!, style, (from, to, classes) => {
    out.push({ text: text.slice(from, to), classes: classes.split(" ") });
  });
  return out;
}

function withClass(text: string, className: string): string[] {
  return ranges(text).filter((range) => range.classes.includes(className)).map((range) => range.text);
}

describe("nginxLanguage comments", () => {
  it("does not treat glob paths as C comments (the swallowed-file bug)", () => {
    const text = [
      "include /usr/share/nginx/modules/*.conf;",
      "http {",
      "    include /etc/nginx/mime.types;",
      "    server { listen 80; }",
      "}",
      "# real comment",
    ].join("\n");
    expect(withClass(text, "cm-test-comment")).toEqual(["# real comment"]);
  });

  it("only starts a comment at a token boundary", () => {
    // nginx 的 sharp_comment 只在词首置位：词中 `#` 属于参数。
    expect(withClass("proxy_pass http://h/a#b;", "cm-test-comment")).toEqual([]);
    expect(withClass("root /var/www/#dir;", "cm-test-comment")).toEqual([]);
    expect(withClass("server_name a.example.com; # tail", "cm-test-comment")).toEqual(["# tail"]);
    expect(withClass("  # indented", "cm-test-comment")).toEqual(["# indented"]);
  });

  it("keeps # inside quoted strings as text", () => {
    expect(withClass('add_header X-Test "a # b";', "cm-test-comment")).toEqual([]);
    expect(withClass("log_format main 'a # b'; # tail", "cm-test-comment")).toEqual(["# tail"]);
  });

  it("does not leak comment state into following lines", () => {
    const text = "include /a/*.conf;\nworker_processes auto;\n# tail";
    expect(withClass(text, "cm-test-comment")).toEqual(["# tail"]);
    // 后续行的指令名仍然是指令色
    expect(withClass(text, "cm-test-keyword")).toEqual(["include", "worker_processes"]);
  });
});

describe("nginxLanguage tokens", () => {
  it("colors directives at statement start, not arguments", () => {
    const text = [
      "user  nginx;",
      "worker_processes  auto;",
      "events {",
      "    worker_connections  1024;",
      "}",
      "http {",
      "    include /etc/nginx/mime.types;",
      "    default_type application/octet-stream;",
      "    log_format main '$remote_addr - $status';",
      "    access_log /var/log/nginx/access.log main;",
      "    client_max_body_size 300m;",
      "}",
    ].join("\n");
    expect(withClass(text, "cm-test-keyword")).toEqual([
      "user", "worker_processes", "events", "worker_connections", "http",
      "include", "default_type", "log_format", "access_log", "client_max_body_size",
    ]);
    expect(withClass(text, "cm-test-number")).toEqual(["1024", "300m"]);
  });

  it("colors variables and string interpolation", () => {
    expect(withClass("set $target http://h;", "cm-test-variable")).toEqual(["$target"]);
    expect(withClass('proxy_set_header Host $host;', "cm-test-variable")).toEqual(["$host"]);
    expect(withClass("rewrite ^/a/(.*)$ /b/$1 permanent;", "cm-test-variable")).toEqual(["$1"]);
    expect(withClass("log_format main 'ua=$http_user_agent';", "cm-test-variable")).toEqual(["$http_user_agent"]);
  });

  it("colors quoted strings, including multi-line log_format values", () => {
    const text = [
      "log_format main '$remote_addr - $remote_user [$time_local]'",
      "                '$status $body_bytes_sent';",
    ].join("\n");
    expect(withClass(text, "cm-test-string").length).toBeGreaterThan(0);
    expect(withClass(text, "cm-test-comment")).toEqual([]);
  });

  it("colors if conditions with operators", () => {
    const text = "if ($request_method = POST) { return 405; }";
    expect(withClass(text, "cm-test-keyword")).toEqual(["if", "return"]);
    expect(withClass(text, "cm-test-variable")).toEqual(["$request_method"]);
  });
});
