import { ref, type Ref } from "vue";
import { reopenHostConnection } from "../lib/hostReopen";
import type { ConnectLog } from "../lib/connectLog";
import type { HostKeyChallenge } from "../lib/sshConnectionTypes";

export interface SshHostKeyOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  connectLog: ConnectLog;
  hostKeyPrompt: Ref<HostKeyChallenge | undefined>;
  rememberHostKey: Ref<boolean>;
  connectionId: Readonly<Ref<string>>;
}

/** 宿主重开（host reopen）与 host-key 挑战应答：由 useSshConnection 先行装配，
 *  函数并入其公开 API，调用方（App.vue / 重连梯子 / 连接卡片）无需感知拆分。 */
export function useSshHostKey(options: SshHostKeyOptions) {
  const { t, showError, connectLog, hostKeyPrompt, rememberHostKey, connectionId } = options;

// 手动重连前请宿主按当前最新配置重开连接：连接在侧边栏被编辑（如改密码）后，
// 宿主会摘掉 connected 标记且不回推插件，sidecar 里存的凭据就此过期——不先
// 重开的话 openSession 只会拿旧凭据反复失败。宿主以打开连接的同款流程重新
// 下发配置（含 vault 里的最新凭据）。失败记录下来，避免后续对空连接表
// 反复发起 ssh/session/open；旧宿主仍可在连接表已有记录时继续拨号。
const hostReopenFailed = ref(false);
async function requestHostReopenConnection(): Promise<boolean> {
  if (!connectionId.value) return false;
  const restored = await reopenHostConnection(window.dbxPlugin, connectionId.value);
  hostReopenFailed.value = !restored;
  return restored;
}

async function resolveHostKey(accept: boolean) {
  const prompt = hostKeyPrompt.value;
  if (!prompt) return;
  hostKeyPrompt.value = undefined;
  try {
    await window.dbxPlugin.invoke("connection/challenge/resolve", {
      challengeId: prompt.challengeId,
      operationId: prompt.operationId,
      accept,
      remember: accept && rememberHostKey.value,
    });
    connectLog.push("info", t("connectCard.log.hostKeyResolved"));
  } catch (cause) {
    showError(cause, "terminal");
  }
}

  return { requestHostReopenConnection, resolveHostKey, hostReopenFailed };
}
