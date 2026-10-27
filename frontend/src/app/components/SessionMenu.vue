<script setup lang="ts">
import { ArrowLeft, ChevronDown, Copy, ListPlus, Loader2, Plus, SquarePlus, Terminal as TerminalIcon, Trash2 } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import type { DockConnectionEntry, LocalShellEntry, SessionMenuActions } from "../lib/workbenchToolbarActions";

/** 工具条「会话」下拉（T19 自 App.vue 模板迁入）：新建 / 复制 / 命令会话 /
 *  本地终端 / Shell 选项 / 导入会话收敛为单入口，Popover 内三视图（主菜单 /
 *  选择 shell / shell 设置）仍是同一弹层的状态切换。会话与 shell 偏好状态、Shell 发现懒加载
 *  逻辑全部留在 App/composables：本组件只渲染并上抛动作，开合态走 v-model 链。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  connectionId: string;
  connected: boolean;
  localUiMode: boolean;
  isRdpConnection: boolean;
  isRdpMode: boolean;
  isVncMode: boolean;
  isSerialMode: boolean;
  isTelnetMode: boolean;
  localShellRestored: boolean;
  isLocalMode: boolean;
  localShells: LocalShellEntry[];
  localShellsLoading: boolean;
  localShellPref: string;
  localShellIntegrationPref: boolean;
  selectedShellInjectable: boolean | undefined;
  localShellCwdPref: string;
  /** 环境变量草稿行：v-model 直接写回父级持有的数组元素（与迁移前同一对象）。 */
  localShellEnvPref: Array<{ name: string; value: string }>;
  localState: string;
  canOpenLocalTab: boolean;
  dockConnections: DockConnectionEntry[];
  actions: SessionMenuActions;
}>();

const sessionMenuOpen = defineModel<boolean>("sessionMenuOpen", { required: true });
const sessionMenuShellOpen = defineModel<boolean>("sessionMenuShellOpen", { required: true });
const sessionMenuShellPickOpen = defineModel<boolean>("sessionMenuShellPickOpen", { required: true });
const localShellSurfaceOpen = defineModel<boolean>("localShellSurfaceOpen", { required: true });

const emit = defineEmits<{
  /** 导入会话弹窗（原 SFTP 工具栏入口，归位会话工具组）。 */
  (e: "open-import"): void;
}>();

const t = props.t;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  toggleSessionMenu,
  openNewSessionTab,
  openCopiedSessionTab,
  openCommandSessionTab,
  toggleLocalTerminal,
  openSessionShellPick,
  openSessionShellPrefs,
  pickLocalShell,
  setLocalShellPref,
  setLocalShellIntegrationPref,
  setLocalShellCwd,
  setLocalShellEnv,
  addLocalShellEnvRow,
  removeLocalShellEnvRow,
  openLocalTerminalTab,
  openLocalShellSurfaceMenu,
  openLocalShellSurface,
  openConnectionSurface,
  restartLocalTerminal,
  requestLocalTerminal,
} = props.actions;
</script>

