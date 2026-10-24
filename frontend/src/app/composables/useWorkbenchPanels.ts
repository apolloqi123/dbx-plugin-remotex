import { nextTick, type Ref } from "vue";
import type { SshWorkbenchPaneOrder } from "../lib/workbenchLayout";

type RightPanelView = "sftp" | "metrics" | "docker";

/**
 * 工作台右侧栏三视图（SFTP/指标/容器）与左右分栏顺序的开关语义：
 * 指标/容器视图本体由各自 composable/组件承载，这里只做互斥、可用性门与
 * 布局偏好落盘（persistState）/重排 fit 调度。
 */
export function useWorkbenchPanels(options: {
  paneOrder: Ref<SshWorkbenchPaneOrder>;
  sftpPaneOpen: Ref<boolean>;
  rightPanelView: Ref<RightPanelView>;
  localUiMode: Ref<boolean>;
  isRdpConnection: Ref<boolean>;
  connected: Ref<boolean>;
  metricsOpen: () => boolean;
  openMetrics: () => void;
  closeMetrics: () => void;
  persistState: () => void;
  scheduleFit: () => void;
}) {
  const {
    paneOrder, sftpPaneOpen, rightPanelView, localUiMode, isRdpConnection,
    connected, metricsOpen, openMetrics, closeMetrics, persistState, scheduleFit,
  } = options;

  function togglePaneOrder() {
    paneOrder.value = paneOrder.value === "terminal-left" ? "sftp-left" : "terminal-left";
    persistState();
    void nextTick(scheduleFit);
  }

  function openRightPanel(view: RightPanelView) {
    // SFTP/指标仅在 SSH 工作台可用；容器面板连本机 daemon 也可用。
    if (view === "sftp" && (localUiMode.value || isRdpConnection.value)) return;
    if (view === "metrics" && (!connected.value || isRdpConnection.value)) return;
    if (view === "docker" && isRdpConnection.value) return;
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

  function toggleDockerPanel() {
    if (sftpPaneOpen.value && rightPanelView.value === "docker") closeRightPanel();
    else openRightPanel("docker");
  }

  return {
    togglePaneOrder,
    openRightPanel,
    closeRightPanel,
    toggleSftpPane,
    toggleMetricsPanel,
    toggleDockerPanel,
  };
}
