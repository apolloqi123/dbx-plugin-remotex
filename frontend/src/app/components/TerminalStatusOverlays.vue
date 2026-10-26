<script setup lang="ts">
import { Disc, Loader2, Square, Terminal as TerminalIcon, TriangleAlert } from "@lucide/vue";
import type { ReconnectCountdown } from "../../features/terminal/lib/behavior/terminalReconnect";
import type { RdpSessionStateView } from "../../features/remote-desktop/lib/rdpFrame";

/** 终端状态层（T20）：重连横幅/录制控制条/录制倒计时、本地终端启动与退出、
 *  Telnet/串口/VNC/RDP 的关闭层。组件只渲染并上抛动作；状态与 sidecar 接线
 *  仍由 App 持有，经显式 props/emits 下发（无 provide/inject/全局态）。 */
const props = defineProps<{
  reconnectPending: boolean;
  reconnectCountdown: ReconnectCountdown | null;
  recordingActive: boolean;
  recordingElapsedSec: number;
  recordCountdown: number | null;
  isLocalMode: boolean;
  localShellRestored: boolean;
  localState: "starting" | "running" | "exited";
  localExitCode: number | null;
  isTelnetMode: boolean;
  telnetState: "idle" | "connecting" | "running" | "closed";
  telnetError: string;
  isSerialMode: boolean;
  serialState: "idle" | "running" | "closed";
  serialError: string;
  isVncMode: boolean;
  vncState: "idle" | "connecting" | "running" | "closed";
  vncError: string;
  isRdpMode: boolean;
  rdpState: RdpSessionStateView;
  rdpClosedTitle: string;
  formatDuration: (seconds: number) => string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "reconnect-now"): void;
  (e: "toggle-recording"): void;
  (e: "cancel-record-countdown"): void;
  (e: "restart-local"): void;
  (e: "close-local"): void;
  (e: "dismiss-restored-shell"): void;
  (e: "close-telnet"): void;
  (e: "close-serial"): void;
  (e: "close-vnc"): void;
  (e: "rdp-reconnect"): void;
  (e: "rdp-edit-credentials"): void;
  (e: "rdp-close"): void;
}>();

const { t } = props;

/** 本地退出卡的关闭出口：恢复外壳未起 shell 时走 dismiss，普通本地终端走
 *  close。模板内三元 emit 会触发 vue-tsc 重载解析失败（T20 上次尝试的坑），
 *  因此拆成脚本内的两个显式 emit 分支。 */
function onLocalExitClose() {
  if (props.localShellRestored) emit("dismiss-restored-shell");
  else emit("close-local");
}
</script>

