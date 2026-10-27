import type { Ref } from "vue";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import { sanitizePaneDirection } from "../../../shared/lib/paneLayout";
import { readPluginMode } from "../../../shared/lib/pluginContext";
import { pluginStore } from "../../../shared/lib/pluginStore";
import { normalizeRemotePath } from "../../../features/sftp/lib/paths/sftpRemotePaths";
import {
  COLUMN_MIN_WIDTHS,
  COLUMN_WIDTH_MAX,
  DEFAULT_VISIBLE_COLUMNS,
  NAME_COLUMN_MAX,
  NAME_COLUMN_MIN,
  sanitizeVisibleColumns,
  type SftpColumn,
} from "../../../features/sftp/lib/entries/sftpEntries";
import { createPersistThrottle } from "../../lib/persistThrottle";
import { readWorkbenchUiMirror, writeWorkbenchUiMirror } from "../../lib/workbenchUiMirror";
import {
  resolveSftpPaneOpen,
  resolveDirectoryFollow,
  sanitizeWorkbenchLayoutPrefs,
  type SshWorkbenchPaneOrder,
} from "../../lib/workbenchLayout";

/** 偏好里允许出现的排序列（与 App 的 SftpSortColumn 同集合）。 */
type SftpSortColumn = "name" | "size" | "modified";

export interface WorkbenchState {
  sessionId?: string;
  terminalSequence?: number;
  sftpPath?: string;
  followDirectory?: boolean;
  /** 目录跟随默认开启（2026-10-27）迁移标记：无此标记的旧状态不采用其中的 false。 */
  followDefaultOn?: boolean;
  sudoMode?: boolean;
  splitRatio?: number;
  paneOrder?: SshWorkbenchPaneOrder;
  sftpPaneOpen?: boolean;
  visibleColumns?: SftpColumn[];
  sftpColumnWidths?: Partial<Record<SftpColumn, number>>;
  sftpNameWidth?: number | null;
  /** 列配置版本标记：六列默认（issue #35）上线后的一次性迁移，老偏好重置为全开。 */
  columnsV2?: boolean;
  /** RDP 会话内用户选择的桌面尺寸（P0-2）：恢复 tab 时优先于连接表单 rdp_resolution。 */
  rdpWidth?: number;
  rdpHeight?: number;
  /** SFTP 隐藏文件显隐（issue #178）：此前完全未持久化。 */
  sftpShowHidden?: boolean;
  /** SFTP 列表排序规则（issue #178）：此前完全未持久化。 */
  sftpSort?: { column: SftpSortColumn; direction: "asc" | "desc" };
}

// 跨重启的布局/视图全局偏好（issue #178）：单键 JSON，记录最近一次的
// paneOrder / splitRatio / SFTP 面板开合 / 列显隐与列宽 / 显示隐藏文件 / 排序。
// writeWorkbenchState 写穿（persistState 防抖统一触发），restoreUiState 启动
// 水合为新工作台初始值；workbenchState 会话恢复字段存在时仍优先覆盖。
const WORKBENCH_LAYOUT_KEY = "ssh-workbench-layout";

/**
 * 工作台跨重启持久化：布局/视图全局偏好档、UI 镜像双写与宿主 workbenchState
 * 桥的写穿/恢复。状态本体（布局 refs、会话、终端序列号、RDP 桌面尺寸）仍由
 * App 持有，全部经 options 注入；序列号是 `let`，走 getter/setter。
 */
