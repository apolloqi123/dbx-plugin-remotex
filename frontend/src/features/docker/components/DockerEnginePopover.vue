<script setup lang="ts">
// 引擎设置（齿轮弹层）：紧凑表单，保存即生效（@change 触发 apply）。
// 草稿由面板持有（打开时从已保存设置复制），这里只做受控编辑与校验提示。
import { computed } from "vue";
import { Settings } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../../shared/ui/popover/index";
import { validateDockerEngineSettings, type DockerEngineSettings } from "../lib/dockerEngine";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  open: boolean;
  draft: DockerEngineSettings;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  "update:draft": [draft: DockerEngineSettings];
  /** 齿轮按钮：由面板切换开合并重置草稿。 */
  toggle: [];
  /** 字段 @change：保存即生效。 */
  apply: [];
}>();

const engineErrors = computed(() => validateDockerEngineSettings(props.draft).errors);

/** 受控字段：写回整份草稿，保持父组件的草稿引用更新（不直接改 prop 对象）。 */
function draftField(name: keyof DockerEngineSettings) {
  return computed({
    get: () => props.draft[name],
    set: (value: string) => emit("update:draft", { ...props.draft, [name]: value }),
  });
}

const cli = draftField("cli");
const socket = draftField("socket");
const host = draftField("host");
</script>

<template>
  <Popover :open="props.open" @update:open="(open: boolean) => emit('update:open', open)">
    <PopoverAnchor as-child>
      <button type="button" class="icon-button" :class="{ 'is-active': props.open }" :title="props.t('docker.engineSettings')" :aria-label="props.t('docker.engineSettings')" @click="emit('toggle')">
        <Settings />
      </button>
    </PopoverAnchor>
    <PopoverContent class="popover docker-engine-popover" align="end" :side-offset="5">
      <h3>{{ props.t("docker.engineSettings") }}</h3>
      <label class="docker-engine-field">
        <span>{{ props.t("docker.engineCli") }}</span>
        <input
          v-model="cli"
          class="mono"
          :placeholder="props.t('docker.engineCliPlaceholder')"
          spellcheck="false"
          @change="emit('apply')"
        />
      </label>
      <label class="docker-engine-field">
        <span>{{ props.t("docker.engineSocket") }}</span>
        <input
          v-model="socket"
          class="mono"
          :placeholder="props.t('docker.engineSocketPlaceholder')"
          spellcheck="false"
          @change="emit('apply')"
        />
      </label>
      <label class="docker-engine-field">
        <span>{{ props.t("docker.engineHost") }}</span>
        <input
          v-model="host"
          class="mono"
          :placeholder="props.t('docker.engineHostPlaceholder')"
          spellcheck="false"
          @change="emit('apply')"
        />
      </label>
      <p v-if="engineErrors.cli" class="docker-hint docker-hint-error">{{ props.t("docker.engineInvalidCli") }}</p>
      <p v-else-if="engineErrors.endpoints === 'conflict'" class="docker-hint docker-hint-error">{{ props.t("docker.engineConflict") }}</p>
      <p v-else-if="engineErrors.endpoints" class="docker-hint docker-hint-error">{{ props.t("docker.engineInvalidEndpoint") }}</p>
      <p class="docker-hint">{{ props.t("docker.engineHint") }}</p>
    </PopoverContent>
  </Popover>
</template>

<style scoped>
/* 引擎设置（齿轮弹层）：紧凑表单，保存即生效。 */
.docker-engine-popover {
  min-width: 300px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.docker-engine-field {
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 12px;
}

.docker-engine-field input {
  border: 1px solid var(--border, rgba(128, 128, 128, 0.35));
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 12px;
  background: var(--background, transparent);
  color: inherit;
}

.docker-engine-field input::placeholder {
  opacity: 0.45;
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
</style>
