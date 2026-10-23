<script setup lang="ts">
// SFTP 单栏文件树（文件夹 + 文件同列）：渲染层只负责展示与交互上抛，
// 节点状态、懒加载与过滤由父级通过 sftpFileTree.ts 维护。虚拟滚动沿用
// 文件列表的 computeWindow（28px 行高 + overscan），测试可关掉窗口化。
// Info 详情卡用 Teleport 挂在 body 上（fixed + pointer-events:none），不进入
// 虚拟列表布局；目录行同时是宿主文件拖放目标（高亮 + 悬停 2s 懒展开）。
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  ChevronDown,
  ChevronRight,
  File as FileIcon,
  FileText,
  Folder,
  FolderOpen,
  Info,
  Link2,
  TriangleAlert,
} from "@lucide/vue";
import type { SftpSanitizedEntry } from "../lib/sftpEntries";
import type { SftpFileTreeNode, SftpTreeRow } from "../lib/sftpFileTree";
import {
  mergeSftpDetail,
  sftpDetailFromEntry,
  sftpDetailRows,
  type SftpDetailInfo,
  type SftpDetailRow,
} from "../lib/sftpDetails";
import { computeWindow } from "../lib/virtualWindow";

const ROW_HEIGHT = 28;
const OVERSCAN = 8;
const DETAIL_HOVER_DELAY = 250;
const DROP_EXPAND_DELAY = 2_000;
const CARD_FALLBACK_WIDTH = 276;
const CARD_FALLBACK_HEIGHT = 196;
const CARD_GAP = 6;
const VIEWPORT_MARGIN = 4;

const props = withDefaults(
  defineProps<{
    rows: SftpTreeRow[];
    selectedUris: Set<string>;
    selectedPath: string;
    currentPath: string;
    renamingPath: string;
    renameDraft: string;
    renameSubmitting: boolean;
    t: (key: string, values?: Record<string, string | number>) => string;
    locale?: string;
    /** 测试可关闭窗口化，一次渲染全部行；生产默认开启。 */
    virtualize?: boolean;
    /** 空态文案（如搜索无结果时传 sftpSearch.noMatch）。 */
    emptyText?: string;
    /** 树列表默认 includeOwner:false，info 图标悬停时按需补齐 owner/group 等详情。 */
    loadDetails?: (entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>;
    /** 宿主桥已判定文件拖拽进行中：部分宿主会剥离 dataTransfer.types，
     *  行级 dragover 不能只依赖 DOM types，否则高亮/悬停展开失效。 */
    externalDragActive?: boolean;
  }>(),
  { virtualize: true, externalDragActive: false },
);

const emit = defineEmits<{
  (event: "toggle", node: SftpFileTreeNode): void;
  (event: "select", entry: SftpSanitizedEntry, sourceEvent: MouseEvent | KeyboardEvent): void;
  (event: "open", node: SftpFileTreeNode): void;
  (event: "context", node: SftpFileTreeNode, mouseEvent: MouseEvent): void;
  (event: "rename-commit", node: SftpFileTreeNode, value: string): void;
  (event: "rename-cancel"): void;
  (event: "row-keydown", node: SftpFileTreeNode, keyEvent: KeyboardEvent): void;
  (event: "drop-on", node: SftpFileTreeNode, dragEvent: DragEvent): void;
  (event: "drop-target-change", path: string): void;
}>();

interface RenderedRow extends SftpTreeRow {
  index: number;
}

const scrollEl = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);
let resizeObserver: ResizeObserver | undefined;

const virtualWindow = computed(() =>
  computeWindow({
    scrollTop: scrollTop.value,
    viewportHeight: viewportHeight.value,
    rowHeight: ROW_HEIGHT,
    total: props.rows.length,
    overscan: OVERSCAN,
  }),
);

const renderedRows = computed<RenderedRow[]>(() => {
  if (!props.virtualize) return props.rows.map((row, index) => ({ ...row, index }));
  const window = virtualWindow.value;
  return props.rows.slice(window.start, window.end).map((row, offset) => ({
    ...row,
    index: window.start + offset,
  }));
});

const resolvedEmptyText = computed(() => props.emptyText || props.t("emptyFolder"));

