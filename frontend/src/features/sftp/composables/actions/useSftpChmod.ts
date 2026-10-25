import { ref } from "vue";
import { pathFromUri } from "../../lib/paths/sftpRemotePaths";

type ChmodEntry = { uri: string; permissions?: string };

// 权限矩阵（所有者/属组/其他人 × 读/写/执行）与八进制草稿双向换算：
// 草稿非法或为空时以 0 为基准，setuid/setgid/sticky 高位原样保留。
export const PERM_ROLES = [
  { who: 6, key: "permOwner" },
  { who: 3, key: "permGroup" },
  { who: 0, key: "permOthers" },
] as const;
export const PERM_COLUMNS = [
  { bit: 4, key: "permRead" },
  { bit: 2, key: "permWrite" },
  { bit: 1, key: "permExec" },
] as const;

export function draftModeBits(raw: string): number {
  const text = raw.trim();
  return /^[0-7]{3,4}$/.test(text) ? parseInt(text, 8) : 0;
}

export function permBit(raw: string, who: number, bit: number): boolean {
  return Boolean(draftModeBits(raw) & (bit << who));
}

export function applyPermBit(raw: string, who: number, bit: number, on: boolean): string {
  const bits = draftModeBits(raw);
  const mode = on ? bits | (bit << who) : bits & ~(bit << who);
  return `0${mode.toString(8)}`;
}

/**
 * SFTP 权限编辑（chmod）对话框状态与提交，以及权限矩阵 ↔ 八进制草稿的
 * 双向换算。属性（attrs）矩阵共用 applyPermBit，但其开关草稿仍由
 * useSftpAttributes 承载，经 attrsMode ref 注入。
 */
export function useSftpChmod<E extends ChmodEntry>(options: {
  canWrite: () => boolean;
  session: () => { sessionId: string } | undefined;
  sudoMode: () => boolean;
  attrsMode: { value: string };
  closeFileMenu: () => void;
  loadDirectory: () => Promise<void>;
  clearSftpDetailCache: () => void;
  showNotice: (message: string) => void;
  showError: (cause: unknown) => void;
  t: (key: string) => string;
}) {
  const {
    canWrite, session, sudoMode, attrsMode, closeFileMenu,
    loadDirectory, clearSftpDetailCache, showNotice, showError, t,
  } = options;

  const chmodTarget = ref<E>();
  const chmodDraft = ref("");
  const chmodSubmitting = ref(false);

  function toggleChmodPerm(who: number, bit: number, event: Event) {
    const input = event.target;
    if (input instanceof HTMLInputElement) chmodDraft.value = applyPermBit(chmodDraft.value, who, bit, input.checked);
  }

  function toggleAttrsPerm(who: number, bit: number, event: Event) {
    const input = event.target;
    if (input instanceof HTMLInputElement) attrsMode.value = applyPermBit(attrsMode.value, who, bit, input.checked);
  }

  function beginChmod(entry: E) {
    if (!canWrite()) return;
    chmodTarget.value = entry;
    chmodDraft.value = entry.permissions || "";
    closeFileMenu();
  }

  async function confirmChmod() {
    const entry = chmodTarget.value;
    const current = session();
    const mode = chmodDraft.value.trim();
    if (!current || !entry || !mode) return;
    chmodSubmitting.value = true;
    try {
      if (sudoMode()) {
        await window.dbxPlugin.invoke("sudo/chmod", {
          sessionId: current.sessionId,
          path: pathFromUri(entry.uri),
          mode,
        });
      } else {
        await window.dbxPlugin.invoke("sftp/chmod", {
          sessionId: current.sessionId,
          path: pathFromUri(entry.uri),
          mode,
        });
      }
      chmodTarget.value = undefined;
      clearSftpDetailCache();
      await loadDirectory();
      showNotice(t("permissionsUpdated"));
    } catch (cause) {
      showError(cause);
    } finally {
      chmodSubmitting.value = false;
    }
  }

  return {
    chmodTarget,
    chmodDraft,
    chmodSubmitting,
    toggleChmodPerm,
    toggleAttrsPerm,
    beginChmod,
    confirmChmod,
  };
}
