<script setup lang="ts">
// 设置·安全分栏：已知主机（ssh/knownHosts/list|remove）与本机密钥发现
// （keys/discover）。数据面 RPC 与加载状态在弹窗（打开设置时并行拉取），
// 本组件只读 + 上抛刷新/删除意图。
import { KeyRound, Loader2, Trash2 } from "@lucide/vue";
import type { DiscoveredKey, KnownHostEntry } from "../lib/settingsModel";

defineProps<{
  knownHosts: KnownHostEntry[];
  knownHostsLoading: boolean;
  knownHostsError: string;
  localKeys: DiscoveredKey[];
  localKeysLoading: boolean;
  localKeysError: string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "refresh-known-hosts"): void;
  (e: "refresh-local-keys"): void;
  (e: "remove-known-host", entry: KnownHostEntry): void;
}>();

function shortFingerprint(fingerprint: string) {
  if (fingerprint.length <= 20) return fingerprint;
  return `${fingerprint.slice(0, 17)}…`;
}
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("knownHosts.title") }}</h3>
    <div v-if="knownHostsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <p v-else-if="knownHostsError" class="task-error">{{ knownHostsError }} <button class="link-button" @click="emit('refresh-known-hosts')">{{ t("refresh") }}</button></p>
    <div v-else-if="!knownHosts.length" class="empty compact">{{ t("knownHosts.empty") }}</div>
    <ul v-else class="settings-list">
      <li v-for="(entry, index) in knownHosts" :key="`${entry.host}:${entry.port}:${entry.keyType}:${index}`">
        <div class="settings-list-main">
          <strong class="mono">{{ entry.host }}:{{ entry.port }}</strong>
          <span class="muted">{{ entry.keyType }} · <span class="mono" :title="entry.fingerprint">{{ shortFingerprint(entry.fingerprint) }}</span></span>
        </div>
        <button class="icon-button" :title="t('delete')" :aria-label="t('delete')" @click="emit('remove-known-host', entry)"><Trash2 /></button>
      </li>
    </ul>

    <h3 class="settings-section-title">{{ t("keysPanel.title") }}</h3>
    <div v-if="localKeysLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <p v-else-if="localKeysError" class="task-error">{{ localKeysError }} <button class="link-button" @click="emit('refresh-local-keys')">{{ t("refresh") }}</button></p>
    <div v-else-if="!localKeys.length" class="empty compact">{{ t("keysPanel.empty") }}</div>
    <ul v-else class="settings-list">
      <li v-for="key in localKeys" :key="key.path">
        <div class="settings-list-main">
          <strong class="mono" :title="key.path">{{ key.path }}</strong>
          <span class="muted">{{ key.algorithm }} · <span class="mono" :title="key.fingerprint">{{ shortFingerprint(key.fingerprint) }}</span><template v-if="key.hasPassphrase"> · {{ t("keysPanel.hasPassphrase") }}</template></span>
        </div>
        <KeyRound class="settings-key-icon" />
      </li>
    </ul>
    <p class="muted settings-note">{{ t("keysPanel.hint") }}</p>
  </div>
</template>
