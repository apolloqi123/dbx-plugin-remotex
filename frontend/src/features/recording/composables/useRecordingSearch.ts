import { computed, ref, watch, type Ref } from "vue";
import { createRequestEpoch } from "../../../shared/lib/requestEpoch";
import { rpc } from "../../../shared/lib/rpc";
import type { RecordingSummary } from "../lib/replayScheduler";

/** 录制列表搜索（名称过滤 + 内容全文命中摘录）：内容搜索走
 * ssh/recording/search 即时扫描（无持久索引），只回命中摘录，前端不在搜索
 * 路径上拉全量事件。列表数据由调用方持有（recordings）。 */
export function useRecordingSearch(options: { recordings: Ref<RecordingSummary[]> }) {
  const { recordings } = options;

const recordingsQuery = ref("");
const recordingHits = ref<Record<string, string[]>>({});
const recordingSearchBusy = ref(false);
let recordingSearchTimer = 0;

const filteredRecordings = computed(() => {
  const query = recordingsQuery.value.trim().toLowerCase();
  if (!query) return recordings.value;
  return recordings.value.filter((item) => {
    if ((item.host || "").toLowerCase().includes(query)) return true;
    if (item.recordingId.toLowerCase().includes(query)) return true;
    return (recordingHits.value[item.recordingId] ?? []).length > 0;
  });
});

const searchEpoch = createRequestEpoch();

async function runRecordingSearch() {
  const query = recordingsQuery.value.trim();
  if (!query) {
    recordingHits.value = {};
    recordingSearchBusy.value = false;
    return;
  }
  // 即时全文扫描无持久索引，慢查询可能晚于新查询返回：过期响应整体丢弃，
  // 不写 hits、不收 busy（收尾只属于最新请求）。
  const epoch = searchEpoch.next();
  recordingSearchBusy.value = true;
  try {
    const result = await rpc<{ recordings: Array<{ recordingId: string; hits?: Array<{ excerpt: string }> }> }>(
      "ssh/recording/search",
      { query },
    );
    if (!searchEpoch.isCurrent(epoch)) return;
    const hits: Record<string, string[]> = {};
    for (const row of result.recordings ?? []) {
      hits[row.recordingId] = (row.hits ?? []).map((hit) => hit.excerpt);
    }
    recordingHits.value = hits;
  } catch {
    if (!searchEpoch.isCurrent(epoch)) return;
    recordingHits.value = {};
  } finally {
    if (searchEpoch.isCurrent(epoch)) recordingSearchBusy.value = false;
  }
}

watch(recordingsQuery, () => {
  window.clearTimeout(recordingSearchTimer);
  recordingSearchTimer = window.setTimeout(() => void runRecordingSearch(), 250);
});

function clearRecordingsSearch() {
  recordingsQuery.value = "";
  recordingHits.value = {};
}

/** 卸载清理：搜索防抖定时器（卸载后不再发起 invoke）。 */
function disposeRecordingSearch() {
  window.clearTimeout(recordingSearchTimer);
}

  return {
    recordingsQuery,
    recordingHits,
    recordingSearchBusy,
    filteredRecordings,
    runRecordingSearch,
    clearRecordingsSearch,
    disposeRecordingSearch,
  };
}
