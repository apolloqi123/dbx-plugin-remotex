// Makefile 着色：此前 language-data 无对应语言，整篇按纯文本显示。
import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { HighlightStyle, ensureSyntaxTree } from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { makefileLanguage } from "./makefileMode";

const style = HighlightStyle.define([
  { tag: tags.comment, class: "cm-test-comment" },
  { tag: tags.variableName, class: "cm-test-variable" },
  { tag: tags.propertyName, class: "cm-test-target" },
  { tag: tags.keyword, class: "cm-test-keyword" },
]);

function highlight(text: string, extension: Extension = makefileLanguage()) {
  const state = EditorState.create({ doc: text, extensions: [extension] });
  const tree = ensureSyntaxTree(state, text.length, 10_000);
  const ranges: Array<{ text: string; classes: string[] }> = [];
  if (!tree) return ranges;
  highlightTree(tree, style, (from, to, classes) => {
    ranges.push({ text: text.slice(from, to), classes: classes.split(" ") });
  });
  return ranges;
}

function withClass(text: string, className: string): string[] {
  return highlight(text)
    .filter((range) => range.classes.includes(className))
    .map((range) => range.text);
}

describe("makefileLanguage", () => {
  it("colors comments, targets, assignments, variables and directives", () => {
    const text = [
      "# build config",
      "CC := gcc",
      "CFLAGS = -O2 -Wall",
      ".PHONY: all clean",
      "all: main.o util.o",
      "\t$(CC) $(CFLAGS) -o app main.o",
      "include common.mk",
    ].join("\n");

    expect(withClass(text, "cm-test-comment")).toEqual(["# build config"]);
    expect(withClass(text, "cm-test-variable")).toEqual(["CC", "CFLAGS", "$(CC)", "$(CFLAGS)"]);
    expect(withClass(text, "cm-test-target")).toEqual([".PHONY", "all"]);
    expect(withClass(text, "cm-test-keyword")).toEqual(["include"]);
  });

  it("treats recipe lines as shell instead of make assignments", () => {
    const text = "\t./configure --prefix=$(PREFIX) # note";
    expect(withClass(text, "cm-test-variable")).toEqual(["$(PREFIX)"]);
    expect(withClass(text, "cm-test-comment")).toEqual(["# note"]);
  });

  it("keeps # inside variable references out of comments", () => {
    expect(withClass("HASH := $(shell echo '#')", "cm-test-comment")).toEqual([]);
    // make 的注释可出现在行内任意位置（变量引用内部除外）。
    expect(withClass("VALUE = a#b", "cm-test-comment")).toEqual(["#b"]);
    expect(withClass("VALUE = a # note", "cm-test-comment")).toEqual(["# note"]);
  });
});
