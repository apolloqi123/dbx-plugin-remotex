// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useWorkbenchPanels } from "./composables/useWorkbenchPanels";
import { useLayoutState } from "./state/useLayoutState";
import workspaceSource from "../components/WorkbenchWorkspace.vue?raw";
import type { SshWorkbenchPaneOrder } from "../lib/workbenchLayout";

function options() {
  return {
    paneOrder: ref<SshWorkbenchPaneOrder>("terminal-left"),
    sftpPaneOpen: ref(true),
    rightPanelView: ref<"sftp" | "metrics">("sftp"),
    dockerPaneOpen: ref(false),
    localUiMode: ref(false),
    isRdpConnection: ref(false),
    connected: ref(true),
    metricsOpen: () => false,
    openMetrics: vi.fn(),
    closeMetrics: vi.fn(),
    persistState: vi.fn(),
    scheduleFit: vi.fn(),
    resetDockerSplitRatio: vi.fn(),
  };
}

describe("workbench panels", () => {
  it("toggles the container pane without touching SFTP", () => {
    const input = options();
    const panels = useWorkbenchPanels(input);
    panels.toggleDockerPane();
    expect(input.dockerPaneOpen.value).toBe(true);
    expect(input.sftpPaneOpen.value).toBe(true);
    expect(input.rightPanelView.value).toBe("sftp");
    expect(input.resetDockerSplitRatio).toHaveBeenCalledOnce();
    panels.toggleDockerPane();
    expect(input.dockerPaneOpen.value).toBe(false);
    expect(input.sftpPaneOpen.value).toBe(true);
  });

  it("keeps the container pane when the SFTP panel closes", () => {
    const input = options();
    input.dockerPaneOpen.value = true;
    const panels = useWorkbenchPanels(input);
    panels.closeRightPanel();
    expect(input.dockerPaneOpen.value).toBe(true);
    expect(input.sftpPaneOpen.value).toBe(false);
  });

  it("refuses to open the container pane for RDP connections", () => {
    const input = options();
    input.isRdpConnection.value = true;
    const panels = useWorkbenchPanels(input);
    panels.toggleDockerPane();
    expect(input.dockerPaneOpen.value).toBe(false);
  });
});

describe("container split defaults", () => {
  it("opens the container pane at a 50/50 split and resets to it", () => {
    const layout = useLayoutState({ paneDirection: ref("auto"), sftpPaneOpen: ref(false) });
    expect(layout.dockerSplitRatio.value).toBe(50);
    expect(layout.dockerPaneBasis.value).toEqual({ flexBasis: "50%" });
    layout.dockerSplitRatio.value = 72;
    layout.resetDockerSplitRatio();
    expect(layout.dockerSplitRatio.value).toBe(50);
    expect(layout.dockerPaneBasis.value).toEqual({ flexBasis: "50%" });
  });
});

describe("SFTP panel default width", () => {
  it("gives the terminal 75% and the SFTP panel 25% by default", () => {
    const layout = useLayoutState({ paneDirection: ref("auto"), sftpPaneOpen: ref(true) });
    expect(layout.splitRatio.value).toBe(75);
    expect(layout.terminalBasis.value).toEqual({ flexBasis: "75%" });
  });
});

describe("container pane surface", () => {
  it("renders the container manager below the terminal stack", () => {
    expect(workspaceSource).toContain("<ContainerManagerPane");
    expect(workspaceSource).toContain('class="docker-pane-surface"');
    expect(workspaceSource).toContain(':style="dockerPaneBasis"');
    expect(workspaceSource).toContain('class="docker-divider"');
    expect(workspaceSource).toContain('@close="closeDockerPane"');
  });
});
