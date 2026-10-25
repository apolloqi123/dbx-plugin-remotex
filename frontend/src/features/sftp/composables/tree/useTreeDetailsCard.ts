import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { SftpSanitizedEntry } from "../../lib/entries/sftpEntries";
import type { SftpFileTreeNode, SftpTreeRow } from "../../lib/tree/sftpFileTree";
import {
  mergeSftpDetail,
  sftpDetailFromEntry,
  sftpDetailRows,
  type SftpDetailInfo,
  type SftpDetailRow,
} from "../../lib/actions/sftpDetails";

const DETAIL_HOVER_DELAY = 250;
const CARD_FALLBACK_WIDTH = 276;
const CARD_FALLBACK_HEIGHT = 196;
const CARD_GAP = 6;
const VIEWPORT_MARGIN = 4;

interface CachedDetail {
  modifiedAt: number | undefined;
  /** chmod 不一定更新 mtime：权限串变化也必须让缓存失效。 */
  permissions: string | undefined;
  info: SftpDetailInfo;
  unavailable: boolean;
}

/** Info 详情卡：每个行尾的 info 图标在 hover/focus 250ms 后打开 fixed 卡片。
 *  loadDetails 只在缓存缺失（或 entry.modifiedAt 变化）时请求一次；成功后按
 *  uri 缓存，失败/返回 null 也缓存，卡片退回 entry 自带字段并标注 unavailable。
 *  卡片本身由父级 Teleport 到 body 渲染（保持父级 scoped CSS 命中）。 */
