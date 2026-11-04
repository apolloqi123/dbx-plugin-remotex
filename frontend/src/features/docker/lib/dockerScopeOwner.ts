// 容器日志页 / 终端页的实例 scope id：后端据此保证同一页签只保留一条远端
// 流或一个 PTY，并在页面关闭后拒绝迟到的请求。字符集与后端 `scope_owner`
// 校验一致（字母数字、`-`、`_`）。
let sequence = 0;

export function createDockerScopeOwner(prefix: "log" | "term"): string {
  sequence += 1;
  return `${prefix}-${Date.now().toString(36)}-${sequence}-${Math.random().toString(36).slice(2, 8)}`;
}
