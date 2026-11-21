// 自带 StreamLanguage 模式的健壮性：不卡死（CodeMirror 会抛 "Stream parser
// failed to advance stream"）、解析覆盖全文、随机输入不崩、增量重解析能恢复状态。
import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { makefileLanguage } from "./makefileMode";
import { nginxLanguage } from "./nginxMode";
import { shellLanguage } from "./shellMode";
import { tclLanguage } from "./tclMode";

const MODES: Array<[string, Extension]> = [
  ["shell", shellLanguage()],
  ["makefile", makefileLanguage()],
  ["nginx", nginxLanguage()],
  ["tcl", tclLanguage()],
];

const tricky = [
  "",
  "\n\n\n",
  "$",
  "$#",
  "${",
  "${x",
  "$(",
  "$((",
  "$(echo \"a)b\")",
  "`",
  "`echo $x`",
  "`echo \"`\"`",
  "'",
  "\"",
  "\"$",
  "\"${x}\"",
  "'a'b'c'",
  "\"a\"#b",
  "echo a#b",
  "#",
  "#!/bin/sh",
  "cat <<EOF\n",
  "cat <<EOF\nEOF\n",
  "cat <<-EOF\n\t\n\tEOF\nx",
  "cat <<'E O F'\nE O F\n",
  "cat <<EOF\n#x\nEOF\n# y",
  "cat <<EOF\r\n#x\r\nEOF\r\n# y",
  "x=1 \\\n  y=2",
  "a=(1 2 3)",
  "case $x in #c\nesac",
  "\\",
  "\\\n",
  "\r\n#c\r\n",
  "\u0000#",
  "if [ \"$a\" = b ]; then echo ok; fi",
  "echo ${x//a/b} ${x#p} ${#arr[@]} $# $@ $$ $? $- $! $0 $1",
  "$((1+2)) $(echo hi) $(echo \"a)b\")",
  "function f() { :; }",
  "echo \"$(\"nested\")\"",
  "echo `echo \\`x\\``",
  "include /a/*.conf;",
  "location ~* \\.(gif|jpg)$ { }",
  "log_format main '$a \"$b\"' '$c';",
  "set $x \"a#b\"",
  "rewrite ^/a/(.*)$ /b/$1;",
  "if ($x = y) { }",
  "puts \"value $a\"",
  "set a 1 ;# c",
  "proc f {} { # c\n}",
  "if {$a > 1} { puts ok }",
  "set x ${a} $b(idx)",
];

function parse(extension: Extension, text: string) {
  const state = EditorState.create({ doc: text, extensions: [extension] });
  return ensureSyntaxTree(state, text.length, 5000);
}

/** StreamLanguage 的树长按行拼接，CRLF 的 `\r` 不计入（legacy 模式同样如此）。 */
function expectedLength(text: string): number {
  return text.length - (text.match(/\r\n/g)?.length ?? 0);
}

describe("stream mode robustness", () => {
  it("parses tricky inputs in every bundled mode without stalling", () => {
    for (const [name, extension] of MODES) {
      for (const text of tricky) {
        const tree = parse(extension, text);
        expect(tree, `${name}: ${JSON.stringify(text)}`).not.toBeNull();
        expect(tree!.length, `${name}: ${JSON.stringify(text)}`).toBe(expectedLength(text));
      }
    }
  });

  it("survives random fuzz input", () => {
    const alphabet = ["#", "$", "{", "}", "(", ")", "'", '"', "`", "<", ">", "\\", "\n", " ", "a", "1", "=", ";", "|", ":", "\t", "*", "/", "[", "]"];
    let seed = 42;
    const rand = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let round = 0; round < 400; round += 1) {
      let text = "";
      const length = Math.floor(rand() * 120);
      for (let i = 0; i < length; i += 1) text += alphabet[Math.floor(rand() * alphabet.length)];
      for (const [name, extension] of MODES) {
        const tree = parse(extension, text);
        expect(tree, `${name}: ${JSON.stringify(text)}`).not.toBeNull();
        expect(tree!.length).toBe(expectedLength(text));
      }
    }
  });

  it("recovers state after edits (copyState keeps quote/heredoc/statement state)", () => {
    const samples: Array<[string, Extension]> = [
      ["cat <<EOF\n# body\nEOF\n", shellLanguage()],
      ["include /a/*.conf;\nworker_processes auto;\n", nginxLanguage()],
      ["puts \"a $b\"\nset a 1\n", tclLanguage()],
      ["VAR := $(shell echo '#')\nall: x\n", makefileLanguage()],
    ];
    for (const [doc, extension] of samples) {
      let state = EditorState.create({ doc, extensions: [extension] });
      expect(ensureSyntaxTree(state, doc.length, 5000)).not.toBeNull();
      // 在文件开头插入一行，触发从中间片段恢复状态（copyState 路径）。
      state = state.update({ changes: { from: 0, insert: "echo hi\n" } }).state;
      const tree = ensureSyntaxTree(state, state.doc.length, 5000);
      expect(tree).not.toBeNull();
      expect(tree!.length).toBe(state.doc.length);
    }
  });
});
