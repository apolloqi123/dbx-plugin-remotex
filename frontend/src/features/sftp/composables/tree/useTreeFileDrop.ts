import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SftpFileTreeNode, SftpTreeRow } from "../../lib/tree/sftpFileTree";
import {
  cursorInWindow,
  dragPointSpread,
  estimateHostScale,
  mapHostDragPoint,
  medianDragPoint,
  offsetFromSample,
  sanitizeDragScale,
  type DragMappingPoint,
  type DragPair,
} from "../../lib/transfer/hostDragMapping";
import type { HostPointerSample } from "../../../../shared/lib/hostPointer";

const DROP_EXPAND_DELAY = 1_000;
/** 同一拖拽内两次探针的最小间隔：光标停住后 3-4 个样本即可收敛。 */
const POINTER_SAMPLE_INTERVAL = 90;
const POINTER_SAMPLE_LIMIT = 20;
/** 偏移样本收敛阈值（px）；超过截止时间就退回中位数，避免拖拽结束后一直采样。 */
const OFFSET_STABLE_SPREAD = 6;
const OFFSET_SETTLE_TIMEOUT = 1_500;

export interface TreeDragPoint {
  x: number;
  y: number;
}

/** 文件拖放到目录行：只有宿主文件拖拽（dataTransfer.types 含 "Files" 或宿主
 *  dragActive）才高亮；折叠目录悬停 1s 后 toggle 由父级懒加载展开。drop 必须
 *  同步上抛原始 DragEvent，父级要在同一 tick 里读取 dataTransfer.items。
 *
 *  宿主转发的原生拖拽坐标在 macOS DBX 上被多除了一次 devicePixelRatio
 *  （见 hostDragMapping.ts）：用 sidecar 的光标探针解出 iframe 偏移后，
 *  后续宿主点即可精确换回 iframe 坐标；探针不可用时退回原始点 + :hover
 *  兜底，行为不劣于旧宿主。 */
