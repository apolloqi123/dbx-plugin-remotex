// Shell 着色回归：`#` 只在「词首」开启注释（POSIX/bash 实测规则），词中 `#`
// 必须保持普通文字色——否则 `${path##*/}`、`$#`、URL 片段整段显示成注释灰。
import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { HighlightStyle, ensureSyntaxTree } from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { shellLanguage } from "./shellMode";

const style = HighlightStyle.define([
  { tag: tags.comment, class: "cm-test-comment" },
  { tag: tags.variableName, class: "cm-test-variable" },
  { tag: tags.keyword, class: "cm-test-keyword" },
]);

interface HighlightRange {
  from: number;
  to: number;
  classes: string[];
}

function highlight(text: string, extension: Extension = shellLanguage()): HighlightRange[] {
  const state = EditorState.create({ doc: text, extensions: [extension] });
  const tree = ensureSyntaxTree(state, text.length, 10_000);
  const ranges: HighlightRange[] = [];
  if (!tree) return ranges;
  highlightTree(tree, style, (from, to, classes) => {
    ranges.push({ from, to, classes: classes.split(" ") });
  });
  return ranges;
}

/** 被判定为注释的文本片段。 */
function comments(text: string): string[] {
  return highlight(text)
    .filter((range) => range.classes.includes("cm-test-comment"))
    .map((range) => text.slice(range.from, range.to));
}

/** 被判定为变量的文本片段。 */
function variables(text: string): string[] {
  return highlight(text)
    .filter((range) => range.classes.includes("cm-test-variable"))
    .map((range) => text.slice(range.from, range.to));
}

describe("shellLanguage comments", () => {
  it("starts a comment at a word boundary", () => {
    expect(comments("echo hi # trailing note")).toEqual(["# trailing note"]);
    expect(comments("echo hi;# after semicolon")).toEqual(["# after semicolon"]);
    expect(comments("true && # after operator")).toEqual(["# after operator"]);
    expect(comments("# whole line")).toEqual(["# whole line"]);
    expect(comments("#!/usr/bin/env bash")).toEqual(["#!/usr/bin/env bash"]);
  });

  it("keeps in-word # out of comments (the false grey lines)", () => {
    // 参数个数 / 数组长度
    expect(comments("count=$#")).toEqual([]);
    expect(comments("args=${#arr[@]}")).toEqual([]);
    // 参数扩展里的模式截断运算符
    expect(comments("name=${path##*/}")).toEqual([]);
    expect(comments("base=${file%%.*}")).toEqual([]);
    // 词中 #（URL 片段、case 模式、赋值）
    expect(comments("url=http://host/a#b")).toEqual([]);
    expect(comments("echo a#b")).toEqual([]);
    expect(comments("x=#value")).toEqual([]);
  });

  it("keeps # inside quotes and heredocs as text", () => {
    expect(comments('echo "a # b"')).toEqual([]);
    expect(comments("echo 'a # b'")).toEqual([]);
    expect(comments("cat <<EOF\n# not a comment\nEOF\n# real comment")).toEqual(["# real comment"]);
    expect(comments("cat <<-'EOF'\n\t# body\n\tEOF\n# real")).toEqual(["# real"]);
  });

  it("still highlights real comments after strings with interpolation", () => {
    expect(comments('echo "${user:-root} # inside" # tail')).toEqual(["# tail"]);
  });
});

describe("shellLanguage tokens", () => {
  it("colors variables, expansions and assignments", () => {
    expect(variables("cmd $HOME $1 $@ $# $? $$")).toEqual([
      "$HOME",
      "$1",
      "$@",
      "$#",
      "$?",
      "$$",
    ]);
    expect(variables("ls ${path##*/}")).toEqual(["${path##*/}"]);
    expect(variables("PATH=/usr/bin:/bin")).toEqual(["PATH"]);
  });

  it("colors keywords and builtins but leaves external commands plain", () => {
    const classes = new Map<string, string[]>();
    const text = "if [ -f x ]; then echo ok; fi";
    for (const range of highlight(text)) {
      classes.set(text.slice(range.from, range.to), range.classes);
    }
    expect(classes.get("if")).toContain("cm-test-keyword");
    expect(classes.get("fi")).toContain("cm-test-keyword");
    // `[` / 外部命令不加关键字色
    expect(classes.get("[")).toBeUndefined();
  });
});
