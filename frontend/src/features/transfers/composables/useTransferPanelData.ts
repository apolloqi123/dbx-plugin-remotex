import { type TransferTask } from "../lib/transferQueue";
import { computed, ref, watch } from "vue";
import { isLiveTransferStatus, sortTransferTasks } from "../lib/transferOrder";

/** 传输面板数据源：开合态、右键菜单负载、活跃区/在途计数，以及面板打开
 *  （或任务转终态）时的历史/对账刷新节拍。 */
export function useTransferPanelData(options: {
  transferTasks: Record<string, TransferTask>;
  refreshTransferHistory: () => Promise<void>;
  refreshResumableUploads: () => Promise<void>;
  restoreTransfers: () => Promise<void>;
  reconcileActiveTransfers: () => Promise<void>;
}) {
  const { transferTasks, refreshTransferHistory, refreshResumableUploads, restoreTransfers, reconcileActiveTransfers } = options;

  const transferPanelOpen = ref(false);

  // 下载历史项右键：只为已有本机落盘路径提供定位/打开操作；taskId 用于按卡受控打开。
  const transferHistoryMenu = ref<{ taskId: string }>();

  // 活跃区只显示进行中的任务（queued/running），按 transferOrder 的稳定规则
  // 排序：先开始/先加入的排最上，同键用 taskId 兜底（issue #18）。此前这里
  // 按对象插入序渲染全部任务：插入序来自后端 HashMap 迭代序 + 事件到达序，
  // 终态行还永久堆积，同一张卡就会在面板里"一会儿在上、一会儿在中间、一会儿
  // 在下"。终态行由历史区承接（落盘 + 内存合并视图），转终态的同一拍刷新。
  const transferList = computed(() =>
    sortTransferTasks(Object.values(transferTasks).filter((task) => isLiveTransferStatus(task.status))),
  );
  const activeTransfers = computed(() => transferList.value.filter((task) => task.status === "queued" || task.status === "running").length);

  /** Refresh every data source rendered by the transfer popover. */
  async function refreshTransferPanel() {
    await Promise.all([
      refreshTransferHistory(),
      refreshResumableUploads(),
      restoreTransfers(),
    ]);
    await reconcileActiveTransfers();
  }

  // 打开传输面板或任一任务转为终态时拉取历史：终态卡从活跃区消失的同一拍
  // 进入历史区，不等最后一个任务结束（issue #18：有传输任务时历史也要可查）。
  // 打开面板的同时对账活跃任务，防止错过终态事件的行永远卡在 running。
  watch(transferPanelOpen, (open) => {
    if (open) {
      void refreshTransferPanel();
    }
  });
  const liveTransferIds = computed(() =>
    transferList.value.map((task) => task.taskId).join("|"),
  );
  watch(liveTransferIds, (current, previous) => {
    if (!transferPanelOpen.value) return;
    const before = new Set((previous ?? "").split("|").filter(Boolean));
    const after = new Set(current.split("|").filter(Boolean));
    // 只有任务离开活跃集合（转终态）才刷新；新任务加入由面板打开路径负责。
    const departed = [...before].some((taskId) => !after.has(taskId));
    if (departed) void refreshTransferHistory();
  });

  return {
    transferPanelOpen,
    transferHistoryMenu,
    transferList,
    activeTransfers,
    refreshTransferPanel,
  };
}