<template>
  <div v-if="reconnectPending" class="reconnect-banner" role="status">
    <Loader2 class="spinning" />
    <span class="reconnect-text">{{ t("reconnectBanner.label") }}</span>
    <span v-if="reconnectCountdown" class="reconnect-countdown mono">{{ t("sessionStatus.reconnectCountdown", { seconds: reconnectCountdown.seconds, attempt: reconnectCountdown.attempt }) }}</span>
    <span v-if="reconnectCountdown" class="reconnect-attempt">{{ t("reconnectBanner.attempt", { attempt: reconnectCountdown.attempt }) }}</span>
    <progress v-if="reconnectCountdown" :value="reconnectCountdown.percent" max="100" />
    <button class="reconnect-now" @click="emit('reconnect-now')">{{ t("reconnectNow") }}</button>
  </div>
  <!-- 录制中悬浮控制条：底部居中，红色呼吸点 + 时长 + 停止 -->
  <div v-if="recordingActive" class="recording-float" role="status">
    <Disc class="recording-float-dot" />
    <span class="recording-elapsed mono">{{ formatDuration(recordingElapsedSec) }}</span>
    <button class="recording-stop" @click="emit('toggle-recording')"><Square />{{ t("recordingStop") }}</button>
  </div>
  <!-- 录制开始倒计时遮罩：大数字 3→2→1（:key 重触发放缩动画），Esc/点击取消 -->
  <div v-if="recordCountdown !== null" class="record-countdown-overlay" role="status" @click="emit('cancel-record-countdown')">
    <span class="record-countdown-number" :key="recordCountdown">{{ recordCountdown }}</span>
    <span class="record-countdown-hint">{{ t("recordingCountdownHint") }}</span>
  </div>
  <!-- 本地终端启动中覆盖层：spawn 正常在数百毫秒内返回；卡在 starting
       （宿主桥丢响应、慢盘冷启动）必须有可见反馈，否则就是零信息黑屏。
       超时/失败由 startLocalTerminal 的 catch 接管转 exited + showError。 -->
  <div v-if="(isLocalMode || localShellRestored) && localState === 'starting'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <Loader2 class="local-exit-icon spinning" />
      <strong>{{ t("localTerminal.starting") }}</strong>
    </div>
  </div>
  <!-- 本地终端退出态覆盖层（含 A4 恢复外壳：restored tab 未起 shell 时
       也由它承担外壳态，spec §8.4）：显示退出码（sidecar 未上报时不显
       示），给重开与关闭两个出口（VS Code 式终端退出体验）。 -->
  <div v-if="(isLocalMode || localShellRestored) && localState === 'exited'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <TerminalIcon class="local-exit-icon" />
      <strong>{{ t("localTerminal.exited") }}</strong>
      <span v-if="localExitCode !== null" class="mono local-exit-code">{{ t("localTerminal.exitCode", { code: localExitCode }) }}</span>
      <div class="local-exit-actions">
        <button class="primary-button" @click="emit('restart-local')">{{ t("localTerminal.restart") }}</button>
        <button @click="onLocalExitClose">{{ t("localTerminal.close") }}</button>
      </div>
    </div>
  </div>
  <!-- Telnet 退出覆盖层（连接失败/对端断开）：给出关闭出口，展示
       sidecar 带回的原因文本（明文协议告警在连接弹窗里）。 -->
  <div v-if="isTelnetMode && telnetState === 'closed'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <TriangleAlert class="local-exit-icon" />
      <strong>{{ t("telnet.closed") }}</strong>
      <span v-if="telnetError" class="mono local-exit-code">{{ telnetError }}</span>
      <div class="local-exit-actions">
        <button @click="emit('close-telnet')">{{ t("telnet.close") }}</button>
      </div>
    </div>
  </div>
  <!-- 串口退出覆盖层（读线程 IO 失败/设备拔线）：给出关闭出口，展示原因。 -->
  <div v-if="isSerialMode && serialState === 'closed'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <TriangleAlert class="local-exit-icon" />
      <strong>{{ t("serial.closed") }}</strong>
      <span v-if="serialError" class="mono local-exit-code">{{ serialError }}</span>
      <div class="local-exit-actions">
        <button @click="emit('close-serial')">{{ t("serial.close") }}</button>
      </div>
    </div>
  </div>
  <!-- VNC 退出覆盖层（认证失败/服务端强制 Tight/对端断开）：给出关闭
       出口，展示 sidecar 带回的原因文本；明文/经典认证告警在连接弹窗。 -->
  <div v-if="isVncMode && vncState === 'closed'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <TriangleAlert class="local-exit-icon" />
      <strong>{{ t("vnc.closed") }}</strong>
      <span v-if="vncError" class="mono local-exit-code">{{ vncError }}</span>
      <div class="local-exit-actions">
        <button @click="emit('close-vnc')">{{ t("vnc.close") }}</button>
      </div>
    </div>
  </div>
  <!-- RDP 重连状态条（退避梯子进行中）：非阻塞展示进度，梯子跑完由
       closed 覆盖层接管（协议：证书/认证/协商失败永不自动重试）。 -->
  <div v-if="isRdpMode && rdpState.state === 'reconnecting'" class="zmodem-status" role="status">
    <Loader2 class="spinning" />
    <span>{{ rdpState.maxAttempts > 0 ? t("rdp.reconnectingAttempt", { attempt: rdpState.attempt, max: rdpState.maxAttempts }) : t("rdp.reconnecting") }}</span>
  </div>
  <!-- RDP 退出覆盖层（graceful close / 终态错误）：errorKind 友好文案为
       主行、sidecar 原因文本为细节行；给出 Reconnect（rdp/reconnect）与
       关闭双出口（协议：服务端主动断开不自动重连，由用户决定）。 -->
  <div v-if="isRdpMode && rdpState.state === 'closed'" class="terminal-overlay">
    <div class="local-exit-card" role="status">
      <TriangleAlert class="local-exit-icon" />
      <strong>{{ rdpClosedTitle }}</strong>
      <span v-if="rdpState.error" class="mono local-exit-code">{{ rdpState.error }}</span>
      <div class="local-exit-actions">
        <button class="primary-button" @click="emit('rdp-reconnect')">{{ t("rdp.reconnect") }}</button>
        <!-- 认证失败是永久错误：重连只会用同一套凭据再撞一次（协议：
             认证失败永不自动重试），把用户带回可改凭据的连接对话框。 -->
        <button v-if="rdpState.errorKind === 'authentication'" @click="emit('rdp-edit-credentials')">{{ t("rdp.editCredentials") }}</button>
        <button @click="emit('rdp-close')">{{ t("rdp.close") }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 录制中：悬浮控制条（红色呼吸点 + 时长 mono 等宽不跳动 + 停止按钮）。
   锚定视口而非终端面板：面板底边可能探出可视区（批量栏等会推高布局），
   绝对定位会被裁掉半截；fixed 与 toast 通知同款，已验证可靠。 */
.recording-float { position: fixed; z-index: 40; bottom: 64px; left: 50%; display: flex; align-items: center; gap: 8px; transform: translateX(-50%); border: 1px solid color-mix(in srgb, var(--destructive) 45%, var(--border)); border-radius: 999px; padding: 5px 7px 5px 12px; background: color-mix(in srgb, var(--background) 92%, transparent); box-shadow: 0 4px 14px color-mix(in srgb, #000 18%, transparent); font-size: 11px; }
.recording-float-dot { width: 12px; height: 12px; flex: 0 0 12px; color: var(--destructive); animation: record-pulse 1.6s ease-in-out infinite; }
.recording-elapsed { color: var(--foreground); font-size: 11px; font-variant-numeric: tabular-nums; }
.recording-stop { display: inline-flex; align-items: center; gap: 4px; height: 24px; border: 1px solid var(--border); border-radius: 999px; padding: 0 10px; background: var(--background); color: var(--destructive); font-size: 11px; cursor: pointer; }
.recording-stop:hover { background: var(--accent); }
.recording-stop svg { width: 11px; height: 11px; }
@keyframes record-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

/* 录制开始倒计时遮罩：终端区中央大数字逐级放缩淡入，点击/Esc 取消。 */
.record-countdown-overlay {
  position: absolute;
  z-index: 5;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  background: color-mix(in srgb, var(--background) 62%, transparent);
  cursor: pointer;
  user-select: none;
}
.record-countdown-number {
  color: var(--foreground);
  font-size: 72px;
  font-weight: 700;
  line-height: 1;
  font-variant-numeric: tabular-nums;
  text-shadow: 0 4px 24px rgb(0 0 0 / 40%);
  animation: record-countdown-pop 0.9s cubic-bezier(0.2, 0.9, 0.3, 1) both;
}
.record-countdown-hint { color: var(--muted-foreground); font-size: 11px; }
@keyframes record-countdown-pop {
  0% { opacity: 0; transform: scale(1.5); }
  25% { opacity: 1; transform: scale(1); }
  85% { opacity: 1; transform: scale(0.96); }
  100% { opacity: 0.25; transform: scale(0.92); }
}
</style>
