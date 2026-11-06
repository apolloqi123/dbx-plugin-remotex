// 设置导航每个分类的图标（lucide 图标 + 组件类型）。
// 单独成文件是为了让 SettingsDialog.vue 只保留编排逻辑：图标表是纯常量，
// 但把它留在组件里会连带 17 个图标 import 一起占行数预算。
import type { Component } from "vue";
import {
  Bot,
  Download,
  Highlighter,
  Keyboard,
  Lock,
  Monitor,
  Paintbrush,
  Palette,
  Plug,
  ShieldCheck,
  Sparkles,
  Terminal as TerminalIcon,
  Zap,
} from "@lucide/vue";

/** 分类图标只做识别与亮色点缀；颜色由 cards.css 的分类规则统一给出。 */
export const CATEGORY_ICONS: Record<string, Component> = {
  appearance: Palette,
  scheme: Paintbrush,
  terminal: TerminalIcon,
  suggestions: Sparkles,
  highlight: Highlighter,
  commands: Zap,
  ai: Bot,
  hotkeys: Keyboard,
  sudo: ShieldCheck,
  agent: Monitor,
  transfer: Download,
  security: Lock,
  mcp: Plug,
};
