import { computed, ref } from "vue";
import { resolveAppearance } from "../../../shared/lib/appearance";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import { resolveWorkbenchId } from "../../../shared/lib/pluginContext";
import { randomUUID } from "../../../shared/lib/uuid";
import { useDialogState } from "./useDialogState";
import { useLayoutState } from "./useLayoutState";
import { useSessionState } from "./useSessionState";
import { useSftpState } from "./useSftpState";
import { useTerminalState } from "./useTerminalState";

export interface WorkbenchContextOptions {
  /** SFTP 面板默认开合（useWorkbenchViewPrefs 在 setup 期求值的结果）。 */
  sftpPaneDefaultOpen: boolean;
  /** 布局轴向偏好（useWorkbenchViewPrefs 持有，paneLayout 消费）。 */
  paneDirection: { readonly value: PaneDirectionPref };
}

/**
 * 工作台状态上下文（Task 21b）：按域创建 refs 并组合成一个扁平命名空间对象。
 *
 * 纯本地工厂（不是全局单例）：每次调用产生一份独立状态，App.vue 在同一位置
 * 解构同名绑定，模板与既有接线无需改动。宿主上下文（hostContext/appearance/
 * locale/workbenchId）也在此单点创建——`fallbackWorkbenchId` 必须继续走
 * lib/uuid shim（issue #77：沙箱里裸 crypto.randomUUID 在非安全上下文抛错）。
 */
export function useWorkbenchContext(options: WorkbenchContextOptions) {
  const hostContext = ref<Record<string, unknown>>({});
  // 宿主未下发 appearance 前的兜底：DBX `.dark` 规范令牌。
  const appearance = ref(resolveAppearance());
  const locale = ref("zh-CN");
  // Host API 1.1 provides a stable workbenchId in the host context; on 1.0 a
  // locally generated id keeps session scoping per workbench instance (A4 W1
  // helper, spec §11: host-authoritative workbenchId; the fallback covers 1.0
  // hosts that omit the injection — see lib/pluginContext.spec.ts).
  const fallbackWorkbenchId = randomUUID();
  const workbenchId = computed(() => resolveWorkbenchId(hostContext.value, fallbackWorkbenchId));

  const terminal = useTerminalState({ appearance });
  const session = useSessionState({ hostContext, appearance, terminalState: terminal.terminalState });
  const sftp = useSftpState({ initialPaneOpen: options.sftpPaneDefaultOpen });
  const layout = useLayoutState({
    paneDirection: options.paneDirection,
    sftpPaneOpen: sftp.sftpPaneOpen,
    dockerPanelOpen: sftp.dockerPanelOpen,
  });
  const dialogs = useDialogState();

  return {
    hostContext,
    appearance,
    locale,
    workbenchId,
    terminal,
    session,
    sftp,
    layout,
    dialogs,
  };
}

export type WorkbenchContext = ReturnType<typeof useWorkbenchContext>;
