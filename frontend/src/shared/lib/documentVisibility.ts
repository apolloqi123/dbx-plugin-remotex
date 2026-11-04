// 文档可见性小工具：后台（窗口最小化/切到别的 App）时暂停轮询与采样，
// 可见后由调用方立即补一拍并恢复定时器。DBX 工作台长时间挂在后台是常态，
// 后台继续每 3–10s 发 SSH exec 只会给宿主与远端增加无谓负载。

export function documentHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}

/** 监听可见性变化；返回解绑函数（幂等）。 */
export function onDocumentVisibilityChange(handler: (hidden: boolean) => void): () => void {
  if (typeof document === "undefined" || typeof document.addEventListener !== "function") {
    return () => {};
  }
  const listener = () => handler(document.visibilityState === "hidden");
  document.addEventListener("visibilitychange", listener);
  return () => document.removeEventListener("visibilitychange", listener);
}
