<script setup lang="ts">
// 端口映射管理器外壳：弹窗/独立页两种形态，内容统一由 PortForwardsPanel 渲染。
// 状态控制器可由工作台注入（与工具栏快捷弹层共享），独立页则自建一份。
import { watch } from "vue";
import { X } from "@lucide/vue";
import { workbenchMessage } from "../../../shared/lib/i18n";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { usePortForwards, type PortForwardsController } from "../composables/usePortForwards";
import PortForwardsPanel from "./PortForwardsPanel.vue";

interface Props {
  locale: string;
  open: boolean;
  /** 转发跟随的连接；面板按它拉取 `ssh/forward/list`。 */
  connectionId: string;
  /** 新建映射要挂到的会话；未连接（null）时添加表单禁用。 */
  sessionId: string | null;
  /** Independent manager starts mappings using connectionId and needs no PTY. */
  independent?: boolean;
  standalone?: boolean;
  quickStart?: boolean;
  /** 打开后直接展开新增表单（工具栏「新增映射」入口）。 */
  focusForm?: boolean;
  /** 工作台共享控制器；缺省由本组件自建并自行处理错误。 */
  controller?: PortForwardsController;
}
const props = defineProps<Props>();
const emit = defineEmits<{ "update:open": [boolean]; error: [unknown] }>();

const t = (key: string, values: Record<string, string | number> = {}) => workbenchMessage(props.locale, key, values);

const controller: PortForwardsController = props.controller ?? usePortForwards({
  connectionId: () => props.connectionId,
  independent: () => !!props.independent,
  sessionId: () => props.sessionId,
  onError: (cause) => emit("error", cause),
});

let quickStartHandled = false;
watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    await controller.open();
    if (props.quickStart && !quickStartHandled) {
      quickStartHandled = true;
      await controller.startSaved();
    }
  },
  { immediate: true },
);
</script>

<template>
  <component :is="props.standalone ? 'div' : Dialog" :open="props.open" @update:open="(open: boolean) => emit('update:open', open)">
    <component
      :is="props.standalone ? 'section' : DialogContent"
      class="modal forwards-modal"
      :class="{ 'forwards-modal--standalone': props.standalone, 'forwards-modal--manager': props.independent }"
      @escape-key-down.prevent
    >
      <header>
        <span class="forward-modal-heading">
          <component :is="props.standalone ? 'h1' : DialogTitle">{{ t("forwards.title") }}</component>
          <span class="forward-modal-subtitle">{{ t("forwards.subtitle") }}</span>
        </span>
        <button v-if="!props.standalone" :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:open', false)"><X /></button>
      </header>
      <PortForwardsPanel
        :controller="controller"
        :locale="props.locale"
        :independent="!!props.independent"
        :session-id="props.sessionId"
        :standalone="props.standalone"
        :focus-form="props.focusForm"
      />
    </component>
  </component>
</template>