export function useTreeFileDrop(options: {
  rows: () => SftpTreeRow[];
  externalDragActive: () => boolean;
  /** 真实光标 + 宿主窗口几何（macOS sidecar 探针）；缺省/失败时不校准。 */
  samplePointer?: () => Promise<HostPointerSample | null>;
  /** 显示缩放比例；缺省取 iframe 的 devicePixelRatio（与宿主同屏一致）。 */
  deviceScale?: () => number;
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
  // 宿主已开始转发原生拖拽坐标：坐标是活的，:hover 在原生拖拽期会冻结在
  // 拖入行，不能再让它覆盖宿主目标。校准与否都不影响这个优先级。
  let hostPositioned = false;
  // 已校准的 iframe 偏移（宿主 CSS 坐标）：跨拖拽缓存，同一布局直接可用，
  // 新拖拽再用探针校正。null = 尚未校准。
  let hostOffset: DragMappingPoint | null = null;
  // 宿主把逻辑点缩小了多少（DBX 0.6.x = devicePixelRatio，修好后 = 1）。
  // null 表示还没测出来，先按默认缩放换算。
  let hostScale: number | null = null;
  let offsetSamples: DragMappingPoint[] = [];
  let dragPairs: DragPair[] = [];
  let calibrationStartedAt = 0;
  let lastSampleAt = 0;
  let samplingDone = false;
  let sampleSequence = 0;
  let sampleInFlight = 0;
  let sampleTimer: ReturnType<typeof setTimeout> | undefined;
  let dragSession = 0;
  let lastHostPoint: DragMappingPoint | null = null;
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
        // 宿主坐标可能先于 active prop 到达：坐标已接管就不要重启兜底，
        // 但迟到的那一帧仍要补一次校准采样。
        if (!hostPositioned) startPointerTracking();
        if (lastHostPoint) void requestPointerSample();
      } else {
        hostPositioned = false;
        stopPointerTracking();
        resetCalibrationSession();
        clearDragVisuals();
      }
    },
    { immediate: true },
  );

  function defaultDragScale(): number {
    const raw = options.deviceScale?.() ?? (typeof window === "undefined" ? 1 : window.devicePixelRatio);
    return sanitizeDragScale(raw);
  }

  /** 当前换算用的缩放：优先用样本测出的宿主行为，测不出时按显示缩放。 */
  function dragScale(): number {
    return hostScale ?? defaultDragScale();
  }

  /** 坐标命中目录行：换算后的点是 iframe CSS 像素，与 elementFromPoint 同域。 */
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

  /** 宿主坐标 → iframe 坐标；未校准时退回原始点（旧宿主/探针不可用）。 */
  function resolveHostPoint(point: DragMappingPoint): DragMappingPoint {
    if (!hostOffset) return point;
    return mapHostDragPoint({ host: point, offset: hostOffset, scale: dragScale() });
  }

  function markPointAt(point: DragMappingPoint): string {
    const target = targetAtPoint(point);
    if (!target) {
      // 已校准的坐标可信：落在文件行/空白区就是没有目标，必须撤掉上一行的
      // 高亮与 1s 计时器；未校准时点本来就可能偏，保留上一次命中更稳。
      if (hostOffset) clearDragTarget();
      return dragOverPath.value;
    }
    markRowDragTarget(target.node);
    return target.node.path;
  }

  async function requestPointerSample() {
    const sampler = options.samplePointer;
    if (!sampler || sampleInFlight || samplingDone || !externalDragActive()) return;
    const now = performance.now();
    if (lastSampleAt && now - lastSampleAt < POINTER_SAMPLE_INTERVAL) return;
    lastSampleAt = now;
    if (!calibrationStartedAt) calibrationStartedAt = now;
    const token = ++sampleSequence;
    const session = dragSession;
    sampleInFlight = token;
    let sample: HostPointerSample | null = null;
    try {
      sample = await sampler();
    } catch {
      sample = null;
    } finally {
      if (sampleInFlight === token) sampleInFlight = 0;
    }
    if (!sample || token !== sampleSequence || session !== dragSession) return;
    // drop 已结束（宿主先发 false 再发 filedrop）时，迟到的样本不能再改落点。
    if (!externalDragActive()) return;
    applyPointerSample(sample);
  }

  function applyPointerSample(sample: HostPointerSample) {
    const host = lastHostPoint;
    if (!sample.cursor || !sample.window || !host) return;
    const cursorWindow = cursorInWindow({ cursor: sample.cursor, window: sample.window });
    if (!cursorWindow) return;
    dragPairs.push({ cursorWindow, host });
    if (dragPairs.length > 10) dragPairs.shift();
    // 宿主若不再多除一次缩放（未来 DBX 修复），位移比会收敛到 1：及时改判，
    // 并把按旧 scale 学到的偏移作废，避免继续用错误换算。
    const estimatedScale = estimateHostScale(dragPairs, defaultDragScale(), dragScale());
    if (estimatedScale !== dragScale()) {
      hostScale = estimatedScale;
      offsetSamples = [];
      hostOffset = null;
    }
    const scale = dragScale();
    const offset = offsetFromSample({ cursorWindow, host, scale, window: sample.window });
    if (!offset) return;
    offsetSamples.push(offset);
    if (offsetSamples.length > 8) offsetSamples.shift();
    const median = medianDragPoint(offsetSamples);
    if (!median) return;
    const spread = dragPointSpread(offsetSamples.slice(-4));
    const confident = spread <= 12;
    // 有缓存但新样本还没收敛时，先信缓存；首次校准（无缓存）立刻接管高亮。
    if (!hostOffset || confident) hostOffset = median;
    stopPointerTracking();
    // 立即按同一份样本落点，不必等下一次 dragstate。
    const activeOffset = hostOffset;
    markPointAt({ x: cursorWindow.x - activeOffset.x, y: cursorWindow.y - activeOffset.y });
    if (confident && spread <= OFFSET_STABLE_SPREAD) samplingDone = true;
    if (offsetSamples.length >= POINTER_SAMPLE_LIMIT) samplingDone = true;
    if (!samplingDone && performance.now() - calibrationStartedAt > OFFSET_SETTLE_TIMEOUT) {
      samplingDone = true;
    }
    // 指针停住时宿主不再发 dragstate；靠定时器把剩余样本跑完，否则首个
    // 样本若有事件/RPC 时间差就会一直用错误的偏移。
    if (!samplingDone) schedulePointerSample();
  }

  function schedulePointerSample() {
    if (sampleTimer !== undefined || samplingDone || !hostPositioned) return;
    sampleTimer = setTimeout(() => {
      sampleTimer = undefined;
      if (samplingDone || !externalDragActive()) return;
      void requestPointerSample();
    }, POINTER_SAMPLE_INTERVAL);
  }

  function stopSampling() {
    samplingDone = true;
    if (sampleTimer !== undefined) {
      clearTimeout(sampleTimer);
      sampleTimer = undefined;
    }
  }

  function resetCalibrationSession() {
    hostPositioned = false;
    dragSession += 1;
    sampleSequence += 1;
    sampleInFlight = 0;
    stopSampling();
    samplingDone = false;
    offsetSamples = [];
    dragPairs = [];
    calibrationStartedAt = 0;
    lastSampleAt = 0;
    lastHostPoint = null;
    // hostOffset 保留：同一布局的下一次拖拽首帧就能命中，再靠探针收敛。
  }

  /** 宿主拖拽坐标入口（dragstate.position）：命中即高亮/计时，返回目标路径。 */
  function markDropTargetAt(point: TreeDragPoint): string {
    hostPositioned = true;
    lastHostPoint = point;
    // 宿主坐标是活的：停掉 :hover/指针兜底，避免冻结的悬停行把目标拉回去。
    stopPointerTracking();
    // 已校准也抽样一次：窗口/布局变了时下一帧就能自愈（拖拽中只会跑完
    // 一轮收敛样本，成本是一次本地 RPC）。
    void requestPointerSample();
    return markPointAt(resolveHostPoint(point));
  }

  // 宿主桥只给 dragActive 布尔值（旧宿主），不保证把 dragover 派发到行元素。
  // 用全局指针位置做兜底：拖拽期间按 elementFromPoint 找到行，复用同一套标记。
  function trackPointer(event: MouseEvent) {
    if (!externalDragActive() || hostPositioned || hostOffset) return;
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
    if (hostPositioned || hostOffset || pointerTracking || typeof document === "undefined") return;
    pointerTracking = true;
    lastHoverUri = "";
    document.addEventListener("mousemove", trackPointer, true);
    document.addEventListener("pointermove", trackPointer, true);
    document.addEventListener("dragover", trackPointer, true);
    // 另一道兜底：某些 WKWebView 宿主在原生拖拽期不派发鼠标/拖拽事件，但
    // CSS :hover 仍会随指针更新。轮询命中的行，不命中时保持现状（避免把
    // 已由 dragover/指针事件标记的目标误清）。
    hoverPollTimer = setInterval(() => {
      if (hostPositioned || hostOffset || !externalDragActive()) return;
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
    stopPointerTracking();
    resetCalibrationSession();
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
    dragSession += 1;
    sampleSequence += 1;
    stopSampling();
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
    /** 新拖拽开始：清掉上一轮的样本与在途探针，保留已学到的偏移。 */
    resetHostDragCalibration: resetCalibrationSession,
    clearDragTarget,
  };
}
