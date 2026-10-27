// Internationalization entry point. Locale blocks and feature supplemental
// tables are split under i18n/messages; this module only composes them.
import enMessages from "./messages/base/en";
import esMessages from "./messages/base/es";
import itMessages from "./messages/base/it";
import jaMessages from "./messages/base/ja";
import ptBRMessages from "./messages/base/pt-BR";
import zhCNMessages from "./messages/base/zh-CN";
import zhTWMessages from "./messages/base/zh-TW";
import common from "./messages/supplemental/common";
import transferUi from "./messages/supplemental/transferUi";
import uploadBridge from "./messages/supplemental/uploadBridge";
import folderUpload from "./messages/supplemental/folderUpload";
import dropGate from "./messages/supplemental/dropGate";
import terminalAppearance from "./messages/supplemental/terminalAppearance";
import terminalFontMoved from "./messages/supplemental/terminalFontMoved";
import quickCommandsManage from "./messages/supplemental/quickCommandsManage";
import terminalBehaviorEn from "./messages/supplemental/terminalBehavior.en";
import terminalBehaviorZhCN from "./messages/supplemental/terminalBehavior.zh-CN";
import terminalBehaviorZhTW from "./messages/supplemental/terminalBehavior.zh-TW";
import terminalBehaviorEs from "./messages/supplemental/terminalBehavior.es";
import terminalBehaviorIt from "./messages/supplemental/terminalBehavior.it";
import terminalBehaviorJa from "./messages/supplemental/terminalBehavior.ja";
import terminalBehaviorPtBR from "./messages/supplemental/terminalBehavior.pt-BR";
import restored from "./messages/supplemental/restored";
import acceleratorMonitor from "./messages/supplemental/acceleratorMonitor";
import metricsDashboard from "./messages/supplemental/metricsDashboard";
import transfersPanel from "./messages/supplemental/transfersPanel";
import suggestionTransfer from "./messages/supplemental/suggestionTransfer";
import sftpPipeline from "./messages/supplemental/sftpPipeline";
import connNameEncoding from "./messages/supplemental/connNameEncoding";
import telnet from "./messages/supplemental/telnet";
import x11 from "./messages/supplemental/x11";
import startupCommands from "./messages/supplemental/startupCommands";
import otpPanel from "./messages/supplemental/otpPanel";
import importWizard from "./messages/supplemental/importWizard";
import serial from "./messages/supplemental/serial";
import vnc from "./messages/supplemental/vnc";
import rdp from "./messages/supplemental/rdp";
import completionMenu from "./messages/supplemental/completionMenu";
import terminalGhost from "./messages/supplemental/terminalGhost";
import historyPanelSettings from "./messages/supplemental/historyPanelSettings";
import serialUpload from "./messages/supplemental/serialUpload";
import serialEnhance from "./messages/supplemental/serialEnhance";
import sudoDownload from "./messages/supplemental/sudoDownload";
import jsonPreview from "./messages/supplemental/jsonPreview";
import terminalOsc from "./messages/supplemental/terminalOsc";
import terminalHistoryPanel from "./messages/supplemental/terminalHistoryPanel";
import transferCompress from "./messages/supplemental/transferCompress";
import sftpWorkbenchEnhancements from "./messages/supplemental/sftpWorkbenchEnhancements";
import terminalTabs from "./messages/supplemental/terminalTabs";
import editorFind from "./messages/supplemental/editorFind";

export type WorkbenchLocale = "en" | "es" | "it" | "ja" | "pt-BR" | "zh-CN" | "zh-TW";

export const messages = {
  en: enMessages,
  es: esMessages,
  it: itMessages,
  ja: jaMessages,
  "pt-BR": ptBRMessages,
  "zh-CN": zhCNMessages,
  "zh-TW": zhTWMessages,
} as const;

export const supplemental: Record<string, Record<string, string>> = { ...common };

/** terminalBehaviour 表按 locale 拆分为 sibling 文件；这里按原 locale 顺序
 *  重新组装成一张表，合并语义（spread 顺序 / last-wins）保持不变。 */
const terminalBehavior: Record<string, Record<string, string>> = {
  en: terminalBehaviorEn,
  "zh-CN": terminalBehaviorZhCN,
  "zh-TW": terminalBehaviorZhTW,
  es: terminalBehaviorEs,
  it: terminalBehaviorIt,
  ja: terminalBehaviorJa,
  "pt-BR": terminalBehaviorPtBR,
};

