<script setup lang="ts">
import { Dialog, DialogContent, DialogTitle } from "../../shared/ui/dialog/index";
import { Loader2, TriangleAlert, X } from "@lucide/vue";
import { purposeKeyLabel, sanitizeTriagePayload, severityClass, type TriageResult } from "../../features/security/lib/alertTriage";
import { rdpCertStatusKey } from "../../features/remote-desktop/lib/rdpFrame";
import type { PasteConfirmation } from "../../features/terminal/lib/dangerousCommands";

/** 会话/安全弹窗组：命令会话、主机密钥、RDP 证书、Agent 审批、告警排查、
 *  粘贴确认、通用确认、本地终端确认。状态与决议 Promise 仍由 App 的
 *  composables 持有；弹窗壳走全局 modal 样式（reka portal）。
 *  拖入上传落点弹窗暂留 App：其输入框 ref 被 useUploadChain 直接 focus。 */
type HostKeyPromptView = { host: string; port: number; keyType: string; fingerprint: string };
type RdpCertPromptView = { host: string; port: number; fingerprint: string; knownHostStatus: string };
type AgentPromptHeadView = { source?: string; tool: string; risk: string };
type PendingConfirmView = { message: string; danger: boolean };

const props = defineProps<{
  spawnSessionDialogOpen: boolean;
  spawnSessionCommand: string;
  hostKeyPrompt: HostKeyPromptView | null | undefined;
  rememberHostKey: boolean;
  rdpCertPrompt: RdpCertPromptView | null | undefined;
  rdpCertRemember: boolean;
  rdpCertRemaining: number;
  agentPromptHead: AgentPromptHeadView | null | undefined;
  agentPromptCommand: string;
  agentPromptRemember: boolean;
  agentPromptRemaining: number;
  alertTriageOpen: boolean;
  alertTriagePayload: string;
  alertTriageBusy: boolean;
  alertTriageError: string;
  alertTriageResult: TriageResult | undefined;
  hasSession: boolean;
  pasteConfirm: PasteConfirmation | undefined;
  pendingConfirmDialog: PendingConfirmView | null | undefined;
  localOpenConfirmOpen: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:spawnSessionDialogOpen", value: boolean): void;
  (e: "update:spawnSessionCommand", value: string): void;
  (e: "update:rememberHostKey", value: boolean): void;
  (e: "update:rdpCertRemember", value: boolean): void;
  (e: "update:agentPromptCommand", value: string): void;
  (e: "update:agentPromptRemember", value: boolean): void;
  (e: "update:alertTriageOpen", value: boolean): void;
  (e: "update:alertTriagePayload", value: string): void;
  (e: "update:localOpenConfirmOpen", value: boolean): void;
  (e: "confirm-command-session-tab"): void;
  (e: "resolve-host-key", accept: boolean): void;
  (e: "resolve-rdp-certificate", accept: boolean): void;
  (e: "resolve-agent-prompt", decision: "approve" | "deny"): void;
  (e: "run-alert-triage"): void;
  (e: "send-suggestion", command: string): void;
  (e: "copy-suggestions"): void;
  (e: "resolve-paste-confirm", accepted: boolean): void;
  (e: "resolve-pending-confirm", accepted: boolean): void;
  (e: "confirm-local-terminal"): void;
}>();

const t = props.t;
</script>

