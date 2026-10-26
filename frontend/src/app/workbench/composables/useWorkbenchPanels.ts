import { nextTick, type Ref } from "vue";
import type { SshWorkbenchPaneOrder } from "../../lib/workbenchLayout";

type RightPanelView = "sftp" | "metrics";

/**
 * 工作台右侧栏两视图（SFTP/指标）与终端列底部容器分屏的开关语义：
 * 视图本体由各自 composable/组件承载，这里只做互斥、可用性门与
 * 布局偏好落盘（persistState）/重排 fit 调度。
 *
 * 容器分屏 `dockerPaneOpen` 与右侧栏完全独立：开关它不读写 `rightPanelView`
 * 或 `sftpPaneOpen`，关闭右侧栏同样不影响容器分屏。
 */
export function useWorkbenchPanels(options: {
  paneOrder: Ref<SshWorkbenchPaneOrder>;
  sftpPaneOpen: Ref<boolean>;
  rightPanelView: Ref<RightPanelView>;
  dockerPaneOpen: Ref<boolean>;
  localUiMode: Ref<boolean>;
  isRdpConnection: Ref<boolean>;
  connected: Ref<boolean>;
  metricsOpen: () => boolean;
  openMetrics: () => void;
  closeMetrics: () => void;
  persistState: () => void;
  scheduleFit: () => void;
  /** 每次打开容器分屏重置为默认 33%（布局域提供）。 */
  resetDockerSplitRatio?: () => void;
}) {
  const {
    paneOrder, sftpPaneOpen, rightPanelView, dockerPaneOpen, localUiMode, isRdpConnection,
    connected, metricsOpen, openMetrics, closeMetrics, persistState, scheduleFit, resetDockerSplitRatio,
  } = options;

  function togglePaneOrder() {
    paneOrder.value = paneOrder.value === "terminal-left" ? "sftp-left" : "terminal-left";
    persistState();
    void nextTick(scheduleFit);
  }

  function openRightPanel(view: RightPanelView) {
    // SFTP/指标仅在 SSH 工作台可用；容器分屏连本机 daemon 也能打开。
    if (view === "sftp" && (localUiMode.value || isRdpConnection.value)) return;
    if (view === "metrics" && (!connected.value || isRdpConnection.value)) return;
    if (view !== "metrics" && metricsOpen()) closeMetrics();
    rightPanelView.value = view;
    sftpPaneOpen.value = true;
    if (view === "metrics") openMetrics();
    persistState();
    void nextTick(scheduleFit);
  }

  function closeRightPanel() {
    if (metricsOpen()) closeMetrics();
    sftpPaneOpen.value = false;
    persistState();
    void nextTick(scheduleFit);
  }

  function toggleSftpPane() {
    if (sftpPaneOpen.value && rightPanelView.value === "sftp") closeRightPanel();
    else openRightPanel("sftp");
  }

  function toggleMetricsPanel() {
    if (sftpPaneOpen.value && rightPanelView.value === "metrics") closeRightPanel();
    else openRightPanel("metrics");
  }

  /** 容器分屏开关：独立于右侧栏；每次打开回到默认比例（不落盘）。 */
  function toggleDockerPane() {
    if (isRdpConnection.value) return;
    dockerPaneOpen.value = !dockerPaneOpen.value;
    if (dockerPaneOpen.value) resetDockerSplitRatio?.();
    persistState();
    void nextTick(scheduleFit);
  }

  function closeDockerPane() {
    if (!dockerPaneOpen.value) return;
    dockerPaneOpen.value = false;
    persistState();
    void nextTick(scheduleFit);
  }

  return {
    togglePaneOrder,
    openRightPanel,
    closeRightPanel,
    toggleSftpPane,
    toggleMetricsPanel,
    toggleDockerPane,
    closeDockerPane,
  };
}