export function useWorkbenchPersistence(options: {
  hostContext: Ref<Record<string, unknown>>;
  session: Ref<{ sessionId: string } | undefined>;
  currentPath: Ref<string>;
  splitRatio: Ref<number>;
  paneOrder: Ref<SshWorkbenchPaneOrder>;
  paneDirection: Ref<PaneDirectionPref>;
  sftpPaneOpen: Ref<boolean>;
  followDirectory: Ref<boolean>;
  sudoMode: Ref<boolean>;
  canWrite: Ref<boolean>;
  visibleColumns: Ref<SftpColumn[]>;
  sftpColumnWidths: Record<SftpColumn, number>;
  sftpNameWidth: Ref<number | null>;
  sftpShowHidden: Ref<boolean>;
  sort: Ref<{ column: SftpSortColumn; direction: "asc" | "desc" }>;
  panelSurface: Ref<boolean>;
  rdpPersistedSize: Ref<{ width: number; height: number } | null>;
  lastSequence: () => number;
  setLastSequence: (value: number) => void;
  loadDirectoryFollowPref: () => boolean;
}) {
  const {
    hostContext, session, currentPath, splitRatio, paneOrder, paneDirection,
    sftpPaneOpen, followDirectory, sudoMode, canWrite, visibleColumns,
    sftpColumnWidths, sftpNameWidth, sftpShowHidden, sort, panelSurface,
    rdpPersistedSize, lastSequence, setLastSequence, loadDirectoryFollowPref,
  } = options;

  function initialState(): WorkbenchState {
    const value = hostContext.value.workbenchState;
    return value && typeof value === "object" ? (value as WorkbenchState) : {};
  }

  /** 水合跨重启的布局/视图全局偏好（issue #178）：作为新工作台的初始值——
   *  字段缺省保持硬编码默认，随后 workbenchState 的会话恢复字段存在时覆盖。 */
  function applyWorkbenchLayoutPrefs() {
    let prefs = null;
    try {
      prefs = sanitizeWorkbenchLayoutPrefs(JSON.parse(pluginStore.getItem(WORKBENCH_LAYOUT_KEY) ?? "null"));
    } catch {
      prefs = null;
    }
    if (!prefs) return;
    if (prefs.paneOrder) paneOrder.value = prefs.paneOrder;
    if (prefs.paneDirection) paneDirection.value = sanitizePaneDirection(prefs.paneDirection);
    if (prefs.splitRatio !== undefined) splitRatio.value = prefs.splitRatio;
    // 面板开合记的是用户最近一次的手动切换；偏好里没有时维持设置页的
    // 「默认打开」偏好（sftpPaneOpen ref 的初始值即来自它）。
    if (prefs.sftpPaneOpen !== undefined) sftpPaneOpen.value = prefs.sftpPaneOpen;
    if (prefs.visibleColumns) visibleColumns.value = sanitizeVisibleColumns(prefs.visibleColumns);
    if (prefs.sftpColumnWidths) {
      const allCols: SftpColumn[] = ["size", "modified", "owner", "group", "permissions"];
      for (const col of allCols) {
        const num = prefs.sftpColumnWidths[col];
        if (typeof num === "number") sftpColumnWidths[col] = Math.max(COLUMN_MIN_WIDTHS[col], Math.min(COLUMN_WIDTH_MAX, num));
      }
    }
    if (prefs.sftpNameWidth !== undefined) sftpNameWidth.value = Math.max(NAME_COLUMN_MIN, Math.min(NAME_COLUMN_MAX, prefs.sftpNameWidth));
    if (prefs.showHidden !== undefined) sftpShowHidden.value = prefs.showHidden;
    if (prefs.sort) sort.value = prefs.sort;
  }

  function restoreUiState() {
    // 两层布局持久化（issue #178 双方案合并）：1) pluginStore 全局偏好
    // （ssh-workbench-layout）先水合为新工作台初始值；2) UI 镜像
    // （ssh-workbench-ui，0.7.6-beta.1 起）作底层默认、initialState 作权威
    // 覆盖——宿主缺 workbenchState 桥时 initialState 恒空对象，镜像整组生效；
    // 桥在时每工作台状态照旧优先，镜像只补它没有的键。
    applyWorkbenchLayoutPrefs();
    const state = { ...readWorkbenchUiMirror(), ...initialState() };
    currentPath.value = typeof state.sftpPath === "string" ? normalizeRemotePath(state.sftpPath) : "/";
    splitRatio.value = typeof state.splitRatio === "number" && state.splitRatio >= 35 && state.splitRatio <= 80 ? state.splitRatio : splitRatio.value;
    paneOrder.value = state.paneOrder === "sftp-left" ? "sftp-left" : state.paneOrder === "terminal-left" ? "terminal-left" : paneOrder.value;
    // Dock panel surface：SFTP 面板默认不开（不自动列目录/不自动建 SFTP 会话，
    // dock 保持轻量）；需要文件面板时用 panel-actions 的开关手动打开（连接建立时
    // loadDirectory 本就会拉列表，打开即有数据）。
    sftpPaneOpen.value = panelSurface.value ? false : resolveSftpPaneOpen(state, sftpPaneOpen.value);
    // 本地终端工作台没有 SFTP 语义：连接less tab 的恢复态/布局镜像若带开合
    // 记录，会在 !localUiMode 门控下留下幽灵分栏（模板 v-if 只显式防了 RDP），
    // 恢复层一律收起。hostContext 在本函数之前已水合（initialize 7787）。
    if (readPluginMode(hostContext.value) === "local-terminal") sftpPaneOpen.value = false;
    // Dock panel surface：目录跟随是 SFTP 面板能力，面板一律关闭。
    // 2026-10-27 起目录跟随默认开启：旧工作台状态没有 followDefaultOn 标记时，
    // 其中自动落盘的 false 不再生效，统一按新默认恢复；标记落盘后按用户最近
    // 一次显式选择恢复。
    const followDefault = loadDirectoryFollowPref();
    followDirectory.value = panelSurface.value
      ? false
      : state.followDefaultOn === true
        ? resolveDirectoryFollow(state, followDefault)
        : followDefault;
    sudoMode.value = state.sudoMode === true && canWrite.value;
    // 一次性迁移：六列默认上线前的旧偏好重置为全开（之后用户自定义照常持久化）。
    const legacyColumns = state.visibleColumns != null && state.columnsV2 !== true;
    if (legacyColumns) visibleColumns.value = [...DEFAULT_VISIBLE_COLUMNS];
    else if (state.visibleColumns != null) visibleColumns.value = sanitizeVisibleColumns(state.visibleColumns);
    // 恢复列宽，未知列或越界用默认值兜底；state 无该列宽度时保持当前值
    // （布局偏好层已写入的值或默认值）。
    const saved = state.sftpColumnWidths || {};
    const allCols: SftpColumn[] = ["size", "modified", "owner", "group", "permissions"];
    for (const col of allCols) {
      const raw = saved[col];
      if (typeof raw === "number") sftpColumnWidths[col] = Math.max(COLUMN_MIN_WIDTHS[col], Math.min(COLUMN_WIDTH_MAX, raw));
    }
    if (typeof state.sftpNameWidth === "number") sftpNameWidth.value = Math.max(NAME_COLUMN_MIN, Math.min(NAME_COLUMN_MAX, state.sftpNameWidth));
    // 镜像/桥里若带 showHidden/sort（ssh-workbench-ui 布局子集，beta.1 起），
    // 字段存在即覆盖偏好水合值；缺字段时保持全局偏好或硬编码默认。
    if (typeof state.sftpShowHidden === "boolean") sftpShowHidden.value = state.sftpShowHidden;
    const savedSort = state.sftpSort;
    if (savedSort && (savedSort.column === "name" || savedSort.column === "size" || savedSort.column === "modified")) {
      sort.value = { column: savedSort.column, direction: savedSort.direction === "desc" ? "desc" : "asc" };
    }
    setLastSequence(typeof state.terminalSequence === "number" ? state.terminalSequence : 0);
    // RDP 会话内选择的桌面尺寸（P0-2）：仅在契约门限内采用。
    const rdpSavedWidth = typeof state.rdpWidth === "number" ? state.rdpWidth : 0;
    const rdpSavedHeight = typeof state.rdpHeight === "number" ? state.rdpHeight : 0;
    rdpPersistedSize.value = rdpSavedWidth >= 640 && rdpSavedWidth <= 3840 && rdpSavedHeight >= 480 && rdpSavedHeight <= 2160
      ? { width: rdpSavedWidth, height: rdpSavedHeight }
      : null;
  }

  function writeWorkbenchState() {
    const uiMirror = {
      sftpPath: currentPath.value,
      followDirectory: followDirectory.value,
      followDefaultOn: true,
      sudoMode: sudoMode.value,
      splitRatio: splitRatio.value,
      paneOrder: paneOrder.value,
      sftpPaneOpen: sftpPaneOpen.value,
      visibleColumns: visibleColumns.value,
      columnsV2: true,
      sftpColumnWidths: { ...sftpColumnWidths } as Record<string, number>,
      sftpNameWidth: sftpNameWidth.value,
      sftpShowHidden: sftpShowHidden.value,
      sftpSort: { column: sort.value.column, direction: sort.value.direction },
    };
    // 布局镜像双写（issue #178）：宿主缺 workbenchState 桥时它是唯一持久层；
    // 桥在时镜像只作恢复兜底（见 restoreUiState 的合并方向）。
    writeWorkbenchUiMirror(uiMirror);
    // 同一载荷写穿跨重启的全局布局偏好（ssh-workbench-layout）：workbenchState 按
    // sessionId 键控、重启即孤儿化，这里把最近一次布局/视图状态另存一份作新工作台初始值。
    try {
      pluginStore.setItem(WORKBENCH_LAYOUT_KEY, JSON.stringify({
        paneOrder: paneOrder.value,
        paneDirection: paneDirection.value,
        splitRatio: splitRatio.value,
        sftpPaneOpen: sftpPaneOpen.value,
        visibleColumns: visibleColumns.value,
        sftpColumnWidths: { ...sftpColumnWidths },
        sftpNameWidth: sftpNameWidth.value,
        showHidden: sftpShowHidden.value,
        sort: sort.value,
      }));
    } catch {
      // 存储不可用（沙箱降级链耗尽）：会话恢复态照写，全局偏好退化为本次会话。
    }
    // 宿主桥可能尚未注入（window.dbxPlugin 未定义）或为旧宿主（无
    // workbenchState）：返回 resolved promise 而不是解引用抛 TypeError。
    // persistThrottle 虽会吞掉落盘异常，但这里仍保持 writeWorkbenchState 的
    // Promise 契约，避免每次调度都走异常路径。
    const workbenchState = window.dbxPlugin?.workbenchState;
    if (!workbenchState) return Promise.resolve();
    return workbenchState.set({
      sessionId: session.value?.sessionId,
      terminalSequence: lastSequence(),
      ...uiMirror,
      sftpColumnWidths: { ...sftpColumnWidths },
      ...(rdpPersistedSize.value ? { rdpWidth: rdpPersistedSize.value.width, rdpHeight: rdpPersistedSize.value.height } : {}),
    }).catch(() => undefined);
  }

  /**
   * 工作台状态落盘节流：终端持续输出时 drain 每帧都会调用 persistState()——
   * 原 150 ms 防抖会在连续输出下被无限重置，导致序列号/布局永不落盘。
   * 现语义：空闲期首次调用立即落盘；连续调用合并到 150 ms；任何挂起变更自
   * 首次变更起最多 1 s 必强制落盘（不会被输出饿死）。
   */
  const persistThrottle = createPersistThrottle(
    () => {
      void writeWorkbenchState();
    },
    { intervalMs: 150, maxWaitMs: 1000 },
  );

  function persistState() {
    persistThrottle.schedule();
  }

  return {
    initialState,
    applyWorkbenchLayoutPrefs,
    restoreUiState,
    writeWorkbenchState,
    persistThrottle,
    persistState,
  };
}
