<script setup lang="ts">
// Logs 抽屉：docker/logs（tail 可选）+ inspect 摘要 + 刷新。加载/状态由面板
// 持有，这里只渲染并派发 tail 变更与刷新。Esc 关闭沿用 reka 自带行为（
// @escape-key-down.prevent 与确认弹层一致，避免同一次 Esc 穿透）。
import { Loader2, RefreshCw, X } from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { TAIL_OPTIONS, type DockerContainer, type DockerInspectSummary } from "../lib/dockerPanelModel";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  open: boolean;
  target: DockerContainer | null;
  text: string;
  meta: DockerInspectSummary | null;
  busy: boolean;
  error: string;
  /** 当前 tail 行数（字符串形式，作为 Select 的 model-value）。 */
  tail: string;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  /** tail 变更：面板写入后立即重取。 */
  "update:tail": [tail: string];
  refresh: [];
}>();
</script>

<template>
  <Dialog :open="props.open" @update:open="(open: boolean) => emit('update:open', open)">
    <DialogContent class="docker-dialog docker-logs-dialog" @escape-key-down.prevent>
      <div class="docker-dialog-header">
        <DialogTitle>
          {{ props.target ? props.t("docker.logsTitle", { name: props.target.name || props.target.id.slice(0, 12) }) : "" }}
        </DialogTitle>
        <button type="button" class="icon-button" :title="props.t('docker.close')" :aria-label="props.t('docker.close')" @click="emit('update:open', false)"><X /></button>
      </div>
      <div class="docker-logs-bar">
        <label class="docker-tail-label">
          <span>{{ props.t("docker.logsTail") }}</span>
          <Select
            :model-value="props.tail"
            @update:model-value="(value: unknown) => emit('update:tail', String(value))"
          >
            <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem v-for="option in TAIL_OPTIONS" :key="option" :value="option">{{ option }}</SelectItem>
            </SelectContent>
          </Select>
        </label>
        <button type="button" class="icon-button" :title="props.t('docker.logsRefresh')" :aria-label="props.t('docker.logsRefresh')" @click="emit('refresh')">
          <Loader2 v-if="props.busy" class="spinning" />
          <RefreshCw v-else />
        </button>
      </div>
      <p v-if="props.error" class="docker-hint docker-hint-error">{{ props.t("docker.logsLoadFailed", { error: props.error }) }}</p>
      <div v-if="props.meta" class="docker-logs-meta mono">
        {{ props.meta.image }} · {{ props.meta.status }}<template v-if="props.meta.health"> · {{ props.meta.health }}</template>
      </div>
      <pre v-if="props.text" class="docker-logs-pre mono">{{ props.text }}</pre>
      <p v-else-if="!props.busy && !props.error" class="docker-empty">{{ props.t("docker.logsEmpty") }}</p>
    </DialogContent>
  </Dialog>
</template>

<style scoped>
.docker-dialog {
  max-width: 460px;
}

.docker-logs-dialog {
  max-width: 820px;
  width: min(820px, 90vw);
}

.docker-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.docker-logs-bar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.docker-tail-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.docker-logs-meta {
  font-size: 11px;
  opacity: 0.65;
}

.docker-logs-pre {
  margin: 0;
  padding: 8px;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.1);
  max-height: 50vh;
  overflow: auto;
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-all;
}

.docker-empty {
  opacity: 0.65;
  font-size: 12px;
  padding: 8px 4px;
}

.docker-hint {
  font-size: 11px;
  opacity: 0.7;
}

.docker-hint-error {
  color: var(--danger, #d64545);
  opacity: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.spinning {
  animation: docker-spin 1s linear infinite;
}

@keyframes docker-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
