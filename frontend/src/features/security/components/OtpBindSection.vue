<script setup lang="ts">
// 单条 OTP 的连接绑定区：已绑定连接列表 + 下拉选择（宿主不支持
// host.listConnections 时降级为手输 connectionId）。绑定结果通过
// update:bindings 回写父级，父级仍持有唯一的 bindings 状态。
import { computed } from "vue";
import { Unlink } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { boundConnectionsOf, type OtpEntryView } from "../lib/otpPanel";

const props = defineProps<{
  entry: OtpEntryView;
  bindings: Record<string, string>;
  connections: Array<{ id: string; name: string }>;
  /** 下拉当前值（由父级按条目持有，关闭再打开不丢草稿）。 */
  pick: string;
  /** 手输 connectionId 草稿（同上）。 */
  manual: string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (event: "update:bindings", bindings: Record<string, string>): void;
  (event: "update:pick", value: string): void;
  (event: "update:manual", value: string): void;
  (event: "error", cause: unknown): void;
}>();

const connectionName = computed(() => {
  const names = new Map<string, string>();
  for (const connection of props.connections) names.set(connection.id, connection.name || connection.id);
  return (id: string) => names.get(id) ?? id;
});

const manualModel = computed({
  get: () => props.manual,
  set: (value: string) => emit("update:manual", value),
});

function onPick(value: unknown) {
  emit("update:pick", typeof value === "string" ? value : "");
}

async function bindEntry() {
  const connectionId = (props.pick || props.manual || "").trim();
  if (!connectionId) return;
  try {
    await window.dbxPlugin.invoke("otp/bind", { connectionId, entryId: props.entry.id });
    emit("update:bindings", { ...props.bindings, [connectionId]: props.entry.id });
    emit("update:pick", "");
    emit("update:manual", "");
  } catch (cause) {
    emit("error", cause);
  }
}

async function unbindConnection(connectionId: string) {
  try {
    const payload = await window.dbxPlugin.invoke<{ removed: boolean }>("otp/unbind", { connectionId });
    if (payload?.removed) {
      const nextBindings = { ...props.bindings };
      delete nextBindings[connectionId];
      emit("update:bindings", nextBindings);
    }
  } catch (cause) {
    emit("error", cause);
  }
}
</script>

<template>
  <div class="otp-bind">
    <p v-if="!boundConnectionsOf(bindings, entry.id).length" class="otp-bind-none">{{ t("otpPanel.bindNone") }}</p>
    <div v-for="connectionId in boundConnectionsOf(bindings, entry.id)" :key="connectionId" class="otp-bind-row">
      <span class="otp-bind-target" :title="connectionId">{{ connectionName(connectionId) }}</span>
      <button type="button" class="icon-button" :title="t('otpPanel.unbind')" :aria-label="t('otpPanel.unbind')" @click="() => void unbindConnection(connectionId)"><Unlink /></button>
    </div>
    <div v-if="connections.length" class="otp-bind-pick">
      <Select :model-value="pick || undefined" @update:model-value="onPick">
        <SelectTrigger size="xs" class="otp-bind-select"><SelectValue :placeholder="t('otpPanel.bindPick')" /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="connection in connections" :key="connection.id" :value="connection.id">{{ connection.name || connection.id }}</SelectItem>
        </SelectContent>
      </Select>
      <button type="button" class="otp-bind-apply" :disabled="!pick" @click="() => void bindEntry()">{{ t("otpPanel.bindApply") }}</button>
    </div>
    <p v-else class="otp-bind-hint">{{ t("otpPanel.bindManualHint") }}</p>
    <input
      v-if="!connections.length"
      v-model="manualModel"
      class="otp-bind-manual"
      :placeholder="t('otpPanel.bindManual')"
      spellcheck="false"
      @keydown.enter="() => void bindEntry()"
    />
  </div>
</template>

<style scoped>
.otp-bind {
  display: flex;
  flex-direction: column;
  gap: 6px;
  border-top: 1px dashed var(--border);
  padding-top: 6px;
}
.otp-bind-none, .otp-bind-hint {
  margin: 0;
  font-size: 11px;
  color: var(--muted-foreground);
}
.otp-bind-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.otp-bind-target {
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
}
.otp-bind-pick {
  display: flex;
  align-items: center;
  gap: 6px;
}
.otp-bind-select { flex: 1 1 auto; min-width: 0; }
.otp-bind-apply {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 2px 8px;
  background: transparent;
  color: var(--primary);
  font-size: 11px;
  cursor: pointer;
}
.otp-bind-apply:hover:not(:disabled) { background: var(--accent); }
.otp-bind-apply:disabled { opacity: 0.5; cursor: default; }
.otp-bind-manual {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 3px 6px;
  background: transparent;
  color: var(--foreground);
  font-size: 11px;
}
</style>
