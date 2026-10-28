<script setup lang="ts">
// 工具栏端口映射快捷弹层：摘要 + 运行中/预设的一键启停，底部进入完整管理器。
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Check,
  ChevronRight,
  CircleStop,
  Copy,
  Loader2,
  Play,
  Plus,
  Waypoints,
} from "@lucide/vue";
import { formatForwardBytes, formatForwardRoute, type ForwardCard, type ForwardKind } from "../lib/portForward";
import type { PortForwardsController } from "../composables/usePortForwards";
import type { TunnelProfile } from "../lib/tunnelProfiles";

const props = defineProps<{
  controller: PortForwardsController;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{ add: []; manage: [] }>();

const t = (key: string, values: Record<string, string | number> = {}) => props.t(key, values);

const { cards, summary, loading, busyId, startingAll, error } = props.controller;
const copiedKey = ref("");
let copiedTimer: ReturnType<typeof setTimeout> | undefined;

const ICONS: Record<ForwardKind, typeof ArrowDownToLine> = {
  local: ArrowDownToLine,
  remote: ArrowUpFromLine,
  dynamic: Waypoints,
};
const TONES: Record<ForwardKind, string> = {
  local: "ui-tone-blue",
  remote: "ui-tone-amber",
  dynamic: "ui-tone-emerald",
};

const running = computed(() => cards.value.filter((card) => card.forward));
const idle = computed(() => cards.value.filter((card) => !card.forward));

function rowRoute(card: ForwardCard) {
  return formatForwardRoute({ ...card, boundPort: card.forward?.boundPort ?? 0 });
}

async function copy(card: ForwardCard) {
  try {
    await props.controller.copyAddress({
      listenHost: card.listenHost,
      listenPort: card.listenPort,
      boundPort: card.forward?.boundPort ?? 0,
    });
    copiedKey.value = card.key;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => { copiedKey.value = ""; }, 1600);
  } catch {
    /* Clipboard fallbacks already degrade; keep the menu quiet. */
  }
}

function stop(card: ForwardCard) {
  if (card.forward) void props.controller.stop(card.forward.id);
}

function start(card: ForwardCard<TunnelProfile>) {
  if (card.profile) void props.controller.startSaved([card.profile]);
}

onMounted(() => { void props.controller.open(); });
onBeforeUnmount(() => { if (copiedTimer) clearTimeout(copiedTimer); });
</script>

<template>
  <section class="forward-menu" :aria-label="t('forwards.title')">
    <header class="forward-menu-head">
      <span class="forward-menu-title">{{ t("forwards.title") }}</span>
      <span class="forward-menu-summary"><span class="forward-dot" />{{ t("forwards.runningCount", { count: summary.running }) }}</span>
    </header>
    <div class="forward-menu-meta">
      <span>{{ t("forwards.connectionsCount", { count: summary.connectionsActive }) }}</span>
      <span>↑{{ formatForwardBytes(summary.bytesUp) }} ↓{{ formatForwardBytes(summary.bytesDown) }}</span>
    </div>

    <p v-if="error" class="forward-form-error forward-menu-error" role="alert">{{ error }}</p>
    <div v-if="loading && !cards.length" class="forward-menu-empty"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <p v-else-if="!cards.length" class="forward-menu-empty">{{ t("forwards.quickEmpty") }}</p>

    <template v-else>
      <div v-if="running.length" class="forward-menu-group">
        <h3>{{ t("forwards.activeTitle") }}</h3>
        <ul>
          <li v-for="card in running" :key="card.key" class="forward-menu-row">
            <span class="forward-menu-icon" :class="TONES[card.kind]"><component :is="ICONS[card.kind]" /></span>
            <button type="button" class="forward-menu-route" :title="t('forwards.copyAddress')" @click="copy(card)">
              <code>{{ rowRoute(card) }}</code>
              <span class="forward-menu-copy" :class="{ 'is-copied': copiedKey === card.key }"><Check v-if="copiedKey === card.key" /><Copy v-else /></span>
            </button>
            <span class="forward-menu-state" :class="`is-${card.state}`">{{ t(`forwards.state.${card.state}`) }}</span>
            <button type="button" class="forward-menu-action" :title="t('forwards.stop')" :aria-label="t('forwards.stop')" :disabled="busyId !== null" @click="stop(card)">
              <Loader2 v-if="busyId === card.forward?.id" class="spinning" /><CircleStop v-else />
            </button>
          </li>
        </ul>
      </div>

      <div v-if="idle.length" class="forward-menu-group">
        <h3>{{ t("forwards.savedTitle") }}</h3>
        <ul>
          <li v-for="card in idle" :key="card.key" class="forward-menu-row">
            <span class="forward-menu-icon" :class="TONES[card.kind]"><component :is="ICONS[card.kind]" /></span>
            <button type="button" class="forward-menu-route" :title="t('forwards.copyAddress')" @click="copy(card)">
              <code>{{ rowRoute(card) }}</code>
              <span class="forward-menu-copy" :class="{ 'is-copied': copiedKey === card.key }"><Check v-if="copiedKey === card.key" /><Copy v-else /></span>
            </button>
            <button type="button" class="forward-menu-action forward-menu-action--start" :title="t('forwards.startSaved')" :aria-label="t('forwards.startSaved')" :disabled="startingAll || busyId !== null" @click="start(card)">
              <Loader2 v-if="startingAll" class="spinning" /><Play v-else />
            </button>
          </li>
        </ul>
      </div>
    </template>

    <footer class="forward-menu-footer">
      <button type="button" class="forward-menu-manage" @click="emit('add')"><Plus />{{ t("forwards.newMapping") }}</button>
      <button type="button" class="forward-menu-manage forward-menu-manage--primary" @click="emit('manage')">{{ t("forwards.manageAll") }}<ChevronRight /></button>
    </footer>
  </section>
</template>
