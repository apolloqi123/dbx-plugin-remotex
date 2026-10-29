/**
 * SSH 终端域夹具：终端帧序号与输出投递、默认回放脚本（Welcome + OSC 633
 * shell-integration 周期）、ssh/exec / batchInput / directoryTracking 以及
 * ssh/terminal/in 的按键回显（含 ?aifix=1 的 AI 修复全链路）。
 */

import { UNHANDLED, type Unhandled, emitHostEvent, emitHostBinary, terminalFrame } from "./shared";
import { aiFixFixture } from "./fixture";

let sequence = 0;

export function resetSshSequence(): void {
  sequence = 0;
}

export function currentSshSequence(): number {
  return sequence;
}

export function emitTerminal(text: string) {
  sequence += 1;
  // mock 镜像当前宿主桥的二进制事件形状（零拷贝 data 字段），与真实宿主一致。
  const event = { channel: "ssh/terminal/out/visual-session", data: terminalFrame(sequence, text) };
  emitHostBinary(event);
}

// 模拟 VS Code 风格 shell-integration 周期（OSC 633），让 command-marker 条
// 在 open 与 reattach 两条启动路径下都有内容可渲染（P2-2）。
const OSC_633 = "\u001b]633;";
const BEL = "\u0007";
const shellIntegrationCycle = [
  `${OSC_633}P;Cwd=/home/demo${BEL}`,
  `${OSC_633}A${BEL}`,
  `${OSC_633}E;systemctl status nginx${BEL}`,
  "user@server:~$ systemctl status nginx\r\n",
  `${OSC_633}C${BEL}`,
  "● nginx.service - A high performance web server\r\n   Active: active (running)\r\n",
  `${OSC_633}D;0${BEL}`,
  // 默认高亮规则的演示输出：挂载即可见红（ERROR/No such file/command not
  // found）、黄（WARN）、琥珀（denied/timed out）、绿（SUCCESS/active running）、
  // 蓝（IPv4）五类着色。
  "user@server:~$ tail -n 3 /var/log/deploy.log\r\n",
  "2026-09-11 10:00:01 WARN disk usage 87%\r\n",
  "2026-09-11 10:00:04 ERROR Permission denied: /var/backup\r\n",
  "2026-09-11 10:00:09 deploy finished SUCCESS\r\n",
  "user@server:~$ cat /etc/nope; bash xyz\r\n",
  "cat: /etc/nope: No such file or directory\r\n",
  "bash: xyz: command not found\r\n",
  // 输出着色（tailspin 风格）演示：JSON / logfmt 行的时间戳、级别、键、数字、
  // IP 由终端输出着色器补上 ANSI 颜色（与日志页/编辑器同一套语义）。
  "user@server:~$ docker logs --tail 2 web\r\n",
  '{"level":"info","msg":"request done","status":200,"ok":true,"upstream":"10.0.0.8"}\r\n',
  "2026-10-29T16:40:10+08:00 INFO  upstream=10.0.0.8 latency=612ms path=/api/items\r\n",
  `${OSC_633}E;curl -m 3 http://10.0.0.12:8080/health${BEL}`,
  "user@server:~$ curl -m 3 http://10.0.0.12:8080/health\r\n",
  "curl: (28) Connection timed out\r\n",
  `${OSC_633}D;28${BEL}`,
  `${OSC_633}A${BEL}`,
  "user@server:~$ ",
].join("");
export const terminalTranscript = `Welcome to DBX SSH/SFTP visual fixture\r\n${shellIntegrationCycle}`;

/** open/attach 后 30ms 推送默认回放脚本（两条启动路径共用）。 */
export function emitTerminalTranscript(): void {
  setTimeout(() => emitTerminal(terminalTranscript), 30);
}

// ?aifix=1 的逐键行缓冲（模块级：跨多次 sendBinary 调用累计当前输入行）。
let sshTypedLine = "";

