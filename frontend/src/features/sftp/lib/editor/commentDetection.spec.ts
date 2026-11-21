// 跨格式「注释识别」回归表。
//
// 用户反馈过两类误判：nginx 的 `modules/*.conf` 被 legacy 模式当成 C 注释并
// 吞掉后续全文；shell 的 `${x#y}`、URL 里的 `#` 被当成注释。这里对常见格式做
// 统一约束：
//   1. 任何被判为注释的片段，必须以该语言的注释起始标记开头（否则说明词法
//      状态泄漏或误判，例如整段被吞）；
//   2. 真实注释必须被识别出来（expected）。
//
// 新增格式支持时在表里加一行即可。已知未覆盖：Perl 的 `s#...#` 引号类运算符
// （legacy perl 模式只认 `[\^'"!~/]` 作为分隔符，`#` 会落进注释分支）。
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { HighlightStyle, ensureSyntaxTree } from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { languages } from "@codemirror/language-data";
import { detectEditorLanguage } from "./languageDetect";
import { loadLanguageSupport } from "./languageSupport";

const style = HighlightStyle.define([{ tag: tags.comment, class: "cm-audit-comment" }]);

interface FormatCase {
  file: string;
  /** 该语言允许的注释起始标记（词首 #、//、/*、--、;、!、% …）。 */
  markers: string[];
  /** 必须被识别为注释的片段。 */
  expected: string[];
  text: string;
}