// 虚拟窗口内实际存在的行：roving tabindex 的兜底只能落在可见行上，
// 否则滚动/过滤后整个树会没有 Tab 停靠点。
const renderedPathSet = computed(() => new Set(renderedRows.value.map((row) => row.node.path)));
const firstRenderedIndex = computed(() => renderedRows.value[0]?.index ?? 0);

function onScroll(event: Event) {
  scrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
  hideDetails();
}

function measureViewport() {
  if (scrollEl.value) viewportHeight.value = scrollEl.value.clientHeight;
}

onMounted(() => {
  measureViewport();
  if (typeof ResizeObserver !== "undefined" && scrollEl.value) {
    resizeObserver = new ResizeObserver(measureViewport);
    resizeObserver.observe(scrollEl.value);
  }
  window.addEventListener("dragend", endDragSession);
  window.addEventListener("scroll", hideDetails, { passive: true });
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  resizeObserver = undefined;
  window.removeEventListener("dragend", endDragSession);
  window.removeEventListener("scroll", hideDetails);
  stopPointerTracking();
  clearDragTarget();
  hideDetails();
});

// 行被过滤/折叠后总高变短：把旧的 scrollTop 夹回有效范围，避免窗口
// start 越过 total 渲染出空列表。
watch(
  () => props.rows.length,
  (total) => {
    const max = Math.max(0, total * ROW_HEIGHT - viewportHeight.value);
    if (scrollTop.value <= max) return;
    scrollTop.value = max;
    if (scrollEl.value) scrollEl.value.scrollTop = max;
  },
);

function displayName(node: SftpFileTreeNode): string {
  return node.name === "/" ? props.t("sftpSide.root") : node.name;
}

function caretName(node: SftpFileTreeNode): string {
  const name = displayName(node);
  return node.expanded
    ? props.t("sftpSide.collapseNode", { name })
    : props.t("sftpSide.expandNode", { name });
}

function isSelected(node: SftpFileTreeNode): boolean {
  return props.selectedPath === node.entry.uri || props.selectedUris.has(node.entry.uri);
}

// 改名草稿：父级持有 renameDraft，输入期间用本地副本保证受控且不写 prop。
const renameValue = ref("");
let renameSettled = false;

watch(
  () => props.renamingPath,
  (path) => {
    if (!path) return;
    renameValue.value = props.renameDraft;
    renameSettled = false;
  },
  { immediate: true },
);

function onRenameInput(event: Event) {
  renameValue.value = (event.target as HTMLInputElement).value;
}

function commitRename(node: SftpFileTreeNode) {
  if (renameSettled) return;
  renameSettled = true;
  if (props.renameSubmitting) return;
  emit("rename-commit", node, renameValue.value);
}

function cancelRename() {
  if (renameSettled) return;
  renameSettled = true;
  emit("rename-cancel");
}

// Roving tabindex：优先记住最后聚焦行，其次当前目录，最后首行兜底。
const activePath = ref("");

function tabIndexFor(node: SftpFileTreeNode, index: number): number {
  if (activePath.value && renderedPathSet.value.has(activePath.value)) {
    return node.path === activePath.value ? 0 : -1;
  }
  if (renderedPathSet.value.has(props.currentPath)) {
    return node.path === props.currentPath ? 0 : -1;
  }
  return index === firstRenderedIndex.value ? 0 : -1;
}

function onRowFocus(node: SftpFileTreeNode) {
  activePath.value = node.path;
}

function rowIndexOf(node: SftpFileTreeNode): number {
  const exact = props.rows.findIndex((row) => row.node === node);
  if (exact >= 0) return exact;
  return props.rows.findIndex((row) => row.node.path === node.path);
}

function findRowElement(index: number): HTMLElement | null {
  return scrollEl.value?.querySelector<HTMLElement>(`[data-tree-index="${index}"]`) ?? null;
}

/** 聚焦第 index 行；目标不在虚拟窗口内时先滚动到它，下一帧再聚焦。 */
function focusRowAt(index: number): boolean {
  if (index < 0 || index >= props.rows.length) return false;
  const existing = findRowElement(index);
  if (existing) {
    existing.focus();
    return true;
  }
  if (!props.virtualize || !scrollEl.value) return false;
  const maxTop = Math.max(0, props.rows.length * ROW_HEIGHT - viewportHeight.value);
  const top = Math.max(0, Math.min(maxTop, index * ROW_HEIGHT - ROW_HEIGHT));
  scrollTop.value = top;
  scrollEl.value.scrollTop = top;
  void nextTick(() => findRowElement(index)?.focus());
  return true;
}