/** ssh/terminal/in 通道载荷：inputAck → 回显（?aifix=1 走 633 失败全链路）。 */
export function handleSshTerminalInput(bytes: Uint8Array): void {
  const inputSequence = Number(new DataView(bytes.buffer, bytes.byteOffset, 8).getBigUint64(0, false));
  emitHostEvent({ method: "ssh/terminal/inputAck", params: { sequence: inputSequence } });
  // 快速输入诊断（#33/#71）：回显键入内容，闭合 fixture 内的输入→显示环，
  // 让 WKWebView/Chromium 的按键投递差异可以在无宿主环境下复现。
  if (bytes.byteLength > 8) {
    const echoed = new TextDecoder().decode(bytes.subarray(8));
    // 真实 PTY 的 onlcr 会把 \r 转成 \r\n；夹具直发 \r 只会让光标回到行首、
    // 后续回显互相覆盖（建议命令/批量发送走查时表现为"命令消失"）。
    // canonical 行规程把控制键按 caret 记法回显成可打印文本（↑ → 字面
    // "^[[A"，光标不动，#150 堡垒机实测形态）：原样回显 ESC 字节会被 xterm
    // 解释成光标移动/控制效果，与真实 bastion 相悖，也让交互提示守卫的
    // 回归夹具失真（提示行被光标上移搅乱）。
    const caretEcho = echoed
      .replace(/\u001b\[[0-9;]*[A-Za-z~]/g, (sequence) => `^[${sequence.slice(1)}`)
      .replace(/\u001bO([A-Za-z0-9])/g, "^[O$1")
      .replace(/\u001b/g, "^[");
    const display = caretEcho.endsWith("\r") ? `${caretEcho.slice(0, -1)}\r\n` : caretEcho;
    // ?aifix=1（Warp AI 对齐批）：逐键回显进行缓冲，回车整行改道「E/C →
    // 失败输出（含敏感值）→ D exit 2 → 提示符」633 全链路——E 帧必须带完整
    // 命令行，否则 App 侧 commandMarker 残留上一条命令、D 码张冠李戴。
    if (aiFixFixture) {
      if (echoed.endsWith("\r")) {
        const command = sshTypedLine.trim();
        sshTypedLine = "";
        if (command) {
          setTimeout(() => emitTerminal(`${command}\r\n\u001b]633;E;${command}\u0007\u001b]633;C\u0007deploy failed\ntoken: ghp_${"a".repeat(36)}\nDB_PASSWORD=hunter2\r\n\u001b]633;D;2\u0007\u001b]133;D;2\u0007\u001b]633;A\u0007user@demo:~$ `), 12);
        }
        return;
      }
      sshTypedLine += echoed;
      setTimeout(() => emitTerminal(display), 12);
      return;
    }
    setTimeout(() => emitTerminal(display), 12);
  }
}

/** ssh/exec 家族与 terminal 控制面方法。 */
export async function handleTerminalInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/exec") {
    const input = params as Record<string, unknown>;
    const command = String(input.command || "");
    const sudo = input.sudo === true;
    // 历史面板的远端历史导入(连接后后台拉取):返回 fixture 历史存量,
    // 让 ↑ 面板演示出「shell history 合并 + 本会话采集」的混合内容;条数
    // 超过一屏,顺带验证列表内部滚动。
    if (command.includes("HISTFILE") && command.includes("tail -n")) {
      const fixture: string[] = [];
      for (let i = 1; i <= 28; i += 1) fixture.push(`deploy step ${i} -- systemctl restart app-${i}`);
      fixture.push(": 1700000000:0;vim /etc/nginx/nginx.conf", "kubectl --context prod get nodes");
      return { success: true, exitCode: 0, output: fixture.join("\n") };
    }
    await new Promise((resolve) => setTimeout(resolve, 1500));
    if (!sudo && !command.includes("sudo")) {
      // ANSI colour codes exercise the control-sequence sanitization in the dialog.
      return { success: true, output: `\u001b[32muid=1000(demo) gid=1000(demo)\u001b[0m\n$ ${command}`, exitCode: 0 };
    }
    setTimeout(() => emitTerminal("user@server:~$ sudo -S systemctl status nginx\r\n[sudo] password for user: \r\n● nginx.service - A high performance web server\r\n   Active: active (running)\r\n"), 30);
    return { success: true, output: "● nginx.service - A high performance web server\n   Loaded: loaded (/lib/systemd/system/nginx.service; enabled)\n   Active: active (running) since Mon 2026-08-24 09:12:31 UTC; 3 days ago", exitCode: 0 };
  }
  if (method === "ssh/terminal/batchInput") {
    const input = params as Record<string, unknown>;
    const sessionIds = Array.isArray(input.sessionIds) ? (input.sessionIds as string[]) : [];
    const command = String(input.command || "");
    const results = sessionIds.map((sessionId) =>
      sessionId === "visual-session"
        ? { sessionId, success: true }
        : { sessionId, success: false, error: "SSH session was not found" });
    if (results.some((row) => row.success)) {
      setTimeout(() => emitTerminal(`$ ${command}\r\nuser@server:~$ `), 30);
    }
    return { results, sent: results.filter((row) => row.success).length, failed: results.filter((row) => !row.success).length };
  }
  if (method === "ssh/terminal/directoryTracking") return { success: true };
  if (method === "ssh/exec/cancel") return { success: true };
  return UNHANDLED;
}