const supplementalTables: Array<Record<string, Record<string, string>>> = [
transferUi,
uploadBridge,
folderUpload,
dropGate,
terminalAppearance,
terminalFontMoved,
quickCommandsManage,
terminalBehavior,
restored,
acceleratorMonitor,
metricsDashboard,
transfersPanel,
suggestionTransfer,
sftpPipeline,
connNameEncoding,
telnet,
x11,
startupCommands,
otpPanel,
importWizard,
serial,
vnc,
rdp,
completionMenu,
terminalGhost,
historyPanelSettings,
serialUpload,
serialEnhance,
sudoDownload,
jsonPreview,
terminalOsc,
terminalHistoryPanel,
transferCompress,
sftpWorkbenchEnhancements,
terminalTabs,
editorFind,
];
for (const table of supplementalTables) {
  for (const locale of Object.keys(table)) {
    supplemental[locale] = { ...(supplemental[locale] ?? {}), ...table[locale] };
  }
}

/** Dev/test helper: flattens a nested message table into dotted `a.b` keys. */
function flattenMessageTable(table: unknown, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  if (!table || typeof table !== "object") return out;
  for (const [key, value] of Object.entries(table as Record<string, unknown>)) {
    const dotted = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object") flattenMessageTable(value, dotted, out);
    else if (typeof value === "string") out[dotted] = value;
  }
  return out;
}
/**
 * Dev/test helper: the full `key -> template` table for one locale
 * (`messages` + `supplemental` merged). Lets tests compare the key sets of
 * all seven locales so a missing translation cannot slip in silently.
 */
export function workbenchMessageTable(locale: WorkbenchLocale): Record<string, string> {
  return { ...flattenMessageTable(messages[locale]), ...(supplemental[locale] ?? {}) };
}
export function resolveWorkbenchLocale(locale: string): WorkbenchLocale {
  const normalized = locale.replace("_", "-");
  if (normalized.toLowerCase().startsWith("zh-tw") || normalized.toLowerCase().startsWith("zh-hk")) return "zh-TW";
  if (normalized.toLowerCase().startsWith("zh")) return "zh-CN";
  if (normalized.toLowerCase().startsWith("pt")) return "pt-BR";
  const language = normalized.split("-")[0] as WorkbenchLocale;
  return language in messages ? language : "zh-CN";
}

/**
 * 每个 locale 的扁平查找表（懒构建、进程内缓存）。App.vue 单次渲染约 500 个
 * t() 调用，原实现对每次调用做 3 次 key.split(".")+reduce 深遍历；扁平化后
 * 单次调用是一次 Map.get。
 *
 * 优先级与嵌套实现保持一致（低 → 高写入，后者覆盖）：
 * supplemental.en → supplemental[locale] → base.en → base[locale]。
 */
const flatLookupCache = new Map<WorkbenchLocale, Map<string, string>>();

function flatLookupTable(locale: WorkbenchLocale): Map<string, string> {
  const cached = flatLookupCache.get(locale);
  if (cached) return cached;
  const table = new Map<string, string>();
  const merge = (source: Record<string, string> | undefined) => {
    if (!source) return;
    for (const key in source) table.set(key, source[key]);
  };
  merge(supplemental.en);
  if (locale !== "en") merge(supplemental[locale]);
  merge(flattenMessageTable(messages.en));
  if (locale !== "en") merge(flattenMessageTable(messages[locale]));
  flatLookupCache.set(locale, table);
  return table;
}

/** 快速 locale 归一：已是合法 locale 时跳过字符串规范化（热路径）。 */
function resolveLocaleFast(locale: string): WorkbenchLocale {
  return (locale in messages ? locale : resolveWorkbenchLocale(locale)) as WorkbenchLocale;
}

const PLACEHOLDER_PATTERN = /\{(\w+)\}/g;

export function workbenchMessage(locale: string, key: string, values: Record<string, string | number> = {}): string {
  const resolved = resolveLocaleFast(locale);
  const template = flatLookupTable(resolved).get(key) ?? key;
  // 无插值占位符（绝大多数 key）→ 跳过正则替换。
  if (template.indexOf("{") === -1) return template;
  return template.replace(PLACEHOLDER_PATTERN, (_match, name: string) => String(values[name] ?? `{${name}}`));
}
