import { computed, ref, type Ref } from "vue";
import { isSelectableSftpEntry } from "../../features/sftp/lib/sftpContextMenu";
import { decideFileRowAction } from "../../features/sftp/lib/fileRowKeydown";

type AttrEntry = { uri: string; name: string; permissions?: string };
type AttrStat = {
  path?: string;
  kind?: string;
  size?: number;
  modifiedAt?: number;
  mode?: string;
  owner?: string;
  group?: string;
  ownerName?: string | null;
  ownerUid?: number | null;
  groupName?: string | null;
  groupGid?: number | null;
};

/**
 * SFTP 文件属性（stat/chmod）对话框：打开时补一次 stat，权限位矩阵即时改写
 * 草稿，保存走 sudo/chmod 或 sftp/chmod 后重载当前目录。
 */
export function useSftpAttributes(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string, actions?: Array<{ label: string; run: () => void }>) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  sudoMode: Ref<boolean>;
  pathFromUri: (uri: string) => string;
  loadDirectory: () => Promise<void> | void;
  closeFileMenu: () => void;
}) {
  const { t, showNotice, showError, session, sudoMode, pathFromUri, loadDirectory, closeFileMenu } = options;

  const attrsTarget = ref<AttrEntry>();
  const attrsInfo = ref<AttrStat>();
  /** WindTerm 风格属主显示：用户名:组名（uid:gid），名字 = uid 数字时只显示一次。 */
  const attrsOwnerDisplay = computed(() => {
    const ai = attrsInfo.value;
    if (!ai) return "–";
    const name = ai.ownerName || ai.owner || "–";
    const gname = ai.groupName || ai.group || "–";
    const label = `${name}:${gname}`;
    if (ai.ownerUid == null || ai.groupGid == null) return label;
    // 如果 ownerName 为 null 且 owner 就是 uid 数字字符串，不重复括号
    const ownerIsUid = ai.ownerName == null && ai.owner === String(ai.ownerUid);
    const groupIsGid = ai.groupName == null && ai.group === String(ai.groupGid);
    if (ownerIsUid && groupIsGid) return label;
    return `${label} (${ai.ownerUid}:${ai.groupGid})`;
  });
  const attrsLoading = ref(false);
  const attrsMode = ref("");
  const attrsSubmitting = ref(false);

  async function openAttributes(entry: AttrEntry) {
    const sessionId = session.value?.sessionId;
    if (!sessionId) return;
    closeFileMenu();
    attrsTarget.value = entry;
    attrsInfo.value = undefined;
    attrsMode.value = entry.permissions || "";
    attrsLoading.value = true;
    try {
      attrsInfo.value = await window.dbxPlugin.invoke<AttrStat>(sudoMode.value ? "sudo/stat" : "sftp/stat", {
        sessionId,
        path: pathFromUri(entry.uri),
      });
      if (attrsInfo.value?.mode) attrsMode.value = attrsInfo.value.mode;
    } catch (cause) {
      showError(cause);
    } finally {
      attrsLoading.value = false;
    }
  }

  function closeAttributes() {
    attrsTarget.value = undefined;
    attrsInfo.value = undefined;
  }

  async function saveAttributesPermissions() {
    const sessionId = session.value?.sessionId;
    const entry = attrsTarget.value;
    const mode = attrsMode.value.trim();
    if (!sessionId || !entry || !mode || attrsSubmitting.value) return;
    attrsSubmitting.value = true;
    try {
      await window.dbxPlugin.invoke(sudoMode.value ? "sudo/chmod" : "sftp/chmod", {
        sessionId,
        path: pathFromUri(entry.uri),
        mode,
      });
      showNotice(t("permissionsUpdated"));
      if (attrsInfo.value) attrsInfo.value = { ...attrsInfo.value, mode };
      await loadDirectory();
    } catch (cause) {
      showError(cause);
    } finally {
      attrsSubmitting.value = false;
    }
  }

  return {
    attrsTarget,
    attrsInfo,
    attrsOwnerDisplay,
    attrsLoading,
    attrsMode,
    attrsSubmitting,
    openAttributes,
    closeAttributes,
    saveAttributesPermissions,
  };
}
