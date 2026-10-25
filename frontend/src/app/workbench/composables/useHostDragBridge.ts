import type { Ref } from "vue";

/** SFTP 单栏树对外暴露的宿主拖拽落点接口（App.vue 回填的树实例 ref 同形）。 */
export interface HostDragTreePane {
  scrollPathToTop(path: string): Promise<boolean>;
  markDropTargetAt(point: { x: number; y: number }): string;
  resetHostDragCalibration(): void;
}

/**
 * 宿主文件拖拽桥：SDK 监听（`fileTransfer.onDragState`）与原始 postMessage
 * 两条通道共用同一份高亮/落点逻辑。高亮态本体（dragActive /
 * sftpDragDropTarget）与树实例仍由 App.vue 持有，经 options 注入。
 */
export function useHostDragBridge(options: {
  dragActive: Ref<boolean>;
  sftpDragDropTarget: Ref<string>;
  sftpTreePane: Ref<HostDragTreePane | null>;
}) {
  const { dragActive, sftpDragDropTarget, sftpTreePane } = options;

  /** 宿主拖拽状态入口：SDK 监听与原始 postMessage 双通道共用同一份逻辑。
   *  原始通道兜底旧版/缓存的注入 SDK（只把 active 传给监听器），只要宿主消息
   *  里带 position，仍能按坐标命中目录行；重复事件对高亮/计时器是幂等的。 */
  function applyHostDragState(active: boolean, position?: { x: number; y: number }) {
    const wasActive = dragActive.value;
    dragActive.value = active;
    if (!active) return;
    // 新一次拖拽的第一条坐标：重新校准 y 偏移；同一拖拽内的后续坐标沿用。
    if (!wasActive) sftpTreePane.value?.resetHostDragCalibration();
    sftpDragDropTarget.value = "";
    if (position) sftpTreePane.value?.markDropTargetAt(position);
  }

  function onHostDragMessage(event: MessageEvent) {
    const data = event.data as {
      source?: unknown;
      type?: unknown;
      active?: unknown;
      position?: { x?: unknown; y?: unknown };
    } | null;
    if (!data || data.source !== "dbx-host" || data.type !== "dragstate" || typeof data.active !== "boolean") return;
    const position = data.position && typeof data.position.x === "number" && typeof data.position.y === "number"
      ? { x: data.position.x, y: data.position.y }
      : undefined;
    applyHostDragState(data.active, position);
  }

  return { applyHostDragState, onHostDragMessage };
}
