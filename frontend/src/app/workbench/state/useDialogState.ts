import { ref } from "vue";

/**
 * UI 开关域状态（Task 21b）：设置/审计/配置档弹窗、OTP 与导入面板、端口转发、
 * 会话下拉与幽灵键帽菜单。
 *
 * 纯本地工厂：弹窗内容与 Esc 链（useWorkbenchModals）仍由 App 接线，这里只
 * 保留开关声明与初始值（全部默认关闭）。
 */
export function useDialogState() {
  const settingsOpen = ref(false);
  const auditOpen = ref(false);
  const profilesOpen = ref(false);
  // OTP 验证码 / 会话导入工具弹窗（SFTP 路径工具栏入口）。
  const otpPanelOpen = ref(false);
  const importPanelOpen = ref(false);
  // 端口映射：工具栏快捷弹层 + 完整管理器（两者共享 usePortForwards 控制器）。
  // focusForm 让「新增映射」入口打开管理器时直接展开表单。
  const forwardsMenuOpen = ref(false);
  const forwardsOpen = ref(false);
  const forwardsFocusForm = ref(false);
  // 键位胶囊（Warp accept-autosuggestion-hint 同位）的下拉菜单与展示标签。
  const ghostKeycapMenuOpen = ref(false);
  // 会话下拉（新建/复制/命令会话/本地终端/Shell 选项）单入口状态；shell 视图
  // 为同弹层内的视图切换（非独立 popover，Esc/外点由 reka 随弹层整体收口）。
  const sessionMenuOpen = ref(false);
  const sessionMenuShellOpen = ref(false);
  // 「选择 Shell 打开」子视图：同弹层视图切换（dock「+」同款列表），语义是
  // 一次性 override 打开，与设置子视图（radio = 默认偏好）刻意分开。
  const sessionMenuShellPickOpen = ref(false);

  return {
    settingsOpen,
    auditOpen,
    profilesOpen,
    otpPanelOpen,
    importPanelOpen,
    forwardsMenuOpen,
    forwardsOpen,
    forwardsFocusForm,
    ghostKeycapMenuOpen,
    sessionMenuOpen,
    sessionMenuShellOpen,
    sessionMenuShellPickOpen,
  };
}

export type DialogState = ReturnType<typeof useDialogState>;
