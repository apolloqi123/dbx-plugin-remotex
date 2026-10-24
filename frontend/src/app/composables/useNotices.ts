import { ref } from "vue";
import { friendlySftpError } from "../../features/sftp/lib/sftpErrors";

export interface BannerAction {
  label: string;
  run: () => void;
}

/**
 * 通知/错误横幅：reka Toast 承载展示与计时（悬停暂停/滑动关闭为内建行为），
 * 通知为单实例替换语义——每次 show 递增 key 重挂 ToastRoot。
 */
export function useNotices(options: { t: (key: string) => string }) {
  const terminalError = ref("");
  const sftpError = ref("");
  const sftpErrorRetry = ref<(() => void) | null>(null);
  const sftpErrorOpen = ref(false);
  const sftpErrorKey = ref(0);
  const notice = ref("");
  const noticeActions = ref<BannerAction[]>([]);
  const noticeOpen = ref(false);
  const noticeKey = ref(0);

  function showNotice(message: string, actions: BannerAction[] = []) {
    notice.value = message;
    noticeActions.value = actions;
    noticeKey.value += 1;
    noticeOpen.value = true;
  }

  function onNoticeOpenChange(open: boolean) {
    if (open) return;
    noticeOpen.value = false;
    notice.value = "";
    noticeActions.value = [];
  }

  function onSftpErrorOpenChange(open: boolean) {
    if (open) return;
    sftpErrorOpen.value = false;
    sftpError.value = "";
    sftpErrorRetry.value = null;
  }

  function showError(cause: unknown, target: "terminal" | "sftp" = "sftp", retry?: () => void) {
    const message = cause instanceof Error ? cause.message : String(cause);
    // 常见错误（权限不足/文件不存在）翻成友好文案；其余原样透出。
    const display = friendlySftpError(message, options.t) ?? message;
    if (target === "terminal") terminalError.value = display;
    else {
      sftpError.value = display;
      sftpErrorRetry.value = retry ?? null;
      // 错误横幅与通知同款自动消失（保留手动关闭），时限放宽到 8s：
      // 错误信息通常更长，需要读完的时间。
      sftpErrorKey.value += 1;
      sftpErrorOpen.value = true;
    }
  }

  return {
    terminalError,
    sftpError,
    sftpErrorRetry,
    sftpErrorOpen,
    sftpErrorKey,
    notice,
    noticeActions,
    noticeOpen,
    noticeKey,
    showNotice,
    onNoticeOpenChange,
    onSftpErrorOpenChange,
    showError,
  };
}
