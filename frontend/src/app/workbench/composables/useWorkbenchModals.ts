import { computed, nextTick, watch, type Ref } from "vue";
import { pickModalFocusTarget } from "../../../shared/lib/modalFocus";
import { createGhostClickGuard } from "../../../features/completion/lib/ghostClickGuard";
import { isCountdownActive } from "../../../features/recording/lib/recordingCountdown";

/** 只读状态门：ref 与 computed 皆可（本模块只观察，不写回）。 */
type ReadonlyValue<T> = { readonly value: T };
/** 弹层对象/目标状态：本模块只做真值与 null 判定，写回一律复位（null/undefined）。 */
type DialogState = Ref<unknown>;
/** 编辑器内联 Esc 消费端（SettingsDialog 实例）：只问一句「表单是否已吃掉 Esc」。 */
type InlineEscConsumer = { consumeInlineEsc(): boolean };

/**
 * 弹层焦点管理与 Esc 关闭链（从 App.vue 原样迁出）：触发元素栈 / 幽灵点击
 * 守卫 / 开状态焦点表 / 逐层 Esc 退出。状态本体（各弹层 open ref 与动作
 * 回调）仍由 App 持有并经 options 注入，这里只做仲裁；监听器注册与注销
 * 仍在 App 的 onMounted/onBeforeUnmount（文档事件挂载点不随抽取改变）。
 */