export function useTreeDetailsCard(options: {
  rows: () => SftpTreeRow[];
  t: () => (key: string, values?: Record<string, string | number>) => string;
  locale: () => string | undefined;
  loadDetails: () => ((entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>) | undefined;
  /** 行展示名（根节点为 "/"），标题沿用树内的同一套名称。 */
  displayName: (node: SftpFileTreeNode) => string;
}) {
  const { rows, t, locale, loadDetails, displayName } = options;

  const detailVisible = ref(false);
  const detailNode = ref<SftpFileTreeNode | null>(null);
  const detailInfo = ref<SftpDetailInfo | null>(null);
  const detailLoading = ref(false);
  const detailUnavailable = ref(false);
  const detailAnchor = ref<HTMLElement | null>(null);
  const detailCardEl = ref<HTMLElement | null>(null);
  const detailCardStyle = ref<Record<string, string>>({});

  const detailCache = new Map<string, CachedDetail>();
  const detailInflight = new Map<string, Promise<Partial<SftpDetailInfo> | null>>();
  let detailHoverTimer: ReturnType<typeof setTimeout> | undefined;
  let detailToken = 0;
  let detailShownUri = "";

  // 树刷新/过滤/展开会重排行，锚元素可能被替换：直接收起，避免卡片悬空。
  watch(
    () => rows(),
    () => {
      if (detailVisible.value) hideDetails();
    },
  );

  const detailTitle = computed(() =>
    detailNode.value ? displayName(detailNode.value) : t()("sftpDetails.title"),
  );

  const detailRows = computed<SftpDetailRow[]>(() =>
    detailInfo.value ? sftpDetailRows(detailInfo.value, t(), locale() || "") : [],
  );

  function cancelDetailTimer() {
    if (detailHoverTimer === undefined) return;
    clearTimeout(detailHoverTimer);
    detailHoverTimer = undefined;
  }

  function hideDetails() {
    cancelDetailTimer();
    detailVisible.value = false;
    detailLoading.value = false;
    detailToken += 1;
    detailShownUri = "";
  }

  function scheduleDetails(node: SftpFileTreeNode, anchor: HTMLElement | null) {
    cancelDetailTimer();
    if (!anchor) return;
    detailAnchor.value = anchor;
    detailHoverTimer = setTimeout(() => {
      detailHoverTimer = undefined;
      void openDetails(node);
    }, DETAIL_HOVER_DELAY);
  }

  function onInfoEnter(node: SftpFileTreeNode, event: MouseEvent) {
    scheduleDetails(node, event.currentTarget as HTMLElement | null);
  }

  function onInfoFocus(node: SftpFileTreeNode, event: FocusEvent) {
    scheduleDetails(node, event.currentTarget as HTMLElement | null);
  }

  function positionDetailCard() {
    const anchor = detailAnchor.value;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const width = detailCardEl.value?.offsetWidth || CARD_FALLBACK_WIDTH;
    const height = detailCardEl.value?.offsetHeight || CARD_FALLBACK_HEIGHT;
    const viewportWidth = window.innerWidth || document.documentElement.clientWidth || width;
    const viewportHeight = window.innerHeight || document.documentElement.clientHeight || height;

    // 默认贴图标右侧；右侧放不下翻到左侧，最后统一夹回视口（含 4px 边距）。
    let left = rect.right + CARD_GAP;
    if (left + width > viewportWidth - VIEWPORT_MARGIN) left = rect.left - width - CARD_GAP;
    left = Math.min(left, viewportWidth - width - VIEWPORT_MARGIN);
    left = Math.max(VIEWPORT_MARGIN, left);

    let top = rect.top;
    top = Math.min(top, viewportHeight - height - VIEWPORT_MARGIN);
    top = Math.max(VIEWPORT_MARGIN, top);

    detailCardStyle.value = {
      position: "fixed",
      left: `${Math.round(left)}px`,
      top: `${Math.round(top)}px`,
    };
  }

  async function openDetails(node: SftpFileTreeNode) {
    const entry = node.entry;
    const uri = entry.uri;
    const token = ++detailToken;
    detailShownUri = uri;
    detailNode.value = node;
    detailVisible.value = true;
    detailUnavailable.value = false;

    const cached = detailCache.get(uri);
    if (cached && cached.modifiedAt === entry.modifiedAt && cached.permissions === entry.permissions) {
      detailInfo.value = mergeSftpDetail(entry, cached.info, node.path);
      detailLoading.value = false;
      detailUnavailable.value = cached.unavailable;
      await nextTick();
      positionDetailCard();
      return;
    }

    const request = loadDetails();
    if (!request) {
      detailInfo.value = sftpDetailFromEntry(entry, node.path);
      detailLoading.value = false;
      await nextTick();
      positionDetailCard();
      return;
    }

    // 加载期间先展示 entry 上已有的字段，再加一行 loading，卡片不会先空一拍。
    detailInfo.value = sftpDetailFromEntry(entry, node.path);
    detailLoading.value = true;
    await nextTick();
    positionDetailCard();

    const inflightKey = `${uri}\u0000${entry.modifiedAt ?? ""}\u0000${entry.permissions ?? ""}`;
    let inflight = detailInflight.get(inflightKey);
    if (!inflight) {
      inflight = Promise.resolve()
        .then(() => request(entry))
        .catch(() => null);
      detailInflight.set(inflightKey, inflight);
      void inflight.finally(() => {
        if (detailInflight.get(inflightKey) === inflight) detailInflight.delete(inflightKey);
      });
    }

    const stat = await inflight;
    const merged = mergeSftpDetail(entry, stat, node.path);
    detailCache.set(uri, {
      modifiedAt: entry.modifiedAt,
      permissions: entry.permissions,
      info: merged,
      unavailable: stat == null,
    });
    // 卡片可能已关闭/换了目标：缓存照写，UI 只在当前 token 仍有效时更新。
    if (token !== detailToken || !detailVisible.value || detailShownUri !== uri) return;
    detailInfo.value = merged;
    detailLoading.value = false;
    detailUnavailable.value = stat == null;
    await nextTick();
    positionDetailCard();
  }

  onBeforeUnmount(hideDetails);

  return {
    detailVisible,
    detailNode,
    detailInfo,
    detailLoading,
    detailUnavailable,
    detailCardEl,
    detailCardStyle,
    detailTitle,
    detailRows,
    hideDetails,
    onInfoEnter,
    onInfoFocus,
    positionDetailCard,
  };
}