function moveRowFocus(node: SftpFileTreeNode, offset: number): boolean {
  const index = rowIndexOf(node);
  return index >= 0 ? focusRowAt(index + offset) : false;
}

function focusParentRow(node: SftpFileTreeNode): boolean {
  const index = rowIndexOf(node);
  if (index < 0) return false;
  const depth = props.rows[index].depth;
  for (let candidate = index - 1; candidate >= 0; candidate -= 1) {
    if (props.rows[candidate].depth < depth) return focusRowAt(candidate);
  }
  return false;
}

function onRowClick(node: SftpFileTreeNode, event: MouseEvent) {
  if (node.kind === "directory") {
    // 双击的第二次 click（detail > 1）忽略：首击已 toggle 一次，dblclick
    // 只负责 open，否则文件夹会先展开再收起（两次 toggle）。
    if (event.detail > 1) return;
    // 带修饰键单击文件夹 = 多选（Ctrl/Cmd 切换、Shift 范围），不展开；普通
    // 单击仍只负责展开/收起，不改变当前目录与选中态。
    if (event.ctrlKey || event.metaKey || event.shiftKey) {
      emit("select", node.entry, event);
      return;
    }
    emit("toggle", node);
    return;
  }
  emit("select", node.entry, event);
}

function onCaretClick(node: SftpFileTreeNode, event: MouseEvent) {
  // caret 双击是「展开 + 打开」手势的组成部分：第二次 click 不再 toggle，
  // 否则会先展开再收起（与行双击的 detail 守卫一致）。
  if (event.detail > 1) return;
  emit("toggle", node);
}

function onRowDblclick(node: SftpFileTreeNode) {
  emit("open", node);
}

