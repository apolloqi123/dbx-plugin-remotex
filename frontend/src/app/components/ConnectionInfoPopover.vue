<script setup lang="ts">
import { Info } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import { formatLatency } from "../../features/connections/lib/connectionInfo";

/** 连接信息入口（工具条 Info 按钮 + 只读摘要弹层）。
 *  开关/延迟探测/认证方式仍由 App 的 useConnectionInfo 持有，本组件只渲染。 */
interface ConnectionInfoSummary {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  readOnly?: boolean;
}

defineProps<{
  connectionInfoOpen: boolean;
  connection: ConnectionInfoSummary;
  metricsDistroBadge: { color: string; name: string; label: string } | null;
  connectionAuthMethodLabel: string;
  connectionReadOnly: boolean;
  connectionLatency: number | null;
  connectionLatencyBusy: boolean;
  connectionLatencyFailed: boolean;
  connected: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:connectionInfoOpen", value: boolean): void;
  (e: "toggle"): void;
  (e: "measure"): void;
}>();
</script>

<template>
  <Popover :open="connectionInfoOpen" @update:open="(value) => emit('update:connectionInfoOpen', value)">
    <PopoverAnchor as-child>
      <button type="button" class="icon-button icon-neutral" :title="t('connectionInfo')" :aria-label="t('connectionInfo')" :aria-expanded="connectionInfoOpen" @pointerdown.stop @click.stop="emit('toggle')"><Info /></button>
    </PopoverAnchor>
    <PopoverContent class="popover connection-info-popover" align="start" :side-offset="5">
    <h3>{{ t("connectionInfo") }}</h3>
    <dl class="connection-info-grid">
      <dt>{{ t("connectionInfoHost") }}</dt><dd class="mono"><span v-if="metricsDistroBadge" class="distro-badge" :style="{ backgroundColor: metricsDistroBadge.color }" :title="metricsDistroBadge.name">{{ metricsDistroBadge.label }}</span> {{ connection.host || connection.name || "–" }}</dd>
      <dt>{{ t("connectionInfoPort") }}</dt><dd class="mono">{{ connection.port || 22 }}</dd>
      <dt>{{ t("connectionInfoUser") }}</dt><dd class="mono">{{ connection.username || "–" }}</dd>
      <dt>{{ t("connectionInfoAuth") }}</dt><dd>{{ connectionAuthMethodLabel }}</dd>
      <template v-if="connection.readOnly || connectionReadOnly"><dt>{{ t("readOnly") }}</dt><dd>{{ t("yes") }}</dd></template>
      <dt>{{ t("connectionInfoLatency") }}</dt>
      <dd>
        <span class="mono">{{ connectionLatencyBusy ? t("connectionInfoMeasuring") : formatLatency(connectionLatency) }}</span>
        <span v-if="connectionLatencyFailed && !connectionLatencyBusy" class="task-error">{{ t("connectionInfoFailed") }}</span>
        <button class="link-button" :disabled="connectionLatencyBusy || !connected" @click="emit('measure')">{{ t("connectionInfoMeasure") }}</button>
      </dd>
    </dl>
    </PopoverContent>
  </Popover>
</template>

<style scoped>
/* 连接信息面板（工具栏下拉，只读；根规则见 style.css 全局） */
.connection-info-popover h3 { margin: 4px 0 8px; font-size: 12px; }
.connection-info-grid { display: grid; grid-template-columns: auto 1fr; gap: 6px 14px; margin: 0; font-size: 12px; }
.connection-info-grid dt { max-width: 16ch; overflow: hidden; color: var(--muted-foreground); text-overflow: ellipsis; white-space: nowrap; }
.connection-info-grid dd { display: flex; min-width: 0; align-items: center; gap: 8px; margin: 0; overflow-wrap: anywhere; }
.connection-info-grid .task-error { font-size: 10px; }
.connection-info-grid .link-button { flex: 0 0 auto; align-self: center; font-size: 10px; }
</style>
