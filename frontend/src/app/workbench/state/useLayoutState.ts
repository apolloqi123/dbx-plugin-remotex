import { computed, ref } from "vue";
import {
  DOCKER_SPLIT_RATIO_DEFAULT,
  paneDirectionClass,
  paneSplitCssVar,
  resolveWorkbenchPaneLayout,
  terminalFlexBasis,
  type PaneDirectionPref,
} from "../../../shared/lib/paneLayout";
import type { SshWorkbenchPaneOrder } from "../../lib/workbenchLayout";

/**
 * 布局域状态（Task 21b）：分栏比例/顺序/轴向与右侧栏让宽计算。
 *
 * 纯本地工厂：`sftpPaneOpen` / `dockerPanelOpen` / `paneDirection` 由组合上下文
 * 注入（SFTP 域与视图偏好持有），拖拽/键盘微调等行为仍在 useSplitDivider。
 */
export function useLayoutState(options: {
  paneDirection: { readonly value: PaneDirectionPref };
  sftpPaneOpen: { readonly value: boolean };
  dockerPanelOpen: { readonly value: boolean };
}) {
  const splitRatio = ref(58);
  // Docker 底部分屏的独立比例：默认终端 67% / 容器 33%，每次打开 Docker 重置。
  const dockerSplitRatio = ref(DOCKER_SPLIT_RATIO_DEFAULT);
  const paneOrder = ref<SshWorkbenchPaneOrder>("terminal-left");
  const sudoMode = ref(false);
  const paneContainer = ref<HTMLElement>();

  // 工作台分栏：Docker 固定为终端在上、容器在下（默认终端 67% / 容器 33%），
  // 不受左右/上下偏好与换位影响；SFTP/指标沿用用户轴向与 splitRatio。
  const paneLayout = computed(() => resolveWorkbenchPaneLayout({
    dockerOpen: options.dockerPanelOpen.value,
    pref: options.paneDirection.value,
    viewportWidth: typeof window !== "undefined" ? window.innerWidth : undefined,
    splitRatio: splitRatio.value,
    dockerSplitRatio: dockerSplitRatio.value,
  }));
  // 终端让宽（纯计算在 lib/paneLayout）：右侧栏按当前生效比例让出份额。
  const terminalBasis = computed(() => ({
    flexBasis: terminalFlexBasis({ sftpOpen: options.sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }),
  }));
  // 纵向分栏（issue #207）：「始终上下」/ 窄屏断点/ Docker 底栏的两栏高度改由
  // 容器上的 --pane-split 变量消费当前生效比例，CSS 不再钉死。
  // solo（右侧栏收起）不设变量，回落既有 solo 全屏规则。
  const paneSplitStyle = computed(() => paneSplitCssVar({ sftpOpen: options.sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }));
  const orderedPaneClass = computed(() => [
    paneOrder.value === "sftp-left" ? "panes panes--reversed" : "panes",
    paneDirectionClass(options.paneDirection.value),
    options.dockerPanelOpen.value ? "panes--docker-split" : "",
    options.sftpPaneOpen.value ? "" : "panes--solo",
  ].filter(Boolean).join(" "));

  return {
    splitRatio,
    dockerSplitRatio,
    paneOrder,
    sudoMode,
    paneContainer,
    paneLayout,
    terminalBasis,
    paneSplitStyle,
    orderedPaneClass,
  };
}

export type LayoutState = ReturnType<typeof useLayoutState>;
