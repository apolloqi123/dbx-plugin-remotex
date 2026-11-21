// Tcl 着色回归：legacy tcl 模式把双引号字符串与运算符都标成注释。
import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { HighlightStyle, ensureSyntaxTree } from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { tclLanguage } from "./tclMode";

const style = HighlightStyle.define([
  { tag: tags.comment, class: "cm-test-comment" },
  { tag: tags.keyword, class: "cm-test-keyword" },
  { tag: tags.variableName, class: "cm-test-variable" },
  { tag: tags.string, class: "cm-test-string" },
]);

function ranges(text: string, extension: Extension = tclLanguage()) {
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

describe("tclLanguage comments", () => {
  it("never marks quotes or operators as comments", () => {
    expect(withClass('puts "value $a"', "cm-test-comment")).toEqual([]);
    expect(withClass("if {$a > 1} { puts ok }", "cm-test-comment")).toEqual([]);
    expect(withClass('set url "http://h/a#b"', "cm-test-comment")).toEqual([]);
  });

  it("recognizes comments at command start only", () => {
    expect(withClass("# whole line", "cm-test-comment")).toEqual(["# whole line"]);
    expect(withClass("  # indented", "cm-test-comment")).toEqual(["# indented"]);
    expect(withClass("set a 1 ;# trailing", "cm-test-comment")).toEqual(["# trailing"]);
    // 行中 `#` 是普通字符，不是注释
    expect(withClass("set a # not a comment", "cm-test-comment")).toEqual([]);
    expect(withClass("set a 1#2", "cm-test-comment")).toEqual([]);
  });

  it("handles comments inside proc bodies", () => {
    const text = ["proc foo {} {", "    # body comment", "    puts hi", "}"].join("\n");
    expect(withClass(text, "cm-test-comment")).toEqual(["# body comment"]);
  });
});

describe("tclLanguage tokens", () => {
  it("colors strings, variables and builtins", () => {
    expect(withClass('puts "hello $name"', "cm-test-string").length).toBeGreaterThan(0);
    expect(withClass('puts "hello $name"', "cm-test-variable")).toEqual(["$name"]);
    expect(withClass("set x ${value}", "cm-test-variable")).toEqual(["${value}"]);
    expect(withClass("if {$a} { puts ok }", "cm-test-keyword")).toEqual(["if", "puts"]);
    // 行中词不是命令起始位置，不着关键字色
    expect(withClass("set a if", "cm-test-keyword")).toEqual(["set"]);
  });
});
