import { ref } from "vue";
import { clampTransferConcurrency, clampTransferDownloadLimit, clampTransferMaxActive, sanitizeTransferDuplicatePolicy, type TransferDuplicatePolicy } from "../lib/transferQueue";

/** 传输偏好（并发/重复策略/最大并发/下载限速）：权威态在内存，localStorage
 *  仅作同步缓存（语义同下载偏好），sidecar preferences 经 syncPrefs 同步。 */
export function useTransferPreferences(options: { syncPrefs: () => void }) {
  const { syncPrefs } = options;

  const TRANSFER_CONCURRENCY_KEY = "ssh-transfer-concurrency";
  const TRANSFER_MAX_ACTIVE_KEY = "ssh-transfer-max-active";
  // 下载限速（issue #66，KiB/s，0=不限速缺省）：同一偏好链路持久化。
  const TRANSFER_DOWNLOAD_LIMIT_KEY = "ssh-transfer-download-limit-kib";

  const TRANSFER_DUPLICATE_KEY = "ssh-transfer-duplicate-policy";

  const transferConcurrencyState = ref(3);

  const transferMaxActiveState = ref(3);
  // 下载限速权威态（KiB/s，0=不限速）；sidecar preferences 同步。
  const transferDownloadLimitState = ref(0);

  const transferDuplicateState = ref<TransferDuplicatePolicy>("ask");

  function loadTransferConcurrency(): number {
    return transferConcurrencyState.value;
  }

  function persistTransferConcurrency(value: number) {
    transferConcurrencyState.value = clampTransferConcurrency(value);
    void syncPrefs();
  }

  function loadTransferDuplicatePolicy(): TransferDuplicatePolicy {
    return transferDuplicateState.value;
  }

  function persistTransferDuplicatePolicy(value: TransferDuplicatePolicy) {
    transferDuplicateState.value = sanitizeTransferDuplicatePolicy(value);
    void syncPrefs();
  }

  // M14-B 三键读写（设置弹窗经适配器调用）。
  function loadTransferMaxActive(): number {
    return transferMaxActiveState.value;
  }

  function persistTransferMaxActive(value: number) {
    transferMaxActiveState.value = clampTransferMaxActive(value);
    void syncPrefs();
  }

  // 下载限速（issue #66）：设置弹窗经适配器读写，权威态在此。
  function loadTransferDownloadLimit(): number {
    return transferDownloadLimitState.value;
  }

  function persistTransferDownloadLimit(value: number) {
    transferDownloadLimitState.value = clampTransferDownloadLimit(value);
    void syncPrefs();
  }

  return {
    TRANSFER_CONCURRENCY_KEY,
    TRANSFER_MAX_ACTIVE_KEY,
    TRANSFER_DOWNLOAD_LIMIT_KEY,
    TRANSFER_DUPLICATE_KEY,
    transferConcurrencyState,
    transferMaxActiveState,
    transferDownloadLimitState,
    transferDuplicateState,
    loadTransferConcurrency,
    persistTransferConcurrency,
    loadTransferDuplicatePolicy,
    persistTransferDuplicatePolicy,
    loadTransferMaxActive,
    persistTransferMaxActive,
    loadTransferDownloadLimit,
    persistTransferDownloadLimit,
  };
}
