<script setup lang="ts">
// 工具栏「命令管理」弹层：直接复用设置页的命令管理器（树 / 右键菜单 / 执行 /
// 编辑 / 导入导出），工具栏入口与设置页保持同一套交互，不再维护第二套列表 UI。
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import QuickCommandsSection from "../../features/quick-commands/components/QuickCommandsSection.vue";
import {
  QUICK_COMMANDS_LIMIT,
  type QuickCommand,
  type QuickCommandGroup,
} from "../../features/quick-commands/lib/quickCommands";
import type { QuickCommandsMenuActions } from "../lib/workbenchToolbarActions";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  connected: boolean;
  quickCommands: QuickCommand[];
  quickGroups: QuickCommandGroup[];
  quickSaving: boolean;
  quickImporting: boolean;
  actions: QuickCommandsMenuActions;
}>();

const quickMenuOpen = defineModel<boolean>("quickMenuOpen", { required: true });

const {
  toggleQuickMenu,
  sendQuickCommand,
  pasteQuickCommand,
  saveQuickCommand,
  deleteQuickCommand,
  importQuickCommands,
  createQuickCommandGroup,
  renameQuickCommandGroupById,
  dissolveQuickCommandGroupById,
  showNotice,
  showError,
} = props.actions;
</script>

<template>
  <div>
    <Popover :open="quickMenuOpen" @update:open="(open) => { if (!open) quickMenuOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-amber" :title="t('quickCommandsManagerTitle')" :aria-label="t('quickCommandsManagerTitle')" :disabled="!connected" @pointerdown.stop @click.stop="toggleQuickMenu"><WorkbenchGlyph name="quick-commands" /></button>
      </PopoverAnchor>
      <PopoverContent class="popover quick-commands-popover quick-commands-manager-popover" align="end" :side-offset="5">
        <QuickCommandsSection
          v-if="quickMenuOpen"
          variant="popover"
          :commands="quickCommands"
          :groups="quickGroups"
          :connected="connected"
          :saving="quickSaving"
          :importing="quickImporting"
          :limit="QUICK_COMMANDS_LIMIT"
          :t="t"
          @save="saveQuickCommand"
          @delete="deleteQuickCommand"
          @import="importQuickCommands"
          @create-group="createQuickCommandGroup"
          @rename-group="(payload) => renameQuickCommandGroupById(payload.groupId, payload.name)"
          @dissolve-group="dissolveQuickCommandGroupById"
          @run="sendQuickCommand"
          @paste="pasteQuickCommand"
          @notice="showNotice"
          @error="showError"
        />
      </PopoverContent>
    </Popover>
  </div>
</template>
