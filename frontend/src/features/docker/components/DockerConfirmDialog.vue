<script setup lang="ts">
// kill/rm 强制确认弹层：标题带动作名，正文带 CLI/动作/容器名；Esc 由面板的
// document 捕获监听统一取消（见 useDockerContainers），这里的
// @escape-key-down.prevent 保持原样以挡住 reka 自带关闭。
import { X } from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { ACTION_LABEL_KEYS, type DockerConfirmTarget } from "../lib/dockerPanelModel";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  target: DockerConfirmTarget | null;
  busy: boolean;
  /** 当前 CLI（确认正文里的命令前缀）。 */
  cli: string;
}>();

const emit = defineEmits<{
  resolve: [accepted: boolean];
}>();
</script>

<template>
  <Dialog :open="!!props.target" @update:open="(open: boolean) => { if (!open) emit('resolve', false); }">
    <DialogContent class="docker-dialog" @escape-key-down.prevent>
      <div class="docker-dialog-header">
        <DialogTitle>
          {{ props.target ? props.t("docker.confirmTitle", { action: props.t(ACTION_LABEL_KEYS[props.target.action]) }) : "" }}
        </DialogTitle>
        <button type="button" class="icon-button" :title="props.t('docker.cancel')" :aria-label="props.t('docker.cancel')" @click="emit('resolve', false)"><X /></button>
      </div>
      <p class="docker-confirm-body mono">
        {{ props.target ? props.t("docker.confirmBody", { cli: props.cli || "docker", action: props.target.action, name: props.target.name }) : "" }}
      </p>
      <div class="docker-dialog-actions">
        <button type="button" class="docker-secondary-button" :disabled="props.busy" @click="emit('resolve', false)">
          {{ props.t("docker.cancel") }}
        </button>
        <button type="button" class="docker-danger-button" :disabled="props.busy" @click="emit('resolve', true)">
          {{ props.busy ? props.t("docker.actionBusy") : props.t("docker.confirmOk") }}
        </button>
      </div>
    </DialogContent>
  </Dialog>
</template>

<style scoped>
.docker-dialog {
  max-width: 460px;
}

.docker-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.docker-confirm-body {
  margin: 10px 0;
  word-break: break-all;
}

.docker-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 8px;
}

.docker-secondary-button,
.docker-danger-button {
  border-radius: 6px;
  padding: 4px 12px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid var(--border, rgba(128, 128, 128, 0.35));
  background: transparent;
  color: inherit;
}

.docker-danger-button {
  background: var(--danger, #d64545);
  border-color: var(--danger, #d64545);
  color: #fff;
}

.docker-danger-button:disabled,
.docker-secondary-button:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