function onRowKeydown(node: SftpFileTreeNode, event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    emit("open", node);
    return;
  }
  if (event.key === " ") {
    event.preventDefault();
    if (node.kind === "directory") {
      emit("toggle", node);
      return;
    }
    // Space 选中文件（与主流文件管理器一致）；改用 select 而不是把未处理
    // 按键丢给 row-keydown 的 no-op 路径。
    emit("select", node.entry, event);
    return;
  }
  if (event.key === "ArrowDown") {
    if (moveRowFocus(node, 1)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowUp") {
    if (moveRowFocus(node, -1)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowLeft") {
    if (node.kind === "directory" && node.expanded) {
      event.preventDefault();
      emit("toggle", node);
      return;
    }
    if (focusParentRow(node)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowRight") {
    if (node.kind === "directory" && !node.expanded) {
      event.preventDefault();
      emit("toggle", node);
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  // F2/Delete 由父级开始改名/删除；其余未处理键也原样上抛。
  emit("row-keydown", node, event);
}

// —— Info 详情卡 ——————————————————————————————————————————————
// 每个行尾的 info 图标在 hover/focus 250ms 后打开 fixed 卡片。loadDetails
// 只在缓存缺失（或 entry.modifiedAt 变化）时请求一次；成功后按 uri 缓存，
// 失败/返回 null 也缓存，卡片退回 entry 自带字段并标注 unavailable。

interface CachedDetail {
  modifiedAt: number | undefined;
  /** chmod 不一定更新 mtime：权限串变化也必须让缓存失效。 */
  permissions: string | undefined;
  info: SftpDetailInfo;
  unavailable: boolean;
}

const detailVisible = ref(false);
const detailNode = ref<SftpFileTreeNode | null>(null);
const detailInfo = ref<SftpDetailInfo | null>(null);
const detailLoading = ref(false);
const detailUnavailable = ref(false);
const detailAnchor = ref<HTMLElement | null>(null);
const detailCardEl = ref<HTMLElement | null>(null);
const detailCardStyle = ref<Record<string, string>>({});

const detailCache = new Map<string, CachedDetail>();
const detailInflight = new Map<string, Promise<Partial<SftpDetailInfo> | null>>();
let detailHoverTimer: ReturnType<typeof setTimeout> | undefined;
let detailToken = 0;
let detailShownUri = "";

// 树刷新/过滤/展开会重排行，锚元素可能被替换：直接收起，避免卡片悬空。
watch(
  () => props.rows,
  () => {
    if (detailVisible.value) hideDetails();
  },
);

const detailTitle = computed(() =>
  detailNode.value ? displayName(detailNode.value) : props.t("sftpDetails.title"),
);

const detailRows = computed<SftpDetailRow[]>(() =>
  detailInfo.value ? sftpDetailRows(detailInfo.value, props.t, props.locale || "") : [],
);

function cancelDetailTimer() {
  if (detailHoverTimer === undefined) return;
  clearTimeout(detailHoverTimer);
  detailHoverTimer = undefined;
}

function hideDetails() {
  cancelDetailTimer();
  detailVisible.value = false;
  detailLoading.value = false;
  detailToken += 1;
  detailShownUri = "";
}

function scheduleDetails(node: SftpFileTreeNode, anchor: HTMLElement | null) {
  cancelDetailTimer();
  if (!anchor) return;
  detailAnchor.value = anchor;
  detailHoverTimer = setTimeout(() => {
    detailHoverTimer = undefined;
    void openDetails(node);
  }, DETAIL_HOVER_DELAY);
}

function onInfoEnter(node: SftpFileTreeNode, event: MouseEvent) {
  scheduleDetails(node, event.currentTarget as HTMLElement | null);
}

function onInfoFocus(node: SftpFileTreeNode, event: FocusEvent) {
  scheduleDetails(node, event.currentTarget as HTMLElement | null);
}

function positionDetailCard() {
  const anchor = detailAnchor.value;
  if (!anchor) return;
  const rect = anchor.getBoundingClientRect();
  const width = detailCardEl.value?.offsetWidth || CARD_FALLBACK_WIDTH;
  const height = detailCardEl.value?.offsetHeight || CARD_FALLBACK_HEIGHT;
  const viewportWidth = window.innerWidth || document.documentElement.clientWidth || width;
  const viewportHeight = window.innerHeight || document.documentElement.clientHeight || height;

  // 默认贴图标右侧；右侧放不下翻到左侧，最后统一夹回视口（含 4px 边距）。
  let left = rect.right + CARD_GAP;
  if (left + width > viewportWidth - VIEWPORT_MARGIN) left = rect.left - width - CARD_GAP;
  left = Math.min(left, viewportWidth - width - VIEWPORT_MARGIN);
  left = Math.max(VIEWPORT_MARGIN, left);

  let top = rect.top;
  top = Math.min(top, viewportHeight - height - VIEWPORT_MARGIN);
  top = Math.max(VIEWPORT_MARGIN, top);

  detailCardStyle.value = {
    position: "fixed",
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
  };
}

async function openDetails(node: SftpFileTreeNode) {
  const entry = node.entry;
  const uri = entry.uri;
  const token = ++detailToken;
  detailShownUri = uri;
  detailNode.value = node;
  detailVisible.value = true;
  detailUnavailable.value = false;

  const cached = detailCache.get(uri);
  if (cached && cached.modifiedAt === entry.modifiedAt && cached.permissions === entry.permissions) {
    detailInfo.value = mergeSftpDetail(entry, cached.info, node.path);
    detailLoading.value = false;
    detailUnavailable.value = cached.unavailable;
    await nextTick();
    positionDetailCard();
    return;
  }

  const loadDetails = props.loadDetails;
  if (!loadDetails) {
    detailInfo.value = sftpDetailFromEntry(entry, node.path);
    detailLoading.value = false;
    await nextTick();
    positionDetailCard();
    return;
  }

  // 加载期间先展示 entry 上已有的字段，再加一行 loading，卡片不会先空一拍。
  detailInfo.value = sftpDetailFromEntry(entry, node.path);
  detailLoading.value = true;
  await nextTick();
  positionDetailCard();

  const inflightKey = `${uri}\u0000${entry.modifiedAt ?? ""}\u0000${entry.permissions ?? ""}`;
  let request = detailInflight.get(inflightKey);
  if (!request) {
    request = Promise.resolve()
      .then(() => loadDetails(entry))
      .catch(() => null);
    detailInflight.set(inflightKey, request);
    void request.finally(() => {
      if (detailInflight.get(inflightKey) === request) detailInflight.delete(inflightKey);
    });
  }

  const stat = await request;
  const merged = mergeSftpDetail(entry, stat, node.path);
  detailCache.set(uri, {
    modifiedAt: entry.modifiedAt,
    permissions: entry.permissions,
    info: merged,
    unavailable: stat == null,
  });
  // 卡片可能已关闭/换了目标：缓存照写，UI 只在当前 token 仍有效时更新。
  if (token !== detailToken || !detailVisible.value || detailShownUri !== uri) return;
  detailInfo.value = merged;
  detailLoading.value = false;
  detailUnavailable.value = stat == null;
  await nextTick();
  positionDetailCard();
}

// —— 文件拖放到目录行 ————————————————————————————————————————————
// 只有宿主文件拖拽（dataTransfer.types 含 "Files"）才高亮；折叠目录悬停
// 2s 后 emit toggle 由父级懒加载展开。drop 必须同步上抛原始 DragEvent，
// 父级要在同一 tick 里读取 dataTransfer.items。

const dragOverUri = ref("");
const dragOverPath = ref("");
let dropExpandTimer: ReturnType<typeof setTimeout> | undefined;
let dropExpandUri = "";
let dropExpandFiredUri = "";
let pointerTracking = false;
let lastPointerTrackAt = 0;
let hoverPollTimer: ReturnType<typeof setInterval> | undefined;
// 冻结的 :hover 会一直返回拖入时的行。仅当轮询看到「与上次不同的行」时才认为
// 指针真的移动了（live hover），用它兜底；同一行的重复上报一律忽略，避免
// 把 pointer/dragover 已命中的新目标（A 展开后拖到 B）覆盖回旧行。
let lastHoverUri = "";

// 宿主桥整包接管 drop 时不会把目标行带进 onDrop 回调；父级用这份路径把
// 「当前悬停的文件夹」作为宿主上传落点。
watch(dragOverPath, (path) => emit("drop-target-change", path));
// 宿主拖拽结束时 DOM 未必补发 dragleave：清掉高亮/计时器，但保留
// dragOverPath 供宿主紧接着的 onDrop 读取（部分宿主先发 false 再发 drop）。
watch(
  () => props.externalDragActive,
  (active) => {
    if (active) startPointerTracking();
    else {
      stopPointerTracking();
      clearDragVisuals();
    }
  },
  { immediate: true },
);

// 宿主桥只给 dragActive 布尔值，不保证把 dragover 派发到行元素。用全局
// 指针位置做兜底：拖拽期间按 elementFromPoint 找到行，复用同一套标记。
function trackPointer(event: MouseEvent) {
  if (!props.externalDragActive) return;
  if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) return;
  if (!event.clientX && !event.clientY) return;
  const now = performance.now();
  if (now - lastPointerTrackAt < 60) return;
  lastPointerTrackAt = now;
  if (typeof document.elementFromPoint !== "function") return;
  const element = document.elementFromPoint(event.clientX, event.clientY) as HTMLElement | null;
  const rowEl = element?.closest<HTMLElement>(".sftp-tree-row");
  if (!rowEl) {
    clearDragTarget();
    return;
  }
  const index = Number(rowEl.dataset.treeIndex);
  const target = props.rows[index];
  if (!target || target.node.kind !== "directory") {
    clearDragTarget();
    return;
  }
  markRowDragTarget(target.node);
}

function startPointerTracking() {
  if (pointerTracking || typeof document === "undefined") return;
  pointerTracking = true;
  lastHoverUri = "";
  document.addEventListener("mousemove", trackPointer, true);
  document.addEventListener("pointermove", trackPointer, true);
  document.addEventListener("dragover", trackPointer, true);
  // 另一道兜底：某些 WKWebView 宿主在原生拖拽期不派发鼠标/拖拽事件，但
  // CSS :hover 仍会随指针更新。轮询命中的行，不命中时保持现状（避免把
  // 已由 dragover/指针事件标记的目标误清）。
  hoverPollTimer = setInterval(() => {
    if (!props.externalDragActive) return;
    const rowEl = document.querySelector<HTMLElement>(".sftp-tree-row:hover");
    if (!rowEl) return;
    const index = Number(rowEl.dataset.treeIndex);
    const target = props.rows[index];
    if (!target || target.node.kind !== "directory") return;
    // 同一行的重复上报 = 冻结的 :hover（或指针没动），不能覆盖事件命中的目标。
    if (target.node.entry.uri === lastHoverUri) return;
    lastHoverUri = target.node.entry.uri;
    markRowDragTarget(target.node);
  }, 120);
}

function stopPointerTracking() {
  if (!pointerTracking) return;
  pointerTracking = false;
  lastHoverUri = "";
  document.removeEventListener("mousemove", trackPointer, true);
  document.removeEventListener("pointermove", trackPointer, true);
  document.removeEventListener("dragover", trackPointer, true);
  if (hoverPollTimer !== undefined) {
    clearInterval(hoverPollTimer);
    hoverPollTimer = undefined;
  }
}

function clearDropExpandTimer() {
  if (dropExpandTimer !== undefined) {
    clearTimeout(dropExpandTimer);
    dropExpandTimer = undefined;
  }
  dropExpandUri = "";
  dropExpandFiredUri = "";
}

function clearDragVisuals() {
  clearDropExpandTimer();
  dragOverUri.value = "";
}

function clearDragTarget() {
  clearDragVisuals();
  dragOverPath.value = "";
}

/** 整场拖拽结束：行间 dragleave 只清目标，lastHoverUri 必须留到会话结束，
 *  否则冻结的 :hover 会在下一次轮询重新生效。 */
function endDragSession() {
  clearDragTarget();
  lastHoverUri = "";
}

function isFileDrag(event: DragEvent): boolean {
  // 宿主桥的 dragActive 是权威的文件拖拽信号；DOM 事件的 types 在部分
  // WKWebView/沙箱拖拽里为空或不含 "Files"，不能单靠它。
  if (props.externalDragActive) return true;
  const types = event.dataTransfer?.types;
  if (!types) return false;
  const list = Array.from(types);
  // 空 types = 宿主沙箱把文件拖拽的类型列表剥离了，但仍会派发 dragover；
  // 文本/元素拖拽至少带 text/plain 或自定义类型，不会被误判。
  return list.includes("Files") || list.length === 0;
}

/** 行级落点标记：dragover 事件与宿主拖拽的指针兜底共用同一套高亮/2s 展开。 */
function markRowDragTarget(node: SftpFileTreeNode) {
  dragOverUri.value = node.entry.uri;
  dragOverPath.value = node.path;
  if (node.expanded || node.loading) {
    clearDropExpandTimer();
    return;
  }
  // 同一行的重复 dragover 不重启 2s 计时器；触发过一次后直到 dragleave/drop
  // 前不再重复 toggle（原生 dragover 在父级异步展开前仍会持续触发）。
  if (dropExpandFiredUri === node.entry.uri) return;
  if (dropExpandTimer !== undefined && dropExpandUri === node.entry.uri) return;
  clearDropExpandTimer();
  dropExpandUri = node.entry.uri;
  dropExpandTimer = setTimeout(() => {
    dropExpandTimer = undefined;
    dropExpandFiredUri = node.entry.uri;
    if (dragOverUri.value !== node.entry.uri) return;
    if (node.expanded || node.loading) return;
    emit("toggle", node);
  }, DROP_EXPAND_DELAY);
}

function onRowDragOver(node: SftpFileTreeNode, event: DragEvent) {
  if (node.kind !== "directory" || !isFileDrag(event)) return;
  // 允许 drop 落到行上（浏览器要求 dragover preventDefault）。
  event.preventDefault();
  markRowDragTarget(node);
}

// 树容器再兜一道：宿主若把 dragover 派发给行内子元素/容器而非行本身，
// 用事件坐标反查当前行，保证移动中的目标始终跟随指针。
function onTreeDragOver(event: DragEvent) {
  if (!props.externalDragActive) return;
  trackPointer(event);
}

function onRowDragLeave(node: SftpFileTreeNode, event: DragEvent) {
  if (node.kind !== "directory") return;
  // 行内子元素之间移动（caret/icon/name）不重置高亮与计时器。
  const current = event.currentTarget as Node | null;
  const related = event.relatedTarget as Node | null;
  if (related && current?.contains(related)) return;
  clearDragTarget();
}

function onRowDrop(node: SftpFileTreeNode, event: DragEvent) {
  if (node.kind !== "directory") return;
  event.preventDefault();
  event.stopPropagation();
  // 高亮/计时器立即收起，但保留 dragOverPath 到宿主 onDrop 读取：DBX 宿主
  // 可能在 DOM drop 之后另行回调 onDrop，落点仍需指向刚放下的文件夹。
  clearDragVisuals();
  emit("drop-on", node, event);
}
</script>

<template>
  <div
    ref="scrollEl"
    class="sftp-tree-pane"
    :lang="locale || undefined"
    @dragover="onTreeDragOver"
    @scroll.passive="onScroll"
  >
    <div class="sftp-tree-list" role="tree" :aria-label="t('sftpSide.tree')">
      <div
        v-if="virtualize && virtualWindow.padTop"
        class="sftp-tree-spacer"
        :style="{ height: `${virtualWindow.padTop}px` }"
        aria-hidden="true"
      ></div>
      <div
        v-for="row in renderedRows"
        :key="row.node.entry.uri"
        class="sftp-tree-row"
        :class="{
          'is-current': row.node.path === currentPath,
          'is-selected': isSelected(row.node),
          'is-file': row.node.kind !== 'directory',
          'is-drop-target': dragOverUri === row.node.entry.uri,
        }"
        role="treeitem"
        :aria-level="row.depth + 1"
        :aria-expanded="row.node.kind === 'directory' ? (row.node.expanded ? 'true' : 'false') : undefined"
        :aria-selected="isSelected(row.node) ? 'true' : 'false'"
        :data-tree-index="row.index"
        :tabindex="tabIndexFor(row.node, row.index)"
        :style="{ paddingLeft: `${8 + row.depth * 14}px` }"
        @click="onRowClick(row.node, $event)"
        @dblclick="onRowDblclick(row.node)"
        @focus="onRowFocus(row.node)"
        @keydown="onRowKeydown(row.node, $event)"
        @contextmenu="emit('context', row.node, $event)"
        @dragover="onRowDragOver(row.node, $event)"
        @dragleave="onRowDragLeave(row.node, $event)"
        @drop="onRowDrop(row.node, $event)"
      >
        <button
          v-if="row.node.kind === 'directory'"
          type="button"
          class="sftp-tree-caret"
          :aria-label="caretName(row.node)"
          @click.stop="onCaretClick(row.node, $event)"
          @dblclick.stop
          @keydown.stop
        >
          <span v-if="row.node.loading" class="sftp-tree-spinner" />
          <ChevronDown v-else-if="row.node.expanded" />
          <ChevronRight v-else />
        </button>
        <span v-else class="sftp-tree-caret-spacer" aria-hidden="true"></span>

        <template v-if="row.node.kind === 'directory'">
          <FolderOpen v-if="row.node.expanded" class="sftp-tree-dir-icon" />
          <Folder v-else class="sftp-tree-dir-icon" />
        </template>
        <Link2 v-else-if="row.node.kind === 'symlink'" class="sftp-tree-file-icon is-link" />
        <FileText v-else-if="row.node.kind === 'other'" class="sftp-tree-file-icon" />
        <FileIcon v-else class="sftp-tree-file-icon" />

        <input
          v-if="renamingPath === row.node.entry.uri"
          :value="renameValue"
          class="rename-input"
          :disabled="renameSubmitting"
          spellcheck="false"
          autofocus
          @input="onRenameInput"
          @click.stop
          @dblclick.stop
          @keydown.stop
          @keydown.enter.stop.prevent="commitRename(row.node)"
          @keydown.escape.stop.prevent="cancelRename"
          @blur="commitRename(row.node)"
        />
        <span v-else class="sftp-tree-name">{{ displayName(row.node) }}</span>
        <TriangleAlert
          v-if="row.node.entry.lossy"
          class="sftp-lossy-icon"
          :title="t('sftpName.lossyTitle')"
        />
        <button
          type="button"
          class="sftp-tree-info"
          :aria-label="t('sftpDetails.title')"
          @click.stop
          @dblclick.stop
          @keydown.stop
          @mouseenter="onInfoEnter(row.node, $event)"
          @mouseleave="hideDetails"
          @focus="onInfoFocus(row.node, $event)"
          @blur="hideDetails"
        >
          <Info />
        </button>
      </div>
      <div
        v-if="virtualize && virtualWindow.padBottom"
        class="sftp-tree-spacer"
        :style="{ height: `${virtualWindow.padBottom}px` }"
        aria-hidden="true"
      ></div>
      <div v-if="!rows.length" class="empty compact">{{ resolvedEmptyText }}</div>
    </div>

    <Teleport to="body">
      <div
        v-if="detailVisible && detailNode"
        ref="detailCardEl"
        class="sftp-detail-card"
        role="tooltip"
        :aria-label="t('sftpDetails.title')"
        :style="detailCardStyle"
      >
        <div class="sftp-detail-head">{{ detailTitle }}</div>
        <div v-if="detailLoading" class="sftp-detail-loading">{{ t("sftpDetails.loading") }}</div>
        <div v-if="detailRows.length" class="sftp-detail-rows">
          <div v-for="item in detailRows" :key="item.key" class="sftp-detail-row">
            <span class="sftp-detail-label">{{ item.label }}</span>
            <span class="sftp-detail-value">{{ item.value }}</span>
          </div>
        </div>
        <div v-if="detailUnavailable && !detailLoading" class="sftp-detail-unavailable">
          {{ t("sftpDetails.unavailable") }}
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.sftp-tree-pane {
  position: relative;
  display: flex;
  min-height: 0;
  height: 100%;
  flex: 1 1 auto;
  flex-direction: column;
  overflow: auto;
  overflow-x: hidden;
  padding: 2px 3px;
}

.sftp-tree-list {
  min-width: 0;
}

.sftp-tree-row {
  height: 28px;
  min-height: 28px;
  flex: 0 0 28px;
  outline: none;
}

.sftp-tree-row:focus-visible {
  outline: 1px solid var(--primary);
  outline-offset: -1px;
}

.sftp-tree-row.is-selected,
.sftp-tree-row.is-selected:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.sftp-tree-row.is-selected .sftp-tree-name {
  color: var(--primary);
  font-weight: 600;
}

.sftp-tree-caret-spacer {
  flex: 0 0 14px;
  width: 14px;
  height: 14px;
}

.sftp-tree-row > svg.sftp-tree-file-icon {
  width: 13px;
  height: 13px;
  flex: 0 0 13px;
  color: var(--muted-foreground);
}

.sftp-tree-row > svg.sftp-tree-file-icon.is-link {
  color: var(--primary);
}

.sftp-tree-name {
  min-width: 0;
  flex: 1 1 auto;
}

/* 行尾 Info：不参与行高（14px 图标放在 28px 行内），hover 行时提亮。 */
.sftp-tree-info {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 16px;
  width: 16px;
  height: 16px;
  margin-left: auto;
  padding: 0;
  border: 0;
  border-radius: 3px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  opacity: 0.5;
}

.sftp-tree-info svg {
  width: 12px;
  height: 12px;
}

.sftp-tree-row:hover .sftp-tree-info,
.sftp-tree-info:hover,
.sftp-tree-info:focus-visible {
  opacity: 1;
}

.sftp-tree-info:hover {
  background: color-mix(in srgb, var(--muted-foreground) 20%, transparent);
  color: var(--foreground);
}

/* 拖拽目标行必须压过 App 层 .drop-overlay（z=12）：否则整屏拖拽提示的
   半透明底色会盖住行高亮，看起来像“没有高亮”。 */
.sftp-tree-row.is-drop-target {
  position: relative;
  z-index: 13;
  background: color-mix(in srgb, var(--primary) 24%, var(--background));
  box-shadow: inset 0 0 0 2px var(--primary);
}

/* 详情卡 Teleport 到 body：fixed 定位 + 不拦截指针，关闭由悬停/焦点/滚动驱动。 */
.sftp-detail-card {
  position: fixed;
  z-index: 80;
  width: 276px;
  max-width: calc(100vw - 8px);
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--popover, var(--background));
  color: var(--popover-foreground, var(--foreground));
  box-shadow: 0 6px 18px rgb(0 0 0 / 0.28);
  font-size: 11px;
  line-height: 1.45;
  pointer-events: none;
}

.sftp-detail-head {
  margin-bottom: 4px;
  overflow: hidden;
  color: var(--foreground);
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sftp-detail-loading {
  margin-bottom: 4px;
  color: var(--muted-foreground);
}

.sftp-detail-row {
  display: grid;
  grid-template-columns: 76px minmax(0, 1fr);
  gap: 6px;
}

.sftp-detail-label {
  color: var(--muted-foreground);
  white-space: nowrap;
}

.sftp-detail-value {
  overflow-wrap: anywhere;
}

.sftp-detail-unavailable {
  margin-top: 4px;
  color: var(--muted-foreground);
  font-style: italic;
}

.rename-input {
  min-width: 0;
  flex: 1 1 auto;
  height: 22px;
}

.sftp-tree-spacer {
  flex: none;
  width: 100%;
}
</style>
