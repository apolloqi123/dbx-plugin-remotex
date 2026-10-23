// 整文件夹覆盖判定的纯函数单测：顶层根目录提取（去重/排序/跳过裸文件）、
// 遍历段归一（`..` 只回弹不越界）与目标路径拼接（非法根返回空串）。
import { describe, expect, it } from "vitest";
import { folderOverwriteTarget, folderUploadRootPaths } from "./folderOverwrite";

describe("folderUploadRootPaths", () => {
  it("extracts the first segment of every relative path, deduped and sorted", () => {
    const roots = folderUploadRootPaths([
      { relativePath: "photos/a.jpg" },
      { relativePath: "photos/b/c.jpg" },
      { relativePath: "docs/readme.md" },
      { relativePath: "photos/d.jpg" },
    ]);
    expect(roots).toEqual(["docs", "photos"]);
  });

  it("normalizes Windows separators, dot segments and dotdot backtracking", () => {
    const roots = folderUploadRootPaths([
      { relativePath: "photos\\a.jpg" },
      { relativePath: "photos/./sub/../b.jpg" },
      { relativePath: "a/../c/d.jpg" },
      { relativePath: "  spaced  /x.jpg" },
    ]);
    expect(roots).toEqual(["c", "photos", "spaced"]);
  });

  it("never yields empty, dot, dotdot, absolute or bare-file roots", () => {
    const roots = folderUploadRootPaths([
      { relativePath: "" },
      { relativePath: "." },
      { relativePath: ".." },
      { relativePath: "/" },
      { relativePath: "a.txt" },
      { relativePath: ".." },
      { relativePath: "../escape.jpg" },
      { relativePath: "a/../../b.jpg" },
    ]);
    // `../escape.jpg` 归一为 `escape.jpg`（栈空回弹），它是裸文件、无根目录；
    // `a/../../b.jpg` 归一为 `b.jpg`，同样没有目录段。
    expect(roots).toEqual([]);
  });

  it("keeps a root reachable only through a legal directory component", () => {
    expect(folderUploadRootPaths([{ relativePath: "deep/../photos/a.jpg" }])).toEqual(["photos"]);
    expect(folderUploadRootPaths([])).toEqual([]);
  });
});

describe("folderOverwriteTarget", () => {
  // 与 App.vue 的 joinRemote 同款归一：父目录去尾斜杠、根目录去首斜杠。
  const join = (parent: string, name: string) => `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/${name.replace(/^\/+/, "")}`;

  it("joins the validated root under the base directory", () => {
    expect(folderOverwriteTarget("/target", "photos", join)).toBe("/target/photos");
    expect(folderOverwriteTarget("/target/", "photos", join)).toBe("/target/photos");
    expect(folderOverwriteTarget("/", "photos", join)).toBe("/photos");
  });

  it("rejects roots that are empty, dot, dotdot, absolute or multi-segment", () => {
    for (const root of ["", ".", "..", "/", "/etc", "a/b", "a\\b", "   "]) {
      expect(folderOverwriteTarget("/target", root, join)).toBe("");
    }
  });

  it("trims surrounding whitespace on a legal root", () => {
    expect(folderOverwriteTarget("/target", " photos ", join)).toBe("/target/photos");
  });
});
