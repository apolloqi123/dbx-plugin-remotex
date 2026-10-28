<script setup lang="ts">
// 端口映射管理面板：摘要 + 统一隧道卡片 + 新增表单。状态全部来自
// usePortForwards（工具栏快捷弹层与完整管理器共享同一实例）。
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from "vue";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Bookmark,
  Check,
  ChevronUp,
  CircleStop,
  Copy,
  Loader2,
  Play,
  Plus,
  Trash2,
  Waypoints,
} from "@lucide/vue";
import { workbenchMessage } from "../../../shared/lib/i18n";
import {
  findForwardConflict,
  formatForwardBytes,
  formatForwardRoute,
  listenHostOptions,
  validateForwardForm,
  type ForwardCard,
  type ForwardFormDraft,
  type ForwardFormError,
  type ForwardKind,
} from "../lib/portForward";
import type { PortForwardsController } from "../composables/usePortForwards";
import type { TunnelProfile } from "../lib/tunnelProfiles";

type RowCard = ForwardCard<TunnelProfile>;

const props = defineProps<{
  controller: PortForwardsController;
  locale: string;
  independent: boolean;
  sessionId: string | null;
  standalone?: boolean;
  focusForm?: boolean;
}>();

const t = (key: string, values: Record<string, string | number> = {}) => workbenchMessage(props.locale, key, values);

const { forwards, interfaces, loading, busyId, startingAll, submitting, error, message, startedCount, summary, cards, inactiveProfiles } = props.controller;
const KINDS: { kind: ForwardKind; icon: typeof ArrowDownToLine; tone: string; labelKey: string; hintKey: string }[] = [
  { kind: "local", icon: ArrowDownToLine, tone: "ui-tone-blue", labelKey: "forwards.local", hintKey: "forwards.localHint" },
  { kind: "remote", icon: ArrowUpFromLine, tone: "ui-tone-amber", labelKey: "forwards.remote", hintKey: "forwards.remoteHint" },
  { kind: "dynamic", icon: Waypoints, tone: "ui-tone-emerald", labelKey: "forwards.dynamic", hintKey: "forwards.dynamicHint" },
];

function kindMeta(kind: ForwardKind) {
  return KINDS.find((entry) => entry.kind === kind) ?? KINDS[0];
}

const draft = reactive<ForwardFormDraft>({
  kind: "local",
  listenHost: "127.0.0.1",
  listenPort: "",
  targetHost: "",
  targetPort: "",
});
const formOpen = ref(false);
const savePreset = ref(false);
const formError = ref("");
const listenHostInput = ref<HTMLInputElement | null>(null);
const copiedKey = ref("");
let copiedTimer: ReturnType<typeof setTimeout> | undefined;

const hostOptionListId = "forward-listen-host-options";
const hostOptions = computed(() => listenHostOptions(draft.kind));
const probedInterfaceOptions = computed(() => {
  if (draft.kind === "remote") return [];
  const known = new Set(hostOptions.value.map((option) => option.value));
  return interfaces.value.filter((iface) => !known.has(iface.addr));
});
const formDisabled = computed(() => (!props.independent && !props.sessionId) || submitting.value);
const preview = computed(() => {
  const listen = `${draft.listenHost.trim() || "127.0.0.1"}:${draft.listenPort.trim() || "?"}`;
  if (draft.kind === "dynamic") return t("forwards.previewDynamic", { address: listen });
  const target = `${draft.targetHost.trim() || "?"}:${draft.targetPort.trim() || "?"}`;
  return t(draft.kind === "remote" ? "forwards.previewRemote" : "forwards.previewLocal", { address: listen, target });
});
const empty = computed(() => !loading.value && !cards.value.length);
const canStartAll = computed(() => inactiveProfiles.value.length > 0 && !startingAll.value && busyId.value === null);
const canStopAll = computed(() => summary.value.running > 0 && busyId.value === null);
const successMessage = computed(() => (startedCount.value !== null && !message.value
  ? t("forwards.profilesStarted", { count: startedCount.value })
  : ""));

function resetFormError() {
  formError.value = "";
}

function selectKind(kind: ForwardKind) {
  draft.kind = kind;
  resetFormError();
}

function openForm() {
  formOpen.value = true;
  resetFormError();
  void nextTick(() => listenHostInput.value?.focus());
}

function toggleForm() {
  if (formOpen.value) formOpen.value = false;
  else openForm();
}

function applyFormError(code: ForwardFormError) {
  formError.value = code ? t(`forwards.error.${code}`) : "";
}

async function submit() {
  const invalid = validateForwardForm(draft);
  if (invalid) {
    applyFormError(invalid);
    return;
  }
  const conflict = findForwardConflict(forwards.value, draft);
  if (conflict) {
    formError.value = t("forwards.error.conflict", {
      route: `${draft.listenHost.trim() || "127.0.0.1"}:${draft.listenPort.trim()}`,
      existing: formatForwardRoute(conflict),
    });
    return;
  }
  if (formDisabled.value) return;
  submitting.value = true;
  formError.value = "";
  try {
    const started = await props.controller.start(draft);
    if (started) {
      if (savePreset.value) props.controller.saveProfile(draft);
      formOpen.value = false;
    } else {
      formError.value = props.controller.error.value;
    }
  } finally {
    submitting.value = false;
  }
}

