import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SftpFileTreeNode, SftpTreeRow } from "../lib/sftpFileTree";

const DROP_EXPAND_DELAY = 1_000;

export interface TreeDragPoint {
  x: number;
  y: number;
}

/** 文件拖放到目录行：只有宿主文件拖拽（dataTransfer.types 含 "Files" 或宿主
 *  dragActive）才高亮；折叠目录悬停 1s 后 toggle 由父级懒加载展开。drop 必须
 *  同步上抛原始 DragEvent，父级要在同一 tick 里读取 dataTransfer.items。 */
export function useTreeFileDrop(options: {
  rows: () => SftpTreeRow[];
  externalDragActive: () => boolean;
  emit: {
    toggle: (node: SftpFileTreeNode) => void;
    dropOn: (node: SftpFileTreeNode, dragEvent: DragEvent) => void;
    dropTargetChange: (path: string) => void;
  };
}) {
  const { rows, externalDragActive, emit } = options;

  const dragOverUri = ref("");
  const dragOverPath = ref("");
  let dropExpandTimer: ReturnType<typeof setTimeout> | undefined;
  let dropExpandUri = "";
  let dropExpandFiredUri = "";
  let pointerTracking = false;
  let lastPointerTrackAt = 0;
  let hoverPollTimer: ReturnType<typeof setInterval> | undefined;
  // 宿主已开始转发原生拖拽坐标：坐标命中优先于 DOM 事件与 :hover 兜底。
  let hostPositioned = false;
  // 冻结的 :hover 会一直返回拖入时的行。仅当轮询看到「与上次不同的行」时才认为
  // 指针真的移动了（live hover），用它兜底；同一行的重复上报一律忽略，避免
  // 把 pointer/dragover 已命中的新目标（A 展开后拖到 B）覆盖回旧行。
  let lastHoverUri = "";

  // 宿主桥整包接管 drop 时不会把目标行带进 onDrop 回调；父级用这份路径把
  // 「当前悬停的文件夹」作为宿主上传落点。
  watch(dragOverPath, (path) => emit.dropTargetChange(path));
  // 宿主拖拽结束时 DOM 未必补发 dragleave：清掉高亮/计时器，但保留
  // dragOverPath 供宿主紧接着的 onDrop 读取（部分宿主先发 false 再发 drop）。
  watch(
    () => externalDragActive(),
    (active) => {
      if (active) {
        // 宿主坐标可能先于 active prop 到达：已由坐标接管时不要重启兜底。
        if (!hostPositioned) startPointerTracking();
      } else {
        hostPositioned = false;
        stopPointerTracking();
        clearDragVisuals();
      }
    },
    { immediate: true },
  );

  /** 坐标命中目录行：宿主转发的是 iframe 内 CSS 像素，与 elementFromPoint 同域。 */
  function targetFromRow(rowEl: HTMLElement): { node: SftpFileTreeNode; index: number } | undefined {
    const index = Number(rowEl.dataset.treeIndex);
    const target = rows()[index];
    if (!target || target.node.kind !== "directory") return undefined;
    return { node: target.node, index };
  }

  function targetAtPoint(point: TreeDragPoint): { node: SftpFileTreeNode; index: number } | undefined {
    if (typeof document === "undefined") return undefined;
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) return undefined;
    // 原生拖拽期 WKWebView 的 elementFromPoint 可能返回 null/拖拽遮罩：
    // 先按命中元素，再按渲染行几何兜底（两者都用同一份 iframe 坐标）。
    if (typeof document.elementFromPoint === "function") {
      const element = document.elementFromPoint(point.x, point.y) as HTMLElement | null;
      const rowEl = element?.closest<HTMLElement>(".sftp-tree-row");
      if (rowEl) return targetFromRow(rowEl);
    }
    for (const rowEl of document.querySelectorAll<HTMLElement>(".sftp-tree-row")) {
      const rect = rowEl.getBoundingClientRect();
      if (point.x < rect.left || point.x > rect.right || point.y < rect.top || point.y > rect.bottom) continue;
      return targetFromRow(rowEl);
    }
    return undefined;
  }

  /** 宿主拖拽坐标入口（dragstate.position）：命中即高亮/计时，返回目标路径。 */
  function markDropTargetAt(point: TreeDragPoint): string {
    hostPositioned = true;
    stopPointerTracking();
    const target = targetAtPoint(point);
    if (!target) {
      clearDragTarget();
      return "";
    }
    markRowDragTarget(target.node);
    return target.node.path;
  }

  // 宿主桥只给 dragActive 布尔值（旧宿主），不保证把 dragover 派发到行元素。
  // 用全局指针位置做兜底：拖拽期间按 elementFromPoint 找到行，复用同一套标记。
  function trackPointer(event: MouseEvent) {
    if (!externalDragActive() || hostPositioned) return;
    if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) return;
    if (!event.clientX && !event.clientY) return;
    const now = performance.now();
    if (now - lastPointerTrackAt < 60) return;
    lastPointerTrackAt = now;
    const target = targetAtPoint({ x: event.clientX, y: event.clientY });
    if (!target) {
      clearDragTarget();
      return;
    }
    markRowDragTarget(target.node);
  }

  function startPointerTracking() {
    if (hostPositioned || pointerTracking || typeof document === "undefined") return;
    pointerTracking = true;
    lastHoverUri = "";
    document.addEventListener("mousemove", trackPointer, true);
    document.addEventListener("pointermove", trackPointer, true);
    document.addEventListener("dragover", trackPointer, true);
    // 另一道兜底：某些 WKWebView 宿主在原生拖拽期不派发鼠标/拖拽事件，但
    // CSS :hover 仍会随指针更新。轮询命中的行，不命中时保持现状（避免把
    // 已由 dragover/指针事件标记的目标误清）。
    hoverPollTimer = setInterval(() => {
      if (hostPositioned || !externalDragActive()) return;
      const rowEl = document.querySelector<HTMLElement>(".sftp-tree-row:hover");
      if (!rowEl) return;
      const index = Number(rowEl.dataset.treeIndex);
      const target = rows()[index];
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
    hostPositioned = false;
    clearDragTarget();
    lastHoverUri = "";
  }

  function isFileDrag(event: DragEvent): boolean {
    // 宿主桥的 dragActive 是权威的文件拖拽信号；DOM 事件的 types 在部分
    // WKWebView/沙箱拖拽里为空或不含 "Files"，不能单靠它。
    if (externalDragActive()) return true;
    const types = event.dataTransfer?.types;
    if (!types) return false;
    const list = Array.from(types);
    // 空 types = 宿主沙箱把文件拖拽的类型列表剥离了，但仍会派发 dragover；
    // 文本/元素拖拽至少带 text/plain 或自定义类型，不会被误判。
    return list.includes("Files") || list.length === 0;
  }

  /** 行级落点标记：dragover 事件与宿主拖拽的指针兜底共用同一套高亮/1s 展开。 */
  function markRowDragTarget(node: SftpFileTreeNode) {
    dragOverUri.value = node.entry.uri;
    dragOverPath.value = node.path;
    if (node.expanded || node.loading) {
      clearDropExpandTimer();
      return;
    }
    // 同一行的重复 dragover 不重启 1s 计时器；触发过一次后直到 dragleave/drop
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
      emit.toggle(node);
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
    if (!externalDragActive()) return;
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
    emit.dropOn(node, event);
  }

  onMounted(() => {
    window.addEventListener("dragend", endDragSession);
  });

  onBeforeUnmount(() => {
    window.removeEventListener("dragend", endDragSession);
    stopPointerTracking();
    clearDragTarget();
  });

  return {
    dragOverUri,
    onTreeDragOver,
    onRowDragOver,
    onRowDragLeave,
    onRowDrop,
    markDropTargetAt,
    clearDragTarget,
  };
}
