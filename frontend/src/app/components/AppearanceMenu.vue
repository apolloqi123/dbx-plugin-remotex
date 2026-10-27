<script setup lang="ts">
import { Settings } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import type { AppearanceMenuActions, PanelSchemeChoice } from "../lib/workbenchToolbarActions";

/** dock 面板底部栏「配色快切」下拉（T19 自 App.vue 模板迁入）：列出与当前宿主
 *  明暗档同调性的方案，单击写入槽位并启用 custom 来源；「跟随宿主」恢复 host。
 *  配色写入本体与开合态仍由 App 的 useWorkbenchToolbar 持有。
 *  行容器/色卡样式在全局 toolbar.css（.panel-palette-*），本组件不需要 scoped CSS。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  isRdpMode: boolean;
  panelSchemeChoices: PanelSchemeChoice[];
  panelActiveSchemeId: string | null;
  actions: AppearanceMenuActions;
}>();

const panelPaletteOpen = defineModel<boolean>("panelPaletteOpen", { required: true });

const t = props.t;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const { togglePanelPaletteMenu, pickPanelScheme, openSettings } = props.actions;
</script>

<template>
  <Popover v-if="!isRdpMode" :open="panelPaletteOpen" @update:open="(open) => { if (!open) panelPaletteOpen = false; }">
    <PopoverAnchor as-child>
      <button class="icon-button compact icon-violet" :class="{ 'is-active': panelPaletteOpen }" :title="t('terminalAppearance.schemeSection')" :aria-label="t('terminalAppearance.schemeSection')" :aria-expanded="panelPaletteOpen" @pointerdown.stop @click.stop="togglePanelPaletteMenu"><WorkbenchGlyph name="palette" /></button>
    </PopoverAnchor>
    <PopoverContent class="popover panel-palette-popover" align="end" :side-offset="4">
      <h3>{{ t("terminalAppearance.schemeSection") }}</h3>
      <ul class="panel-palette-list" role="listbox">
        <li>
          <button type="button" class="panel-palette-row" :class="{ selected: panelActiveSchemeId === null }" role="option" :aria-selected="panelActiveSchemeId === null" @click="pickPanelScheme(null)">
            <span class="panel-palette-name">{{ t("terminalAppearance.modeFollowHost") }}</span>
          </button>
        </li>
        <li v-for="scheme in panelSchemeChoices" :key="scheme.id">
          <button type="button" class="panel-palette-row" :class="{ selected: panelActiveSchemeId === scheme.id }" role="option" :aria-selected="panelActiveSchemeId === scheme.id" :title="scheme.name" :aria-label="scheme.name" @click="pickPanelScheme(scheme.id)">
            <span class="panel-palette-name">{{ scheme.name }}</span>
            <span class="panel-palette-swatch" aria-hidden="true"><i v-for="(color, index) in scheme.colors.slice(0, 6)" :key="index" :style="{ background: color }" /></span>
          </button>
        </li>
      </ul>
      <footer class="panel-palette-footer">
        <button type="button" class="link-button" @click="panelPaletteOpen = false; openSettings()"><Settings />{{ t("terminalAppearance.moreSettings") }}</button>
      </footer>
    </PopoverContent>
  </Popover>
</template>