async function copyCard(card: RowCard) {
  try {
    await props.controller.copyAddress({
      listenHost: card.listenHost,
      listenPort: card.listenPort,
      boundPort: card.forward?.boundPort ?? 0,
    });
    copiedKey.value = card.key;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => { copiedKey.value = ""; }, 1800);
  } catch (cause) {
    formError.value = String(cause);
  }
}

function stop(card: RowCard) {
  if (card.forward) void props.controller.stop(card.forward.id);
}

function start(card: RowCard) {
  if (card.profile) void props.controller.startSaved([card.profile]);
}

function remove(card: RowCard) {
  if (card.profile) props.controller.removeProfile(card.profile.id);
}

function stateClass(state: ForwardCard["state"]) {
  return state === "saved" ? "is-saved" : `is-${state}`;
}

function statsLine(card: ForwardCard) {
  if (card.state === "saved") return "";
  return t("forwards.cardStats", {
    active: card.connectionsActive,
    total: card.connectionsTotal,
    up: formatForwardBytes(card.bytesUp),
    down: formatForwardBytes(card.bytesDown),
  });
}

watch(
  () => props.focusForm,
  (focus) => { if (focus) openForm(); },
  { immediate: true },
);

// 列表/启停失败落在控制器 error 上：表单展开时也要把它带到可见位置。
watch(error, (value) => {
  if (value && !formError.value) formError.value = value;
});

// 首次加载完成后没有任何隧道时展开表单；有数据时保持收起，让列表占主位。
let autoFormOpened = false;
watch([loading, cards], ([isLoading, list]) => {
  if (autoFormOpened || isLoading || list.length) return;
  autoFormOpened = true;
  formOpen.value = true;
});

onBeforeUnmount(() => {
  if (copiedTimer) clearTimeout(copiedTimer);
});
</script>

