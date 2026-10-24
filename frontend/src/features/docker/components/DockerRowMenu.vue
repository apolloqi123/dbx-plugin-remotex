<script setup lang="ts">
// 容器行的「操作」下拉：触发器（⌄，终端回填成功短暂显示 ✓）+ hover 菜单。
// 开关状态机由面板持有（单个 controller 保证全局只有一个菜单打开），这里
// 只负责触发/内容渲染与派发；选项目标动作的收尾（先收菜单再派发）在面板。
import { Check, ChevronDown, Play, RotateCw, ScrollText, Square, Terminal, Trash2, X } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../../shared/ui/popover/index";
import type { RowMenuController } from "../lib/dockerRowMenu";
import type { DockerActionName } from "../lib/dockerActions";
import type { DockerContainer } from "../lib/dockerPanelModel";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  /** 面板持有的共享菜单 controller（同一时刻只开一行）。 */
  menu: RowMenuController;
  /** 「在终端打开」回填成功后的短暂 ✓ 反馈。 */
  fillRequested: boolean;
  /** 该行有动作在跑。 */
  busy: boolean;
  running: boolean;
  /** 只有 SSH 模式可「在终端打开」。 */
  canOpenTerminal: boolean;
}>();

const emit = defineEmits<{
  action: [action: DockerActionName];
  logs: [];
  terminal: [];
}>();

/** reka 打开弹层默认把焦点迁入首控件；hover 开启的菜单不迁移——用户可能正在
 *  终端输入，焦点被抢走后按键（Space/Enter）会落到菜单项上误触容器操作。
 *  click/键盘/触屏开启的菜单保留默认迁移，方向键与 Esc 才能在菜单内工作。 */
function onOpenAutoFocus(event: Event): void {
  if (props.menu.openedByHover()) event.preventDefault();
}
</script>

<template>
  <Popover :open="props.menu.isOpen(props.container.id)" @update:open="(open: boolean) => { if (!open) props.menu.close(); }">
    <PopoverAnchor as-child>
      <button
        type="button"
        class="icon-button docker-row-trigger"
        :class="{ 'is-active': props.menu.isOpen(props.container.id) }"
        :title="props.t('docker.colActions')"
        :aria-label="props.t('docker.colActions')"
        aria-haspopup="menu"
        :aria-expanded="props.menu.isOpen(props.container.id)"
        @pointerenter="props.menu.hoverTrigger(props.container.id)"
        @pointerleave="props.menu.leaveToClose()"
        @click="props.menu.toggle(props.container.id)"
      >
        <Check v-if="props.fillRequested" />
        <ChevronDown v-else />
      </button>
    </PopoverAnchor>
    <PopoverContent
      class="docker-row-menu w-auto gap-1 p-1"
      align="end"
      :side-offset="4"
      @open-auto-focus="onOpenAutoFocus"
      @pointerenter="props.menu.hoverContent()"
      @pointerleave="props.menu.leaveToClose()"
    >
      <button type="button" class="docker-row-menu-item" :disabled="props.running || props.busy" @click="emit('action', 'start')">
        <Play />{{ props.t("docker.actionStart") }}
      </button>
      <button type="button" class="docker-row-menu-item" :disabled="!props.running || props.busy" @click="emit('action', 'stop')">
        <Square />{{ props.t("docker.actionStop") }}
      </button>
      <button type="button" class="docker-row-menu-item" :disabled="!props.running || props.busy" @click="emit('action', 'restart')">
        <RotateCw />{{ props.t("docker.actionRestart") }}
      </button>
      <button type="button" class="docker-row-menu-item docker-row-menu-item--danger" :disabled="props.busy" @click="emit('action', 'kill')">
        <X />{{ props.t("docker.actionKill") }}
      </button>
      <button type="button" class="docker-row-menu-item docker-row-menu-item--danger" :disabled="props.busy" @click="emit('action', 'rm')">
        <Trash2 />{{ props.t("docker.actionRemove") }}
      </button>
      <div class="docker-row-menu-sep" role="separator" />
      <button type="button" class="docker-row-menu-item" @click="emit('logs')">
        <ScrollText />{{ props.t("docker.actionLogs") }}
      </button>
      <button v-if="props.canOpenTerminal" type="button" class="docker-row-menu-item" @click="emit('terminal')">
        <Terminal />{{ props.t("docker.actionTerminal") }}
      </button>
    </PopoverContent>
  </Popover>
</template>

<style scoped>
/* 行操作悬浮下拉：触发器（⌄，终端回填成功短暂显示 ✓）+ hover 菜单。 */
.docker-row-trigger svg {
  width: 14px;
  height: 14px;
}

.docker-row-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  border: none;
  border-radius: 6px;
  padding: 5px 10px;
  background: none;
  color: inherit;
  font: inherit;
  font-size: 12px;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
}

.docker-row-menu-item svg {
  width: 14px;
  height: 14px;
  flex-basis: 14px;
  opacity: 0.75;
}

.docker-row-menu-item:hover:not(:disabled),
.docker-row-menu-item:focus-visible {
  background: color-mix(in srgb, var(--foreground) 8%, transparent);
  outline: none;
}

.docker-row-menu-item:disabled {
  opacity: 0.45;
  cursor: default;
}

.docker-row-menu-item--danger:hover:not(:disabled),
.docker-row-menu-item--danger:focus-visible {
  background: color-mix(in srgb, var(--danger, #d64545) 16%, transparent);
  color: var(--danger, #d64545);
}

.docker-row-menu-sep {
  height: 1px;
  margin: 3px 6px;
  background: var(--border, rgba(128, 128, 128, 0.25));
}
</style>