<template>
  <div>
    <Popover :open="sessionMenuOpen" @update:open="(open) => { if (!open) sessionMenuOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-emerald" :class="{ 'is-active': sessionMenuOpen }" :title="t('sessionMenu.title')" :aria-label="t('sessionMenu.title')" :aria-expanded="sessionMenuOpen" @pointerdown.stop @click.stop="toggleSessionMenu"><WorkbenchGlyph name="sessions" /></button>
      </PopoverAnchor>
      <PopoverContent class="popover session-menu-popover" :class="{ 'session-menu-shell': sessionMenuShellOpen || sessionMenuShellPickOpen }" align="start" :side-offset="5">
        <template v-if="!sessionMenuShellOpen && !sessionMenuShellPickOpen">
          <h3>{{ t("sessionMenu.title") }}</h3>
          <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId" @click="sessionMenuOpen = false; openNewSessionTab();"><SquarePlus />{{ t("newSessionTab") }}</button>
          <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId || !connected" @click="sessionMenuOpen = false; openCopiedSessionTab();"><Copy />{{ t("copySessionTab") }}</button>
          <button v-if="!localUiMode && !isRdpConnection" class="session-menu-item" :disabled="!connectionId || !connected" @click="sessionMenuOpen = false; openCommandSessionTab();"><TerminalIcon />{{ t("spawnSessionTab") }}</button>
          <template v-if="!localUiMode"><hr class="session-menu-separator" /></template>
          <!-- 本地终端：sidecar 所在机器的登录 shell。与 SSH 会话互斥展示，
               已连接时经确认先关 SSH；退出态由终端覆盖层提供重开出口。
               Telnet/VNC/RDP/串口模式下为对应的断开动作。 -->
          <button class="session-menu-item" @click="sessionMenuOpen = false; toggleLocalTerminal();"><TerminalIcon />{{ isRdpMode ? t("rdp.disconnect") : isVncMode ? t("vnc.disconnect") : isSerialMode ? t("serial.disconnect") : isTelnetMode ? t("telnet.disconnect") : localUiMode && !localShellRestored ? t("localTerminal.close") : t("localTerminal.open") }}</button>
          <button v-if="!isTelnetMode && !isSerialMode && !isVncMode && !isRdpMode" class="session-menu-item" @click="openSessionShellPick"><ChevronDown />{{ t("localTerminal.pickShell") }}</button>
          <button v-if="!isTelnetMode && !isSerialMode && !isVncMode && !isRdpMode" class="session-menu-item" @click="openSessionShellPrefs"><ChevronDown />{{ t("localTerminal.settings") }}</button>
          <hr class="session-menu-separator" />
          <button class="session-menu-item" @click="sessionMenuOpen = false; emit('open-import')"><WorkbenchGlyph name="import-sessions" />{{ t("importWizard.title") }}</button>
        </template>
        <template v-else-if="sessionMenuShellPickOpen">
          <header class="session-shell-head">
            <button class="icon-button compact" :title="t('cancel')" :aria-label="t('cancel')" @click="sessionMenuShellPickOpen = false"><ArrowLeft /></button>
            <h3>{{ t("localTerminal.pickShell") }}</h3>
          </header>
          <p class="muted local-shell-hint">{{ t("localTerminal.pickShellHint") }}</p>
          <div v-if="localShellsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <template v-else-if="localShells.length">
            <!-- dock「+」同款列表语义：点击即以该 shell 一次性打开/重启，
                 不改 localShell 默认偏好；首项 = 偏好/自动探测链。 -->
            <button class="shell-surface-item" @click="pickLocalShell()">
              <TerminalIcon class="h-3.5 w-3.5" />{{ t("localTerminal.autoShell") }}
            </button>
            <button v-for="entry in localShells" :key="entry.program" class="shell-surface-item" @click="pickLocalShell(entry.program)">
              <TerminalIcon class="h-3.5 w-3.5" />{{ entry.name }}<span class="mono local-shell-program">{{ entry.program }}</span>
              <span v-if="entry.isDefault" class="local-shell-badge">{{ t("localTerminal.defaultBadge") }}</span>
              <span v-if="entry.isUserShell" class="local-shell-badge">{{ t("localTerminal.userShellBadge") }}</span>
            </button>
          </template>
          <p v-else class="muted local-shell-hint">{{ t("localTerminal.shellsUnavailable") }}</p>
          <footer class="local-shell-footer">
            <button class="primary-button" :disabled="localState === 'starting'" @click="sessionMenuOpen = false; sessionMenuShellPickOpen = false; pickLocalShell()">
              {{ localUiMode ? t("localTerminal.restart") : t("localTerminal.open") }}
            </button>
          </footer>
        </template>
        <template v-else>
          <header class="session-shell-head">
            <button class="icon-button compact" :title="t('cancel')" :aria-label="t('cancel')" @click="sessionMenuShellOpen = false"><ArrowLeft /></button>
            <h3>{{ t("localTerminal.settings") }}</h3>
          </header>
          <p class="muted local-shell-hint">{{ t("localTerminal.settingsHint") }}</p>
          <div v-if="localShellsLoading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <template v-else-if="localShells.length">
            <label v-for="entry in localShells" :key="entry.program" class="agent-mode-option">
              <input type="radio" name="local-shell" :checked="localShellPref ? localShellPref === entry.program : entry.isDefault" @change="setLocalShellPref(entry.program)" />
              <span class="local-shell-row">
                <strong>{{ entry.name }}</strong>
                <span class="mono local-shell-program">{{ entry.program }}</span>
                <span v-if="entry.isDefault" class="local-shell-badge">{{ t("localTerminal.defaultBadge") }}</span>
                <span v-if="entry.isUserShell" class="local-shell-badge">{{ t("localTerminal.userShellBadge") }}</span>
              </span>
            </label>
          </template>
          <p v-else class="muted local-shell-hint">{{ t("localTerminal.shellsUnavailable") }}</p>
          <label class="agent-mode-option" :title="selectedShellInjectable === false ? t('localTerminal.injectionUnavailable') : ''">
            <input type="checkbox" name="local-shell-integration" :checked="localShellIntegrationPref" :disabled="selectedShellInjectable === false" @change="setLocalShellIntegrationPref(($event.target as HTMLInputElement).checked)" />
            <span>{{ t("localTerminal.injection") }}</span>
          </label>
          <!-- 启动目录 + 环境变量（localShellCwd/localShellEnv 偏好）：
               change 时落库，草稿行（未填名）只留在 UI 不持久化。 -->
          <div class="local-shell-field">
            <label class="local-shell-field-label" for="local-shell-cwd">{{ t("localTerminal.cwdLabel") }}</label>
            <input id="local-shell-cwd" :value="localShellCwdPref" class="mono local-shell-input" :placeholder="t('localTerminal.cwdPlaceholder')" spellcheck="false" @change="setLocalShellCwd(($event.target as HTMLInputElement).value)" />
            <p class="muted local-shell-hint">{{ t("localTerminal.cwdHint") }}</p>
          </div>
          <div class="local-shell-field">
            <div class="local-shell-env-head">
              <span class="local-shell-field-label">{{ t("localTerminal.envLabel") }}</span>
              <button class="icon-button compact" :title="t('localTerminal.envAdd')" :aria-label="t('localTerminal.envAdd')" @click="addLocalShellEnvRow"><Plus /></button>
            </div>
            <div v-for="(_, index) in localShellEnvPref" :key="index" class="local-shell-env-row">
              <input v-model="localShellEnvPref[index].name" class="mono local-shell-input" :placeholder="t('localTerminal.envName')" :aria-label="t('localTerminal.envName')" spellcheck="false" @change="setLocalShellEnv()" />
              <input v-model="localShellEnvPref[index].value" class="mono local-shell-input" :placeholder="t('localTerminal.envValue')" :aria-label="t('localTerminal.envValue')" spellcheck="false" @change="setLocalShellEnv()" />
              <button class="icon-button compact" :title="t('localTerminal.envRemove')" :aria-label="t('localTerminal.envRemove')" @click="removeLocalShellEnvRow(index)"><Trash2 /></button>
            </div>
            <p class="muted local-shell-hint">{{ t("localTerminal.envHint") }}</p>
          </div>
          <footer class="local-shell-footer">
            <!-- 本地模式中按钮保持可用：restart 语义（关当前 → 按新偏好重开）。
                 仅 starting 期间禁用防双击。 -->
            <button
              v-if="canOpenLocalTab"
              class="local-tab-button"
              :title="t('localTerminal.openInNewTab')" :aria-label="t('localTerminal.openInNewTab')"
              @click="openLocalTerminalTab"
            ><SquarePlus /></button>
            <Popover :open="localShellSurfaceOpen" @update:open="(open) => (localShellSurfaceOpen = open)">
              <PopoverAnchor as-child>
                <button
                  v-if="canOpenLocalTab"
                  class="local-tab-button"
                  :title="t('localTerminal.openShellSurface')" :aria-label="t('localTerminal.openShellSurface')"
                  @pointerdown.stop @click.stop="openLocalShellSurfaceMenu"
                ><ListPlus /></button>
              </PopoverAnchor>
              <PopoverContent class="popover" align="end" :side-offset="5">
                <button class="shell-surface-item" @click="openLocalShellSurface()">
                  <TerminalIcon class="h-3.5 w-3.5" />{{ t("localTerminal.autoShell") }}
                </button>
                <!-- 已配置默认 Shell 时隐藏发现列表（与 dock「+」launch-options
                     同规则）：autoShell 项已按偏好打开，无需再选。 -->
                <template v-if="!localShellPref">
                  <button v-for="entry in localShells" :key="entry.program" class="shell-surface-item" @click="openLocalShellSurface(entry.program)">
                    <TerminalIcon class="h-3.5 w-3.5" />{{ entry.name }}<span class="mono local-shell-program">{{ entry.program }}</span>
                  </button>
                </template>
                <template v-if="dockConnections.length">
                  <p class="shell-surface-header">{{ t("localTerminal.connectionTerminals") }}</p>
                  <button v-for="connection in dockConnections" :key="`conn-${connection.id}`" class="shell-surface-item" @click="openConnectionSurface(connection)">
                    <TerminalIcon class="h-3.5 w-3.5" />{{ connection.name }}
                  </button>
                </template>
              </PopoverContent>
            </Popover>
            <button class="primary-button" :disabled="localState === 'starting'" @click="sessionMenuOpen = false; sessionMenuShellOpen = false; localShellRestored || isLocalMode ? restartLocalTerminal() : requestLocalTerminal()">
              {{ localUiMode ? t("localTerminal.restart") : t("localTerminal.open") }}
            </button>
          </footer>
        </template>
      </PopoverContent>
    </Popover>
  </div>
</template>

<style scoped>
/* shell 设置视图里的单选/复选行（原 App.vue scoped 样式随 DOM 迁入；
   批量栏等其它宿主各自持有同名规则）。 */
.agent-mode-option { display: flex; align-items: center; gap: 8px; padding: 4px 0; font-size: 12px; cursor: pointer; }
</style>
