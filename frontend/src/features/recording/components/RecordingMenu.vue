<script setup lang="ts">
// 工具栏统一录制入口：一个图标内选择「开始/停止录制」或打开「录制记录」。
// 点击图标只展开菜单，不会直接开始录制；开始录制沿用现有 3 秒倒计时。
import { computed, ref } from "vue";
import { Circle, Disc, ListVideo, Square, X } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../../shared/ui/popover/index";
import WorkbenchGlyph from "../../../shared/ui/icons/WorkbenchGlyph.vue";
import type { RecordingMenuActions } from "../lib/recordingMenu";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  canRecord: boolean;
  recordingActive: boolean;
  recordCountdown: number | null;
  recordingElapsedSec: number;
  recordingsCount: number;
  recordingsOpen: boolean;
  compact?: boolean;
  actions: RecordingMenuActions;
}>();

const open = ref(false);

const elapsed = computed(() => {
  const total = Math.max(0, Math.floor(props.recordingElapsedSec));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
});

const status = computed(() => {
  if (props.recordingActive) return props.t("recordingMenuActive");
  if (props.recordCountdown !== null) return props.t("recordingMenuCountdown", { seconds: props.recordCountdown });
  return props.t("recordingMenuIdle");
});

function startOrStop() {
  if (!props.canRecord) return;
  open.value = false;
  props.actions.toggleRecording();
}

function cancelCountdown() {
  open.value = false;
  props.actions.cancelRecordCountdown();
}

function openRecords() {
  open.value = false;
  props.actions.toggleRecordings();
}
</script>

<template>
  <div>
    <Popover v-model:open="open">
      <PopoverAnchor as-child>
        <button
          class="icon-button recording-trigger"
          :class="{ compact, 'is-recording': recordingActive, 'is-active': recordingsOpen }"
          :title="t('recordingMenuTitle')"
          :aria-label="t('recordingMenuTitle')"
          :aria-pressed="recordingActive"
          @pointerdown.stop
          @click.stop="open = !open"
        >
          <WorkbenchGlyph name="record" />
          <span v-if="recordingActive" class="recording-live-dot" aria-hidden="true" />
        </button>
      </PopoverAnchor>
      <PopoverContent class="popover recording-menu-popover" align="end" :side-offset="5">
        <header class="recording-menu-head">
          <span class="recording-menu-icon" :class="{ live: recordingActive }"><Disc /></span>
          <div class="recording-menu-head-text">
            <strong>{{ t("recordingMenuTitle") }}</strong>
            <span :class="{ live: recordingActive }">{{ status }}<template v-if="recordingActive"> · {{ elapsed }}</template></span>
          </div>
        </header>
        <div class="recording-menu-list">
          <button v-if="recordCountdown !== null" class="recording-menu-item" @click="cancelCountdown"><X />{{ t("recordingMenuCancel") }}</button>
          <button v-else class="recording-menu-item" :class="{ stop: recordingActive }" :disabled="!canRecord" @click="startOrStop">
            <Square v-if="recordingActive" />
            <Circle v-else />
            {{ recordingActive ? t("recordingStop") : t("recordingMenuStart") }}
          </button>
          <button class="recording-menu-item" @click="openRecords">
            <ListVideo />{{ t("recordingsTitle") }}
            <span v-if="recordingsCount" class="recording-menu-count">{{ recordingsCount }}</span>
          </button>
        </div>
        <p class="recording-menu-hint">{{ canRecord ? t("recordingMenuHint") : t("recordingMenuNoTarget") }}</p>
      </PopoverContent>
    </Popover>
  </div>
</template>

<style scoped>
.recording-trigger { position: relative; }
.recording-live-dot { position: absolute; top: 4px; right: 4px; width: 5px; height: 5px; border-radius: 50%; background: var(--destructive); box-shadow: 0 0 0 1px var(--background); }
</style>