const CASES: FormatCase[] = [
  {
    file: "nginx.conf",
    markers: ["#"],
    expected: ["# real"],
    text: [
      "include /usr/share/nginx/modules/*.conf;",
      "http {",
      "    include /etc/nginx/mime.types;",
      "    log_format main '$remote_addr \"$request\" $status';",
      "    proxy_pass http://h/a#b;",
      "}",
      "# real",
    ].join("\n"),
  },
  {
    file: "a.sh",
    markers: ["#"],
    expected: ["# real"],
    text: '#!/bin/bash\nname=${path##*/}\ncount=$#\nurl=http://h/a#b\ncase a#b) : ;;\necho hi # real\n',
  },
  {
    file: "Makefile",
    markers: ["#"],
    expected: ["# real"],
    text: "VAR := $(shell echo '#')\nall: x\n\t$(CC) -o y # real\n",
  },
  {
    file: "Dockerfile",
    markers: ["#"],
    expected: ["# real"],
    text: 'FROM node:20\nRUN curl -fsSL https://x/s.sh | bash - && \\\n    echo "a#b" > /tmp/x\nENV URL=http://x/y#z\n# real\n',
  },
  {
    file: "app.ini",
    markers: ["#", ";", "!"],
    expected: ["# real", "; real", "! real"],
    text: "# real\n; real\n! real\nkey=value # inline\nurl=http://x/y#z\n[sec]\n",
  },
  {
    file: "a.tcl",
    markers: ["#"],
    expected: ["# real"],
    text: 'puts "value $a"\nif {$a > 1} { puts ok }\nset a 1 ;# real\n',
  },
  {
    file: "a.ps1",
    markers: ["#", "<#"],
    expected: ["# real"],
    text: 'Write-Host "Hello $env:USER"\n$a = "#x"\nGet-ChildItem -Path C:\\temp # real\n',
  },
  {
    file: "CMakeLists.txt",
    markers: ["#"],
    expected: ["# real"],
    text: 'set(SRC "a#b.cpp")\nadd_executable(app main.cpp) # real\n',
  },
  {
    file: "Jenkinsfile",
    markers: ["//", "/*"],
    expected: ["// real", "/* block */"],
    text: 'def url = "http://x/y#z"\n// real\n/* block */\n',
  },
  {
    file: "a.kt",
    markers: ["//", "/*"],
    expected: ["// real", "/* block */"],
    text: 'val url = "http://x/y#z" // real\n/* block */\n',
  },
  {
    file: "a.swift",
    markers: ["//", "/*"],
    expected: ["// real", "/* block */"],
    text: 'let url = "http://x/y#z" // real\n/* block */\n',
  },
  {
    file: "a.rb",
    markers: ["#"],
    expected: ["# real"],
    text: 'url = "http://x/y#z"\nputs "hi #{name}" # real\n',
  },
  {
    file: "a.lua",
    markers: ["--"],
    expected: ["-- real", "--[[ block ]]"],
    text: 'local url = "http://x/y#z"\nprint(url) -- real\n--[[ block ]]\n',
  },
  {
    file: "Cargo.toml",
    markers: ["#"],
    expected: ["# real"],
    text: 'url = "http://x/y#z"\nkey = 1 # real\n[table]\n',
  },
  {
    // 已知缺口：legacy ProtoBuf 模式不识别 `/* */` 块注释（proto3 风格指南推荐 `//`）。
    file: "a.proto",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'syntax = "proto3";\n// real\noption java_package = "com.x";\nstring url = 1;\n',
  },
  {
    file: "a.r",
    markers: ["#"],
    expected: ["# real"],
    text: 'url <- "http://x/y#z"\nprint(url) # real\n',
  },
  {
    // 原始 HTTP 报文没有注释语法；这里只断言 `#` 不会被误判。
    file: "a.http",
    markers: ["#"],
    expected: [],
    text: "GET /a#b HTTP/1.1\nHost: example.com\n# not-a-comment\n",
  },
  {
    file: "site.pp",
    markers: ["#"],
    expected: ["# real"],
    text: "file { '/tmp/x':\n  content => \"a#b\",\n  # real\n}\n",
  },
  {
    file: "a.erl",
    markers: ["%"],
    expected: ["% real"],
    text: '-module(demo).\nurl() -> "http://x/y#z". % real\n',
  },
  {
    file: "a.hs",
    markers: ["--", "{-"],
    expected: ["-- real", "{- block -}"],
    text: 'url = "http://x/y#z"\nmain = putStrLn url -- real\n{- block -}\n',
  },
  {
    file: "a.ml",
    markers: ["(*"],
    expected: ["(* block *)"],
    text: 'let url = "http://x/y#z" (* block *)\n',
  },
  {
    file: "a.scala",
    markers: ["//", "/*"],
    expected: ["// real", "/* block */"],
    text: 'val url = "http://x/y#z" // real\n/* block */\n',
  },
  {
    file: "a.dart",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'var url = "http://x/y#z"; // real\n',
  },
  {
    file: "a.vb",
    markers: ["'", "REM"],
    expected: ["' real"],
    text: 'Dim url = "http://x/y#z" \' real\n',
  },
  {
    file: "a.f90",
    markers: ["!"],
    expected: ["! real"],
    text: "print *, 'a#b' ! real\n",
  },
  {
    file: "a.tex",
    markers: ["%"],
    expected: ["% real"],
    text: "Text % real\n\\url{http://x/y#z}\n",
  },
  {
    file: "a.jl",
    markers: ["#"],
    expected: ["# real"],
    text: 'url = "http://x/y#z" # real\n',
  },
  {
    file: "a.clj",
    markers: [";"],
    expected: ["; real"],
    text: '(def url "http://x/y#z") ; real\n',
  },
  {
    file: "a.pas",
    markers: ["//", "{"],
    expected: ["// real"],
    text: "writeln('a#b'); // real\n",
  },
  {
    file: "a.coffee",
    markers: ["#", "###"],
    expected: ["# real"],
    text: 'url = "http://x/y#z" # real\n',
  },
  {
    file: "a.elm",
    markers: ["--", "{-"],
    expected: ["-- real"],
    text: 'url = "http://x/y#z" -- real\n',
  },
  {
    file: "a.cr",
    markers: ["#"],
    expected: ["# real"],
    text: 'url = "http://x/y#z" # real\n',
  },
  {
    file: "a.hx",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'var url = "http://x/y#z"; // real\n',
  },
  {
    file: "a.v",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: "// real\nmodule top; endmodule\n",
  },
  {
    file: "a.vhd",
    markers: ["--"],
    expected: ["-- real"],
    text: "-- real\nsignal a : std_logic;\n",
  },
  {
    file: "a.styl",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: "// real\nbody { color #fff }\n",
  },
  {
    file: "a.vm",
    markers: ["##", "#*"],
    expected: ["## real"],
    text: '#set($x = 1)\n## real\nurl = "http://x/y#z"\n',
  },
  {
    file: "a.st",
    markers: ['"'],
    expected: ['"real"'],
    text: "x := 'a#b'. \"real\"\n",
  },
  {
    // `.m` 在语言目录里同时属于 Mathematica/Octave/Objective-C，用 `.mm` 避免歧义。
    file: "a.mm",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'NSString *u = @"http://x/y#z"; // real\n',
  },
  {
    file: "a.cs",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'var u = "http://x/y#z"; // real\n',
  },
  {
    file: "a.go",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'u := "http://x/y#z" // real\n',
  },
  {
    file: "a.rs",
    markers: ["//", "/*"],
    expected: ["// real"],
    text: 'let u = "http://x/y#z"; // real\n',
  },
  {
    file: "a.py",
    markers: ["#"],
    expected: ["# real"],
    text: 'u = "http://x/y#z"  # real\n',
  },
  {
    file: "a.sql",
    markers: ["--", "/*", "#"],
    expected: ["-- real"],
    text: "select 'http://x/y#z'; -- real\n",
  },
  {
    file: "a.yaml",
    markers: ["#"],
    expected: ["# real"],
    text: 'u: "http://x/y#z" # real\n',
  },
  {
    file: "a.xml",
    markers: ["<!--"],
    expected: ["<!-- real -->"],
    text: '<?xml version="1.0"?>\n<a href="http://x/y#z"/><!-- real -->\n',
  },
  {
    file: "a.md",
    markers: ["<!--"],
    expected: ["<!-- real -->"],
    text: "# Title\n[link](http://x/y#z)\n<!-- real -->\n",
  },
  {
    file: "a.css",
    markers: ["/*"],
    expected: ["/* real */"],
    text: ".a { background: url(http://x/y#z); } /* real */\n",
  },
];

async function commentTexts(file: string, text: string): Promise<string[]> {
  const support = await loadLanguageSupport(detectEditorLanguage(file, text, languages));
  const state = EditorState.create({ doc: text, extensions: support ? [support] : [] });
  const tree = ensureSyntaxTree(state, text.length, 10_000);
  expect(tree, `${file}: parse tree missing`).not.toBeNull();
  const out: string[] = [];
  highlightTree(tree!, style, (from, to, classes) => {
    if (classes.split(" ").includes("cm-audit-comment")) out.push(text.slice(from, to));
  });
  return out;
}

describe("comment detection across formats", () => {
  it.each(CASES)("$file", async ({ file, markers, expected, text }) => {
    const comments = await commentTexts(file, text);
    // 1) 不允许出现「没有注释起始标记却被判为注释」的片段（状态泄漏/误判）。
    const suspicious = comments
      .map((comment) => comment.trim())
      .filter((comment) => comment.length > 0 && !markers.some((marker) => comment.startsWith(marker)));
    expect(suspicious, `${file}: 非注释片段被标成注释`).toEqual([]);
    // 2) 真实注释必须被识别。
    for (const fragment of expected) {
      expect(comments.some((comment) => comment.includes(fragment)), `${file}: 漏掉注释 ${fragment}`).toBe(true);
    }
  });
});
