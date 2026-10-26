// @vitest-environment happy-dom
// SftpTreePane 组件测试：文件夹/文件单击语义、caret 阻断、双击打开、右键、
// 键盘（Enter/Space/方向键/roving focus）、改名提交/取消、选中/当前态与空态；
// 以及 Info 详情卡（250ms 悬停/聚焦、懒加载缓存）与拖放目标（高亮、1s 悬停
// 展开、drop-on 同步上抛）。
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import SftpTreePane from "./SftpTreePane.vue";
import type { SftpEntryKind, SftpSanitizedEntry } from "../lib/entries/sftpEntries";
import type { SftpFileTreeNode, SftpTreeRow } from "../lib/tree/sftpFileTree";
import type { SftpDetailInfo } from "../lib/actions/sftpDetails";
import type { HostPointerSample } from "../../../shared/lib/hostPointer";

export const t = (key: string, values?: Record<string, string | number>) => {
  if (key === "sftpSide.root") return "ROOT";
  const template =
    key === "sftpSide.expandNode"
      ? "expand {name}"
      : key === "sftpSide.collapseNode"
        ? "collapse {name}"
        : key;
  return template.replace(/\{(\w+)\}/g, (_match, name: string) => String(values?.[name] ?? `{${name}}`));
};

export function makeEntry(path: string, kind: SftpEntryKind, lossy = false): SftpSanitizedEntry {
  const name = path === "/" ? "/" : path.slice(path.lastIndexOf("/") + 1);
  return { name, uri: `sftp:${path}`, kind, ...(lossy ? { lossy: true } : {}) };
}

export function makeNode(
  path: string,
  kind: SftpEntryKind = "file",
  overrides: Partial<SftpFileTreeNode> = {},
): SftpFileTreeNode {
  const entry = makeEntry(path, kind, overrides.entry?.lossy);
  return {
    path,
    name: entry.name,
    kind,
    entry: { ...entry, ...overrides.entry },
    expanded: false,
    loaded: false,
    loading: false,
    children: [],
    ...overrides,
  };
}

export function row(node: SftpFileTreeNode, depth = 0): SftpTreeRow {
  return { node, depth };
}

export interface PaneProps {
  rows: SftpTreeRow[];
  selectedUris: Set<string>;
  selectedPath: string;
  currentPath: string;
  renamingPath: string;
  renameDraft: string;
  renameSubmitting: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  virtualize: boolean;
  emptyText: string;
  loadDetails?: (entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>;
  externalDragActive?: boolean;
  pointerSampler?: () => Promise<HostPointerSample | null>;
  followPadding?: boolean;
}

export function mountPane(rows: SftpTreeRow[], overrides: Partial<Omit<PaneProps, "rows">> = {}) {
  const props: PaneProps = {
    rows,
    selectedUris: new Set<string>(),
    selectedPath: "",
    currentPath: "/",
    renamingPath: "",
    renameDraft: "",
    renameSubmitting: false,
    t,
    locale: "en",
    virtualize: false,
    emptyText: "",
    externalDragActive: false,
    ...overrides,
  };
  return mount(SftpTreePane, { attachTo: document.body, props });
}


/** 详情卡里 loadDetails 的 promise 在 fake timers 下需要手动冲洗微任务。 */
export async function flushMicrotasks(times = 6): Promise<void> {
  for (let index = 0; index < times; index += 1) await Promise.resolve();
  await nextTick();
}

/**
 * DragEvent-like 事件：happy-dom 的 dataTransfer 只读，直接 defineProperty
 * 注入最小桩；dispatch 时 currentTarget 由浏览器补齐。
 */
export function dispatchDrag(
  target: Element,
  type: string,
  options: { types?: string[]; relatedTarget?: Node | null } = {},
): DragEvent {
  const event = new Event(type, {
    bubbles: true,
    cancelable: type !== "dragleave" && type !== "dragend",
  }) as DragEvent;
  Object.defineProperty(event, "dataTransfer", {
    value: {
      types: options.types ?? ["Files"],
      items: [],
      files: [],
      getData: () => "",
      setData: () => undefined,
    },
  });
  Object.defineProperty(event, "relatedTarget", { value: options.relatedTarget ?? null });
  target.dispatchEvent(event);
  return event;
}

export function detailsCard(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>(".sftp-detail-card");
}

