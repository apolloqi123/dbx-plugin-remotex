import { computed, ref } from "vue";
import {
  DOCKER_SPLIT_RATIO_DEFAULT,
  paneDirectionClass,
  paneSplitCssVar,
  resolvePaneDirection,
  terminalFlexBasis,
  type PaneDirectionPref,
} from "../../../shared/lib/paneLayout";
import type { SshWorkbenchPaneOrder } from "../../lib/workbenchLayout";

/**
 * 布局域状态（Task 21b）：分栏比例/顺序/轴向与右栏让宽计算。
 *
 * 纯本地工厂：`sftpPaneOpen` / `paneDirection` 由组合上下文注入（SFTP 域与视图
 * 偏好持有），拖拽/键盘微调等行为仍在 useSplitDivider。容器分屏比例独立于
 * 右侧栏：`dockerPaneBasis` 供容器页内联样式消费，不参与左右/上下分栏。
 */
export function useLayoutState(options: {
  paneDirection: { readonly value: PaneDirectionPref };
  sftpPaneOpen: { readonly value: boolean };
}) {
  // splitRatio 是终端占比：默认终端 75% / SFTP 面板 25%。
  const splitRatio = ref(75);
  // 容器底部分屏的独立比例：默认终端 60% / 容器 40%（4:6，容器占 4），每次打开容器页重置。
  const dockerSplitRatio = ref(DOCKER_SPLIT_RATIO_DEFAULT);
  const paneOrder = ref<SshWorkbenchPaneOrder>("terminal-left");
  const sudoMode = ref(false);
  const paneContainer = ref<HTMLElement>();
  // 工作台外层列（终端+SFTP 上区 + 容器全宽底部行）；容器分屏把手以它为坐标系。
  const workbenchSplitRef = ref<HTMLElement>();

  // 工作台分栏只由用户轴向偏好的右栏（SFTP/指标）决定；容器分屏在终端列
  // 内部独立堆叠，不影响左右/上下比例。
  const paneLayout = computed(() => ({
    direction: resolvePaneDirection({
      pref: options.paneDirection.value,
      viewportWidth: typeof window !== "undefined" ? window.innerWidth : undefined,
    }),
    ratio: splitRatio.value,
  }));
  // 终端让宽（纯计算在 lib/paneLayout）：右侧栏按当前生效比例让出份额。
  const terminalBasis = computed(() => ({
    flexBasis: terminalFlexBasis({ sftpOpen: options.sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }),
  }));
  // 纵向分栏（issue #207）：「始终上下」/ 窄屏断点的两栏高度改由容器上的
  // --pane-split 变量消费当前生效比例，CSS 不再钉死。
  // solo（右侧栏收起）不设变量，回落既有 solo 全屏规则。
  const paneSplitStyle = computed(() => paneSplitCssVar({ sftpOpen: options.sftpPaneOpen.value, splitRatio: paneLayout.value.ratio }));
  // 容器页高度占比 = 100 - 终端占比；内联 flex-basis 落在容器页上。
  const dockerPaneBasis = computed(() => ({ flexBasis: `${100 - dockerSplitRatio.value}%` }));
  const orderedPaneClass = computed(() => [
    paneOrder.value === "sftp-left" ? "panes panes--reversed" : "panes",
    paneDirectionClass(options.paneDirection.value),
    options.sftpPaneOpen.value ? "" : "panes--solo",
  ].filter(Boolean).join(" "));

  /** 打开容器页时回到默认 60%（关闭不重置，下次打开再回默认）。 */
  function resetDockerSplitRatio() {
    dockerSplitRatio.value = DOCKER_SPLIT_RATIO_DEFAULT;
  }

  return {
    splitRatio,
    dockerSplitRatio,
    paneOrder,
    sudoMode,
    paneContainer,
    workbenchSplitRef,
    paneLayout,
    terminalBasis,
    paneSplitStyle,
    dockerPaneBasis,
    orderedPaneClass,
    resetDockerSplitRatio,
  };
}

export type LayoutState = ReturnType<typeof useLayoutState>;
