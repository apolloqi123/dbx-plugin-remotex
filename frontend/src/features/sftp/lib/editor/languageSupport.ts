// 检测结果 → 实际注入编辑器的语言扩展。
//
// 插件自带实现优先（Shell/Nginx/Tcl 修掉 legacy 模式的注释误判，Makefile 目录
// 里本来就没有）；其余走 language-data 的懒加载，加载失败静默降级为纯文本。

import type { Extension } from "@codemirror/state";
import type { DetectedEditorLanguage } from "./languageDetect";
import { makefileLanguage } from "./makefileMode";
import { nginxLanguage } from "./nginxMode";
import { shellLanguage } from "./shellMode";
import { tclLanguage } from "./tclMode";

export async function loadLanguageSupport(
  detected: DetectedEditorLanguage,
): Promise<Extension | undefined> {
  if (detected.override === "shell") return shellLanguage();
  if (detected.override === "makefile") return makefileLanguage();
  if (detected.override === "nginx") return nginxLanguage();
  if (detected.override === "tcl") return tclLanguage();
  if (!detected.language) return undefined;
  return detected.language.load().catch(() => undefined);
}