<template>
  <div class="forwards-body">
    <section class="forward-summary" aria-live="polite">
      <span class="forward-summary-stat forward-summary-stat--running"><span class="forward-dot" />{{ t("forwards.runningCount", { count: summary.running }) }}</span>
      <span class="forward-summary-stat">{{ t("forwards.connectionsCount", { count: summary.connectionsActive }) }}</span>
      <span class="forward-summary-stat forward-summary-stat--traffic">↑{{ formatForwardBytes(summary.bytesUp) }} ↓{{ formatForwardBytes(summary.bytesDown) }}</span>
      <span v-if="summary.failed" class="forward-summary-stat forward-summary-stat--failed">{{ t("forwards.failedCount", { count: summary.failed }) }}</span>
      <span class="forward-summary-actions">
        <button v-if="canStartAll" type="button" class="forward-action-button" :disabled="startingAll" @click="props.controller.startSaved()">
          <Loader2 v-if="startingAll" class="spinning" /><Play v-else />{{ t("forwards.startAllPresets") }}
        </button>
        <button v-if="canStopAll" type="button" class="forward-action-button forward-action-button--danger" :disabled="busyId !== null" @click="props.controller.stopAll()">
          <CircleStop />{{ t("forwards.stopAll") }}
        </button>
        <button type="button" class="forward-action-button forward-action-button--primary" :aria-expanded="formOpen" @click="toggleForm">
          <component :is="formOpen ? ChevronUp : Plus" />{{ t(formOpen ? "forwards.hideForm" : "forwards.newMapping") }}
        </button>
      </span>
    </section>

    <p v-if="error && !formOpen" class="forward-form-error forward-status" role="alert">{{ error }}</p>

    <div v-if="loading && !cards.length" class="empty"><Loader2 class="spinning" />{{ t("loading") }}</div>

    <div v-else-if="empty" class="forward-empty">
      <span class="forward-empty-icon"><component :is="ArrowDownToLine" /></span>
      <strong>{{ t("forwards.emptyTitle") }}</strong>
      <p>{{ t("forwards.empty") }}</p>
      <button type="button" class="primary-button" @click="openForm"><Plus />{{ t("forwards.newMapping") }}</button>
    </div>

    <ul v-else class="forwards-list forward-cards">
      <li v-for="card in cards" :key="card.key" class="forward-card" :class="stateClass(card.state)">
        <span class="forward-card-icon" :class="kindMeta(card.kind).tone">
          <component :is="kindMeta(card.kind).icon" />
        </span>
        <div class="forward-card-main">
          <div class="forward-card-route">
            <code>{{ formatForwardRoute({ ...card, boundPort: card.forward?.boundPort ?? 0 }) }}</code>
            <span class="forward-card-state">{{ card.state === "saved" ? t("forwards.presetBadge") : t(`forwards.state.${card.state}`) }}</span>
          </div>          <div class="forward-card-meta">
            <span class="forward-card-kind">{{ t(kindMeta(card.kind).labelKey) }}</span>
            <span v-if="card.profile && card.forward" class="forward-card-preset"><Bookmark />{{ t("forwards.presetBadge") }}</span>
            <span v-if="statsLine(card)" class="forward-card-stats">{{ statsLine(card) }}</span>
            <span v-if="card.error" class="forward-card-error">{{ card.error }}</span>
          </div>
        </div>
        <div class="forward-card-actions">
          <button type="button" class="forward-icon-button" :class="{ 'is-copied': copiedKey === card.key }" :title="t('forwards.copyAddress')" :aria-label="t('forwards.copyAddress')" @click="copyCard(card)">
            <Check v-if="copiedKey === card.key" /><Copy v-else />
          </button>
          <button v-if="card.forward" type="button" class="forward-icon-button forward-icon-button--danger" :title="t('forwards.stop')" :aria-label="t('forwards.stop')" :disabled="busyId !== null" @click="stop(card)">
            <Loader2 v-if="busyId === card.forward.id" class="spinning" /><CircleStop v-else />
          </button>
          <button v-else type="button" class="forward-icon-button forward-icon-button--start" :title="t('forwards.startSaved')" :aria-label="t('forwards.startSaved')" :disabled="startingAll || busyId !== null" @click="start(card)">
            <Play />
          </button>
          <button v-if="card.profile" type="button" class="forward-icon-button forward-icon-button--muted" :title="t('forwards.deleteSaved')" :aria-label="t('forwards.deleteSaved')" @click="remove(card)">
            <Trash2 />
          </button>
        </div>
      </li>
    </ul>

    <p v-if="message" class="forward-form-error forward-status" role="alert">{{ message }}</p>
    <p v-else-if="successMessage" class="forward-status" role="status">{{ successMessage }}</p>

    <form v-if="formOpen" class="forward-form forward-form-card" @submit.prevent="submit">
      <div class="forward-directions" role="radiogroup" :aria-label="t('forwards.kindLabel')">
        <button
          v-for="entry in KINDS"
          :key="entry.kind"
          type="button"
          class="forward-direction"
          :class="[entry.tone, { 'is-active': draft.kind === entry.kind }]"
          role="radio"
          :aria-checked="draft.kind === entry.kind"
          @click="selectKind(entry.kind)"
        >
          <span class="forward-direction-icon"><component :is="entry.icon" /></span>
          <span class="forward-direction-copy">
            <strong>{{ t(entry.labelKey) }}</strong>
            <small>{{ t(entry.hintKey) }}</small>
          </span>
        </button>
      </div>
      <fieldset class="forward-form-fields" :disabled="formDisabled" style="border:0;margin:0;padding:0;min-inline-size:0">
        <label class="forward-field">
          <span>{{ t("forwards.listen") }}</span>
          <span class="forward-field-pair">
            <input
              ref="listenHostInput"
              v-model="draft.listenHost"
              :placeholder="t('forwards.listenHostPlaceholder')"
              :list="hostOptionListId"
              @input="resetFormError"
            />
            <input v-model="draft.listenPort" inputmode="numeric" :placeholder="t('forwards.portPlaceholder')" @input="resetFormError" />
          </span>
        </label>
        <datalist :id="hostOptionListId">
          <option v-for="option in hostOptions" :key="option.value" :value="option.value">{{ t(option.labelKey) }}</option>
          <option v-for="iface in probedInterfaceOptions" :key="iface.addr" :value="iface.addr">
            {{ iface.isLoopback ? t("forwards.loopback") : iface.name }}
          </option>
        </datalist>
        <label v-if="draft.kind !== 'dynamic'" class="forward-field">
          <span>{{ t("forwards.target") }}</span>
          <span class="forward-field-pair">
            <input v-model="draft.targetHost" :placeholder="t('forwards.targetHostPlaceholder')" @input="resetFormError" />
            <input v-model="draft.targetPort" inputmode="numeric" :placeholder="t('forwards.portPlaceholder')" @input="resetFormError" />
          </span>
        </label>
      </fieldset>
      <p class="forward-preview" :class="`forward-preview--${draft.kind}`">{{ preview }}</p>
      <p v-if="draft.kind === 'remote'" class="forward-form-hint">{{ t("forwards.remoteListenTip") }}</p>
      <p v-if="!props.independent && !props.sessionId" class="forward-form-hint">{{ t("forwards.error.noSession") }}</p>
      <p v-if="formError" class="forward-form-error">{{ formError }}</p>
      <footer>
        <label class="forward-save-preset">
          <input v-model="savePreset" type="checkbox" />{{ t("forwards.saveAsPreset") }}
        </label>
        <button type="submit" class="primary-button" :disabled="formDisabled">
          <Loader2 v-if="submitting" class="spinning" /><Plus v-else />{{ t("forwards.addAndStart") }}
        </button>
      </footer>
    </form>
  </div>
</template>