export function useWorkbenchModals(options: {
  // —— modalOpenStates：参与聚焦与触发元素归还的弹层在开状态 ——
  localOpenConfirmOpen: ReadonlyValue<boolean>;
  telnetDialogOpen: ReadonlyValue<boolean>;
  serialDialogOpen: ReadonlyValue<boolean>;
  serialUploadDialogOpen: ReadonlyValue<boolean>;
  vncDialogOpen: ReadonlyValue<boolean>;
  rdpDialogOpen: ReadonlyValue<boolean>;
  rdpCertPrompt: DialogState;
  folderPickerTarget: DialogState;
  otpPanelOpen: Ref<boolean>;
  importPanelOpen: Ref<boolean>;
  previewOpen: ReadonlyValue<boolean>;
  pasteConfirm: DialogState;
  pendingConfirmDialog: DialogState;
  dropUploadPrompt: DialogState;
  downloadPrompt: DialogState;
  downloadConflictPrompt: DialogState;
  uploadDuplicatePrompt: DialogState;
  attrsTarget: DialogState;
  deleteTarget: DialogState;
  batchDeleteOpen: Ref<boolean>;
  recordingDeleteTarget: DialogState;
  recordingClearAllOpen: Ref<boolean>;
  transferHistoryClearOpen: Ref<boolean>;
  chmodTarget: DialogState;
  newFileDialog: Ref<boolean>;
  operationDialog: DialogState;
  symlinkDialog: DialogState;
  watchModifiedQueue: ReadonlyValue<unknown[]>;
  customEditorOpen: ReadonlyValue<boolean>;
  profilesOpen: Ref<boolean>;
  auditOpen: Ref<boolean>;
  auditClearOpen: Ref<boolean>;
  settingsOpen: Ref<boolean>;
  alertTriageOpen: Ref<boolean>;
  hostKeyPrompt: DialogState;
  agentPromptHead: DialogState;
  spawnSessionDialogOpen: ReadonlyValue<boolean>;
  // —— Esc 链的动作依赖（提交中禁用、逐层退出调用的收口动作）——
  batchDeleteSubmitting: ReadonlyValue<boolean>;
  recordingDeleteSubmitting: ReadonlyValue<boolean>;
  recordingClearAllSubmitting: ReadonlyValue<boolean>;
  auditClearSubmitting: ReadonlyValue<boolean>;
  metricsOpen: ReadonlyValue<boolean>;
  recordingsOpen: Ref<boolean>;
  batchSaveMode: ReadonlyValue<boolean>;
  dockerPaneOpen: ReadonlyValue<boolean>;
  settingsDialog: ReadonlyValue<InlineEscConsumer | undefined>;
  recordCountdown: ReadonlyValue<number | null>;
  hasEditorTabs: ReadonlyValue<boolean>;
  // —— 键盘动作回调表：编辑器保存快捷键与逐层退出动作 ——
  editorPaneContainsFocus: () => boolean;
  saveActive: () => Promise<void>;
  cancelRecordCountdown: () => void;
  closePreview: () => Promise<void>;
  resolvePasteConfirm: (accepted: boolean) => void;
  resolvePendingConfirmDialog: (accepted: boolean) => void;
  resolveDropUpload: (choice: "cancel" | "cwd" | { dir: string }) => void;
  resolveDownloadPrompt: (result?: { dir: string; setDefault: boolean }) => void;
  resolveDownloadConflict: (choice: "rename" | "overwrite" | undefined) => void;
  resolveUploadDuplicate: (choice: "overwrite" | "rename" | undefined) => void;
  closeAttributes: () => void;
  cancelBatchBarSave: () => void;
  closeDockerPane: () => void;
  closeRightPanel: () => void;
}) {
  const {
    localOpenConfirmOpen, telnetDialogOpen, serialDialogOpen, serialUploadDialogOpen,
    vncDialogOpen, rdpDialogOpen, rdpCertPrompt, folderPickerTarget, otpPanelOpen,
    importPanelOpen, previewOpen, pasteConfirm, pendingConfirmDialog, dropUploadPrompt,
    downloadPrompt, downloadConflictPrompt, uploadDuplicatePrompt, attrsTarget,
    deleteTarget, batchDeleteOpen, recordingDeleteTarget, recordingClearAllOpen,
    transferHistoryClearOpen, chmodTarget, newFileDialog, operationDialog, symlinkDialog,
    watchModifiedQueue, customEditorOpen, profilesOpen, auditOpen,
    auditClearOpen, settingsOpen, alertTriageOpen, hostKeyPrompt, agentPromptHead,
    spawnSessionDialogOpen,
    batchDeleteSubmitting, recordingDeleteSubmitting, recordingClearAllSubmitting,
    auditClearSubmitting, metricsOpen, recordingsOpen, batchSaveMode, dockerPaneOpen,
    settingsDialog, recordCountdown, hasEditorTabs,
    editorPaneContainsFocus, saveActive, cancelRecordCountdown, closePreview,
    resolvePasteConfirm, resolvePendingConfirmDialog, resolveDropUpload,
    resolveDownloadPrompt, resolveDownloadConflict, resolveUploadDuplicate,
    closeAttributes, cancelBatchBarSave, closeDockerPane, closeRightPanel,
  } = options;

  /**
   * 弹层焦点管理（P1-2）：打开时焦点进入弹层首控件、关闭后归还触发元素。
   * 原生 autofocus 在 Vue 动态插入时不生效，改为显式驱动；Tab 圈定已移交
   * reka Dialog 的 FocusScope（Phase 6），Esc 关闭链沿用下方
   * onDocumentKeydown 的分层退出。
   */
  // 触发元素栈：与弹层嵌套深度同步 push/pop。右键菜单项这类"打开弹层后自身
  // 随菜单卸载"的触发元素无法承接归还焦点，逐层弹出时跳过已断连元素。
  const modalTriggerStack: HTMLElement[] = [];
  // 弹层外最近聚焦的稳定元素：右键菜单项属瞬态控件（点击打开弹层后随菜单
  // 卸载，无法承接归还焦点），归还目标回退到菜单打开前的焦点宿主；
  // 弹层内聚焦不覆盖该记录。
  let lastStableFocus: HTMLElement | null = null;
  // 幽灵点击守卫（R3-P1-1）：焦点归还后短窗内拦截无 mousedown 前驱的合成
  // click；决策逻辑走 ghostClickGuard 纯模块（有单测），真实鼠标点击放行。
  const ghostClickGuard = createGhostClickGuard();
  function onDocumentMouseDownCapture(event: MouseEvent) {
    ghostClickGuard.noteMouseDown();
    // 批量目标/高亮规则两个 popover 的点空白收起已移交 reka DismissableLayer
    // （pointerdown-outside → update:open(false)），capture 手动清扫移除。
  }
  function onDocumentClickCapture(event: MouseEvent) {
    if (!ghostClickGuard.shouldSuppress()) return;
    event.preventDefault();
    event.stopPropagation();
  }
  function trackStableFocus(event: FocusEvent) {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    if (target.closest('[data-slot="dialog-content"]') || target.closest('[data-slot="context-menu-content"]')) return;
    lastStableFocus = target;
  }
  // 与 Esc 关闭链同源的弹层在开状态（hostKey/agent 审批属安全弹窗：
  // 参与聚焦与 Tab 陷阱，但不参与 Esc 关闭）。按模板出现顺序排列，
  // 计数变化驱动聚焦/归还；同层互斥由交互保证。
  const modalOpenStates = computed(() => [
    localOpenConfirmOpen.value,
    telnetDialogOpen.value,
    serialDialogOpen.value,
    serialUploadDialogOpen.value,
    vncDialogOpen.value,
    rdpDialogOpen.value,
    rdpCertPrompt.value !== null,
    folderPickerTarget.value !== null,
    // SFTP 工具栏工具弹窗（OTP / 导入会话），模板顺序在预览弹窗之前。
    otpPanelOpen.value,
    importPanelOpen.value,
    previewOpen.value,
    pasteConfirm.value,
    // 通用确认弹窗（SSH-H1）：模板顺序在 pasteConfirm 之后、dropUpload 之前。
    !!pendingConfirmDialog.value,

    dropUploadPrompt.value,
    downloadPrompt.value !== null,
    downloadConflictPrompt.value !== null,
    uploadDuplicatePrompt.value !== null,
    attrsTarget.value,
    deleteTarget.value,
    batchDeleteOpen.value,
    recordingDeleteTarget.value !== null,
    recordingClearAllOpen.value,
    transferHistoryClearOpen.value,
    chmodTarget.value,
    newFileDialog.value,
    operationDialog.value,
    symlinkDialog.value !== null,
    watchModifiedQueue.value.length > 0,
    // 自定义编辑器命令弹窗（编辑器映射增强），模板顺序在 watch 确认弹窗之后。
    customEditorOpen.value,
    profilesOpen.value,
    auditOpen.value,
    auditClearOpen.value,
    settingsOpen.value,
    alertTriageOpen.value,
    hostKeyPrompt.value,
    agentPromptHead.value,
    spawnSessionDialogOpen.value,
  ]);
  const modalOpenCount = computed(() => modalOpenStates.value.filter(Boolean).length);

  /** 当前最顶层弹层容器；无弹层时返回 null（嵌套时取首个命中即最外层——焦点回落语义）。 */
  function topModalContainer(): HTMLElement | null {
    if (!modalOpenCount.value) return null;
    return document.querySelector<HTMLElement>('[data-slot="dialog-content"]');
  }

  function focusTopModal() {
    pickModalFocusTarget(topModalContainer())?.focus({ preventScroll: true });
  }

  watch(modalOpenCount, (count, previous) => {
    if (count > previous) {
      // 打开：整组从无到有时记录触发元素供关闭归还；嵌套打开（如命令
      // 对话框上叠粘贴确认）逐层入栈。触发元素优先取"弹层外稳定焦点"，
      // 避免抓到已随右键菜单卸载的菜单项。
      for (let i = previous; i < count; i++) {
        modalTriggerStack.push(lastStableFocus ?? document.body);
      }
      void nextTick(focusTopModal);
      return;
    }
    if (!count) {
      // 全部关闭：焦点归还触发按钮；触发元素已随右键菜单等卸载时逐层回退，
      // 找不到任何在档元素则落回 BODY（无焦点宿主可还）。
      while (modalTriggerStack.length) {
        const trigger = modalTriggerStack.pop()!;
        if (trigger.isConnected) {
          // 幽灵点击守卫（R3-P1-1）：键盘 Enter 提交后归还焦点的瞬间，浏览器
          // 会在刚聚焦的按钮上派发一次无 mousedown 的合成 click 并重开弹层；
          // 短窗内拦截该 click，纯键盘流一次 Enter 即成功关闭。
          ghostClickGuard.arm();
          trigger.focus({ preventScroll: true });
          break;
        }
      }
      return;
    }
    // 内层弹层关闭、外层仍在：焦点回落外层弹层首控件。
    void nextTick(focusTopModal);
  });

  /**
   * Esc 关闭链（一次按键关一层）：预览 > 对话框 > 右键菜单 > 工具栏弹出层。
   * 逐层 if-return：无内容打开时按键穿透，不影响终端内 vim 等自身 Esc 语义。
   */
  function onDocumentKeydown(event: KeyboardEvent) {
    // 编辑器保存快捷键：仅当焦点在编辑器面板内且存在页签时接管 Cmd/Ctrl+S；
    // 终端内不抢（保留用户既有终端/远端程序语义）。
    if (
      (event.metaKey || event.ctrlKey) &&
      !event.altKey &&
      event.key.toLowerCase() === "s" &&
      hasEditorTabs.value &&
      editorPaneContainsFocus()
    ) {
      event.preventDefault();
      void saveActive();
      return;
    }
    // Tab 焦点陷阱已移交 reka Dialog 的 FocusScope（每个弹层独立圈定，嵌套时顶层
    // 生效）；此处不再拦截 Tab。注意 FocusScope 的 keydown 挂在内容元素上，冒泡先于
    // 本 document 处理器，若恢复自制陷阱会与 reka 双步进，勿回退。
    if (event.key === "Tab") return;
    if (event.key !== "Escape") return;
    // reka Select/ContextMenu/Popover 弹层打开时 Esc 由 reka 消费（关弹层），不落入下方
    // 弹窗关闭链。本处理器注册早于 DismissableLayer，先跑时弹层 DOM 仍在。
    if (document.querySelector('[data-slot="select-content"], [data-slot="context-menu-content"], [data-slot="popover-content"]')) return;
    // 录制倒计时优先取消（遮罩在终端区，不属于弹层体系）。
    if (isCountdownActive(recordCountdown.value)) {
      cancelRecordCountdown();
      return;
    }
    if (previewOpen.value) {
      closePreview();
      return;
    }
    if (otpPanelOpen.value) {
      otpPanelOpen.value = false;
      return;
    }
    if (importPanelOpen.value) {
      importPanelOpen.value = false;
      return;
    }
    // ---- 对话框（安全取消语义；hostKey/agent 审批等安全弹窗不在此列）----
    if (folderPickerTarget.value) {
      folderPickerTarget.value = null;
      return;
    }
    if (pasteConfirm.value) {
      resolvePasteConfirm(false);
      return;
    }
    if (pendingConfirmDialog.value) {
      resolvePendingConfirmDialog(false);
      return;
    }
    if (dropUploadPrompt.value) {
      resolveDropUpload("cancel");
      return;
    }
    if (downloadPrompt.value) {
      resolveDownloadPrompt(undefined);
      return;
    }
    if (downloadConflictPrompt.value) {
      resolveDownloadConflict(undefined);
      return;
    }
    if (uploadDuplicatePrompt.value) {
      resolveUploadDuplicate(undefined);
      return;
    }
    if (attrsTarget.value) {
      closeAttributes();
      return;
    }
    if (deleteTarget.value) {
      deleteTarget.value = undefined;
      return;
    }
    if (batchDeleteOpen.value) {
      if (!batchDeleteSubmitting.value) batchDeleteOpen.value = false;
      return;
    }
    if (recordingDeleteTarget.value) {
      if (!recordingDeleteSubmitting.value) recordingDeleteTarget.value = null;
      return;
    }
    if (recordingClearAllOpen.value) {
      if (!recordingClearAllSubmitting.value) recordingClearAllOpen.value = false;
      return;
    }
    if (transferHistoryClearOpen.value) {
      transferHistoryClearOpen.value = false;
      return;
    }
    if (chmodTarget.value) {
      chmodTarget.value = undefined;
      return;
    }
    if (newFileDialog.value) {
      newFileDialog.value = false;
      return;
    }
    if (operationDialog.value) {
      operationDialog.value = null;
      return;
    }
    if (alertTriageOpen.value) {
      alertTriageOpen.value = false;
      return;
    }
    if (profilesOpen.value) {
      profilesOpen.value = false;
      return;
    }
    if (auditOpen.value) {
      if (auditClearOpen.value) {
        if (!auditClearSubmitting.value) auditClearOpen.value = false;
        return;
      }
      auditOpen.value = false;
      return;
    }
    if (settingsOpen.value) {
      // 内联 profile 管理（设置弹窗内）沿用 profilesOpen→settingsOpen 的逐层
      // 退出语义：先关编辑表单，再收起配置档 section，最后关弹窗
      // （表单/section 状态在 SettingsDialog 内部，经组件实例询问是否已消费）。
      if (settingsDialog.value?.consumeInlineEsc()) return;
      settingsOpen.value = false;
      return;
    }
    // 右键菜单与九个工具栏 popover 均已迁移 reka（ContextMenu/Popover）：Esc 与外点
    // 由 reka 自行消费（见上方 content 守卫），不再占 Esc 链一层。本层只剩
    // 指标/录制、容器分屏与批量保存态（带草稿清理）。容器分屏与右侧栏可同时
    // 打开，一次 Esc 只收一层：先容器分屏，再右侧栏。容器页内部确认弹层/引擎
    // popover 自带 Esc 消费（捕获并 stopPropagation），不会落到这一层。
    if (metricsOpen.value || recordingsOpen.value || batchSaveMode.value || dockerPaneOpen.value) {
      if (batchSaveMode.value) cancelBatchBarSave();
      if (dockerPaneOpen.value) closeDockerPane();
      else if (metricsOpen.value) closeRightPanel();
      if (recordingsOpen.value) recordingsOpen.value = false;
    }
  }

  return {
    modalOpenStates,
    modalOpenCount,
    topModalContainer,
    focusTopModal,
    onDocumentMouseDownCapture,
    onDocumentClickCapture,
    trackStableFocus,
    onDocumentKeydown,
  };
}