<template>
    <!-- 命令会话（WT-4，WezTerm spawn 对标）：输入命令后在已认证 transport 的新 channel 上执行 -->
    <Dialog :open="spawnSessionDialogOpen" @update:open="(open) => { if (!open) emit('update:spawnSessionDialogOpen', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("spawnSessionDialog.title") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:spawnSessionDialogOpen', false)"><X /></button></header>
        <p class="muted">{{ t("spawnSessionDialog.hint") }}</p>
        <input
          :value="spawnSessionCommand" @input="emit('update:spawnSessionCommand', ($event.target as HTMLInputElement).value)"
          autofocus
          spellcheck="false"
          autocomplete="off"
          :placeholder="t('spawnSessionDialog.placeholder')"
          @keydown.enter="emit('confirm-command-session-tab')"
        />
        <footer>
          <button @click="emit('update:spawnSessionDialogOpen', false)">{{ t("cancel") }}</button>
          <button class="primary-button" :disabled="!spawnSessionCommand.trim()" @click="emit('confirm-command-session-tab')">{{ t("confirm") }}</button>
        </footer>
      </DialogContent>
    </Dialog>
    <!-- 安全弹窗：不允许 Esc / 点击遮罩关闭，必须显式信任或拒绝（不在 Esc 链中） -->
    <Dialog :open="!!hostKeyPrompt">
      <DialogContent class="modal host-key-modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="hostKeyPrompt">
        <header><DialogTitle>{{ t("hostKeyDialog.title") }}</DialogTitle></header>
        <p>{{ t("hostKeyDialog.desc") }}</p>
        <dl><dt>{{ t("hostKeyDialog.server") }}</dt><dd>{{ hostKeyPrompt.host }}:{{ hostKeyPrompt.port }}</dd><dt>{{ t("hostKeyDialog.keyType") }}</dt><dd>{{ hostKeyPrompt.keyType }}</dd><dt>{{ t("hostKeyDialog.fingerprint") }}</dt><dd class="fingerprint">{{ hostKeyPrompt.fingerprint }}</dd></dl>
        <label class="remember"><input :checked="rememberHostKey" @change="emit('update:rememberHostKey', ($event.target as HTMLInputElement).checked)" type="checkbox" /> {{ t("hostKeyDialog.remember") }}</label>
        <footer><button @click="emit('resolve-host-key', false)">{{ t("hostKeyDialog.reject") }}</button><button class="primary-button" @click="emit('resolve-host-key', true)">{{ t("hostKeyDialog.trust") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- RDP 证书确认（rdp-certificate challenge）：安全弹窗，不允许 Esc / 点击
         遮罩关闭；SHA256 指纹 + knownHostStatus 徽标 + 120s 倒计时 + remember，
         超时 fail-closed（应答与 sidecar 超时同样按拒绝处理）。 -->
    <Dialog :open="!!rdpCertPrompt">
      <DialogContent class="modal host-key-modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="rdpCertPrompt">
        <header><DialogTitle>{{ t("rdp.cert.title") }}</DialogTitle></header>
        <p>{{ t("rdp.cert.desc") }}</p>
        <dl>
          <dt>{{ t("rdp.cert.server") }}</dt><dd>{{ rdpCertPrompt.host }}:{{ rdpCertPrompt.port }}</dd>
          <dt>{{ t("rdp.cert.fingerprint") }}</dt><dd class="fingerprint">{{ rdpCertPrompt.fingerprint }}</dd>
        </dl>
        <p class="muted rdp-cert-status">{{ t(`rdp.cert.status.${rdpCertStatusKey(rdpCertPrompt.knownHostStatus)}`) }}</p>
        <label class="remember"><input :checked="rdpCertRemember" @change="emit('update:rdpCertRemember', ($event.target as HTMLInputElement).checked)" type="checkbox" /> {{ t("rdp.cert.remember") }}</label>
        <p class="muted rdp-cert-expires">{{ t("rdp.cert.expires", { seconds: rdpCertRemaining }) }}</p>
        <footer><button @click="emit('resolve-rdp-certificate', false)">{{ t("rdp.cert.reject") }}</button><button class="primary-button" @click="emit('resolve-rdp-certificate', true)">{{ t("rdp.cert.accept") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- 安全弹窗：不允许 Esc / 点击遮罩关闭，必须显式批准或拒绝（不在 Esc 链中） -->
    <Dialog :open="!!agentPromptHead">
      <DialogContent class="modal" @escape-key-down.prevent @pointer-down-outside.prevent>
        <template v-if="agentPromptHead">
        <header><DialogTitle>{{ agentPromptHead.source === "mcp" ? t("agentPrompt.mcpSource", { tool: agentPromptHead.tool }) : t("agentPromptTitle") }}</DialogTitle></header>
        <div class="agent-prompt-meta">
          <span>{{ t("agentPromptSource") }} <code class="mono">{{ agentPromptHead.tool }}</code></span>
          <span class="agent-risk-badge" :class="agentPromptHead.risk === 'elevated' ? 'elevated' : 'low'">{{ agentPromptHead.risk === "elevated" ? t("agentPromptRiskElevated") : t("agentPromptRiskLow") }}</span>
        </div>
        <label class="agent-prompt-command">
          <span>{{ t("agentPromptCommandLabel") }}</span>
          <textarea :value="agentPromptCommand" @input="emit('update:agentPromptCommand', ($event.target as HTMLTextAreaElement).value)" class="mono" rows="3" spellcheck="false" />
        </label>
        <!-- 记住不限风险档：strict 模式下低危命令同样每次弹审、同样需要免审
             记忆（IMPL_PLAN 预期 strict/auto 下 approve+remember 二次零弹窗）；
             破坏性命令由后端 D2 兜底忽略 remember。 -->
        <label class="agent-prompt-remember">
          <input :checked="agentPromptRemember" @change="emit('update:agentPromptRemember', ($event.target as HTMLInputElement).checked)" type="checkbox" />
          <span>{{ t("approval.remember") }}</span>
        </label>
        <p class="muted agent-prompt-countdown">{{ t("agentPromptTimeoutHint", { seconds: Math.ceil(agentPromptRemaining) }) }}</p>
        <footer><button @click="emit('resolve-agent-prompt', 'deny')">{{ t("agentPromptDeny") }}</button><button class="primary-button" @click="emit('resolve-agent-prompt', 'approve')">{{ t("agentPromptApprove") }}</button></footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- 告警排查：异构告警 → 结构化 + 分类 + 只读诊断命令清单 -->
    <Dialog :open="alertTriageOpen" @update:open="(open) => { if (!open) emit('update:alertTriageOpen', false); }">
      <DialogContent class="modal alert-triage-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("alertTriage.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:alertTriageOpen', false)"><X /></button>
        </header>
        <p class="muted alert-triage-hint">{{ t("alertTriage.hint") }}</p>
        <textarea :value="alertTriagePayload" @input="emit('update:alertTriagePayload', ($event.target as HTMLTextAreaElement).value)" class="mono alert-triage-payload" rows="6" :placeholder="t('alertTriage.placeholder')" :disabled="alertTriageBusy" spellcheck="false" autofocus />
        <p v-if="alertTriageError" class="task-error" role="alert">{{ alertTriageError }}</p>
        <footer class="alert-triage-actions">
          <button class="primary-button" :disabled="alertTriageBusy || !sanitizeTriagePayload(alertTriagePayload)" @click="emit('run-alert-triage')">{{ t("alertTriage.analyze") }}</button>
        </footer>
        <div v-if="alertTriageBusy" class="empty compact" role="status"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="!alertTriageResult && !alertTriageError" class="empty compact">{{ t("alertTriage.emptyResult") }}</div>
        <div v-else-if="alertTriageResult" class="alert-triage-result">
          <div class="alert-triage-summary">
            <span class="alert-severity-badge" :class="severityClass(alertTriageResult.normalized.severity)">{{ t(`alertTriage.severity.${severityClass(alertTriageResult.normalized.severity)}`) }}</span>
            <span class="alert-category">{{ t(`alertTriage.category.${alertTriageResult.category}`) }}</span>
            <strong v-if="alertTriageResult.normalized.title" class="alert-title">{{ alertTriageResult.normalized.title }}</strong>
          </div>
          <p v-if="alertTriageResult.normalized.message" class="muted alert-message mono">{{ alertTriageResult.normalized.message }}</p>
          <p v-if="!alertTriageResult.suggestions.length" class="muted">{{ t("alertTriage.emptyResult") }}</p>
          <ul v-else class="alert-suggestion-list">
            <li v-for="suggestion in alertTriageResult.suggestions" :key="suggestion.command" class="alert-suggestion-row">
              <code class="mono alert-suggestion-command">{{ suggestion.command }}</code>
              <span class="alert-purpose muted">{{ purposeKeyLabel(suggestion.purposeKey, t) }}</span>
              <button :disabled="!hasSession" :title="!hasSession ? t('alertTriage.noSession') : ''" :aria-label="!hasSession ? t('alertTriage.noSession') : ''" @click="emit('send-suggestion', suggestion.command)">{{ t("alertTriage.sendToTerminal") }}</button>
            </li>
          </ul>
          <footer v-if="alertTriageResult.suggestions.length" class="alert-triage-actions">
            <button @click="emit('copy-suggestions')">{{ t("alertTriage.copyAll") }}</button>
          </footer>
        </div>
      </DialogContent>
    </Dialog>
    <Dialog :open="!!pasteConfirm" @update:open="(open) => { if (!open) emit('resolve-paste-confirm', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="pasteConfirm">
        <header>
          <DialogTitle>{{ pasteConfirm.danger ? t("terminalDanger.title") : t("terminalPasteConfirm.title") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('resolve-paste-confirm', false)"><X /></button>
        </header>
        <div v-if="pasteConfirm.danger" class="destructive-copy warning">
          <span class="destructive-icon"><TriangleAlert /></span>
          <div>
            <strong>{{ t("terminalDanger.detected") }}</strong>
            <p class="task-error mono">{{ pasteConfirm.hits.map((hit) => hit.label).join(" · ") }}</p>
            <p class="muted">{{ t("terminalDanger.desc") }}</p>
          </div>
        </div>
        <p v-else class="muted">{{ t("terminalPasteConfirm.desc") }}</p>
        <p class="muted">{{ t("terminalPasteConfirm.summary", { lines: pasteConfirm.lines, chars: pasteConfirm.chars }) }}<template v-if="pasteConfirm.lines > 1"> · {{ t("terminalPasteConfirm.multiline") }}</template></p>
        <pre class="command-output mono">{{ pasteConfirm.preview }}</pre>
        <footer>
          <button @click="emit('resolve-paste-confirm', false)">{{ t("cancel") }}</button>
          <button :class="pasteConfirm.danger ? 'danger-button' : 'primary-button'" @click="emit('resolve-paste-confirm', true)">{{ t("terminalPasteConfirm.confirm") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- SSH-H1：通用应用内确认弹窗（宿主沙箱 iframe 无 allow-modals，window.confirm
         恒 false）——预览关闭/大文件预览/重命名覆盖/粘贴覆盖/网页大文件下载/进程强杀
         此前在真机上全部静默失效，现统一走本弹窗（Promise 原语同 pasteConfirm）。 -->
    <Dialog :open="!!pendingConfirmDialog" @update:open="(open) => { if (!open) emit('resolve-pending-confirm', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <template v-if="pendingConfirmDialog">
        <header>
          <DialogTitle>{{ t("confirm") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('resolve-pending-confirm', false)"><X /></button>
        </header>
        <p class="muted confirm-message">{{ pendingConfirmDialog.message }}</p>
        <footer>
          <button @click="emit('resolve-pending-confirm', false)">{{ t("cancel") }}</button>
          <button :class="pendingConfirmDialog.danger ? 'danger-button' : 'primary-button'" @click="emit('resolve-pending-confirm', true)">{{ t("confirm") }}</button>
        </footer>
        </template>
      </DialogContent>
    </Dialog>
    <!-- 本地终端确认：SSH 会话仍连着时先关闭再进入本地模式 -->
    <Dialog :open="localOpenConfirmOpen" @update:open="(open) => { if (!open) emit('update:localOpenConfirmOpen', false); }">
      <DialogContent class="modal small-modal" @escape-key-down.prevent>
        <header>
          <DialogTitle>{{ t("localTerminal.openConfirmTitle") }}</DialogTitle>
          <button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:localOpenConfirmOpen', false)"><X /></button>
        </header>
        <p class="muted">{{ t("localTerminal.openConfirm") }}</p>
        <footer>
          <button @click="emit('update:localOpenConfirmOpen', false)">{{ t("cancel") }}</button>
          <button class="primary-button" @click="emit('confirm-local-terminal')">{{ t("localTerminal.open") }}</button>
        </footer>
      </DialogContent>
    </Dialog>
</template>

<style scoped>
/* 安全/确认弹窗内部排版（原 App scoped 规则随 DOM 迁入；弹窗骨架走全局）。 */
.confirm-message { white-space: pre-line; }
.rdp-cert-status { margin: 0 0 8px; font-size: 11px; }
.rdp-cert-expires { margin: 0 0 8px; font-size: 11px; }
.agent-prompt-meta { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 4px 0; font-size: 12px; }
.agent-risk-badge { flex: 0 0 auto; border-radius: 999px; padding: 2px 8px; font-size: 10px; font-weight: 600; letter-spacing: .02em; }
.agent-risk-badge.low { border: 1px solid var(--border); background: var(--accent); color: var(--muted-foreground); }
.agent-risk-badge.elevated { border: 1px solid color-mix(in srgb, var(--destructive) 55%, transparent); background: color-mix(in srgb, var(--destructive) 12%, transparent); color: var(--destructive); }
.agent-prompt-command { display: flex; flex-direction: column; gap: 5px; padding: 4px 0 8px; font-size: 12px; }
.agent-prompt-command span { color: var(--muted-foreground); }
.agent-prompt-command textarea { width: 100%; resize: vertical; border: 1px solid var(--border); border-radius: 5px; padding: 6px 8px; background: var(--background); color: var(--foreground); font-family: var(--terminal-font-family); font-size: 12px; }
.agent-prompt-command textarea:focus { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); outline: none; }
.agent-prompt-countdown { padding-bottom: 10px; }
</style>
