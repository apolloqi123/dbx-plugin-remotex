// 本 webview 持有的容器 PTY 登记表：工作台关闭（宿主 workbench/close）发生在
// 组件卸载之前，且宿主随后直接拆除 webview；只靠组件卸载补发 close 会让远端
// `docker exec`（本机目标尤其）活到 2 小时 TTL。关闭钩子按 id 直接发
// `docker/terminal/close`，不复用连接级 FIFO（关页面不能被其它 RPC 排住）。
import { rpc } from "../../../shared/lib/rpc";

const activeTerminalIds = new Set<string>();

/** 登记一个已建立或正在建立的 PTY；返回注销函数（幂等）。 */
export function trackDockerTerminal(terminalId: string): () => void {
  activeTerminalIds.add(terminalId);
  return () => {
    activeTerminalIds.delete(terminalId);
  };
}

/**
 * 关闭本 webview 登记的所有容器 PTY。未知/已结束的 id 由后端幂等返回
 * `stopped:false`；桥已断开等失败不阻断工作台关闭。
 */
export async function stopActiveDockerTerminals(): Promise<void> {
  const terminalIds = [...activeTerminalIds];
  activeTerminalIds.clear();
  if (!terminalIds.length) return;
  // 后端 close 会等待 runner 释放 channel（上限 2s），给桥留一点余量。
  await Promise.allSettled(
    terminalIds.map((terminalId) =>
      rpc("docker/terminal/close", { terminalId }, { timeoutMs: 3_000 }).then(
        () => undefined,
        () => undefined,
      ),
    ),
  );
}

/** 测试复位；生产里 webview 关闭后不会再用。 */
export function resetDockerTerminalRegistry(): void {
  activeTerminalIds.clear();
}
