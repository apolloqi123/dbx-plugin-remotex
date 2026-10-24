// 全局 quick sudo 配置档：列表 + 编辑表单状态 + CRUD。从 SettingsDialog.vue
// 抽出——设置弹窗内联 section 与独立配置档弹窗（ProfilesManagerDialog）共用
// 同一份状态，密钥只在提交时发送。主「保存」会串行提交未保存的 profile 编辑，
// 因此 editing/draft/error 与 Esc 内联分层退出链都由本组合式持有。
import { computed, reactive, watch } from "vue";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { rpc } from "../../../shared/lib/rpc";
import { settingsErrorOf, type SudoProfileView } from "../lib/settingsModel";

type Translate = (key: string, values?: Record<string, string | number>) => string;

/** 认证流程枚举 → i18n key（未知值回退「密码后 OTP」）。 */
export function flowModeLabel(t: Translate, mode: string) {
  if (mode === "off") return t("flowOff");
  if (mode === "password_only") return t("flowOnly");
  if (mode === "password_plus_otp") return t("flowPlusOtp");
  return t("flowThenOtp");
}

/** 配置档只读摘要（内联列表与独立弹窗共用）。 */
export function profileSummary(t: Translate, profile: SudoProfileView) {
  return [
    `${t("settingsSudoPassword")}: ${profile.sudoPasswordSet ? t("settingsConfigured") : "—"}`,
    `${t("settingsTotp")}: ${profile.totpConfigured ? t("settingsConfigured") : "—"}`,
    t("settingsFlowMode") + ": " + flowModeLabel(t, profile.authFlowMode),
    profile.sudoUsePty ? t("settingsUsePty") : "",
  ].filter(Boolean).join(" · ");
}

export function useSudoProfiles(options: {
  t: Translate;
  settingsDraft: { quickSudoProfileId: string };
  isProfilesOpen: () => boolean;
  refreshSettingsMeta: () => Promise<void>;
  emitNotice: (message: string) => void;
}) {
  const state = reactive({
    profiles: [] as SudoProfileView[],
    loading: false,
    error: "",
    /** 设置弹窗内联 section 的展开态（Esc 分层退出第一层）。 */
    inlineOpen: false,
    /** 编辑表单是否打开（Esc 分层退出最内层；主保存串行提交）。 */
    editing: false,
    saving: false,
    hadPassword: false,
    hadTotp: false,
    draft: {
      id: "",
      name: "",
      sudoPassword: "",
      totpSecret: "",
      authFlowMode: "password_then_otp",
      passwordPromptHint: "",
      totpPromptHint: "",
      sudoUsePty: false,
    },
  });

  const boundProfile = computed(
    () => state.profiles.find((profile) => profile.id === options.settingsDraft.quickSudoProfileId),
  );

  watch(() => options.isProfilesOpen(), (open) => {
    if (!open) return;
    state.editing = false;
    resetProfileDraft();
    void loadSudoProfiles();
  });

  async function loadSudoProfiles() {
    state.loading = true;
    state.error = "";
    try {
      const result = await rpc<{ profiles: SudoProfileView[] }>("sudo/profiles/list", {});
      state.profiles = result.profiles;
    } catch (cause) {
      state.profiles = [];
      state.error = settingsErrorOf(cause);
    } finally {
      state.loading = false;
    }
  }

  function resetProfileDraft() {
    state.draft.id = "";
    state.draft.name = "";
    state.draft.sudoPassword = "";
    state.draft.totpSecret = "";
    state.draft.authFlowMode = "password_then_otp";
    state.draft.passwordPromptHint = "";
    state.draft.totpPromptHint = "";
    state.draft.sudoUsePty = false;
    state.hadPassword = false;
    state.hadTotp = false;
  }

  function startProfileCreate() {
    resetProfileDraft();
    state.editing = true;
  }

  function startProfileEdit(profile: SudoProfileView) {
    resetProfileDraft();
    state.draft.id = profile.id;
    state.draft.name = profile.name;
    state.draft.authFlowMode = profile.authFlowMode || "password_then_otp";
    state.draft.passwordPromptHint = profile.passwordPromptHint || "";
    state.draft.totpPromptHint = profile.totpPromptHint || "";
    state.draft.sudoUsePty = profile.sudoUsePty;
    state.hadPassword = profile.sudoPasswordSet;
    state.hadTotp = profile.totpConfigured;
    state.editing = true;
    // 回显已存原值供编辑（工作台专用 reveal 方法；失败保持占位提示）。
    if (profile.sudoPasswordSet || profile.totpConfigured) {
      const editingId = profile.id;
      void rpc<{ profile: { sudoPassword?: string; totpSecret?: string } }>("sudo/profiles/reveal", { id: editingId })
        .then((revealed) => {
          if (state.editing && state.draft.id === editingId) {
            state.draft.sudoPassword = revealed.profile?.sudoPassword || "";
            state.draft.totpSecret = revealed.profile?.totpSecret || "";
          }
        })
        .catch(() => undefined);
    }
  }

  async function saveProfileDraft() {
    if (state.saving) return;
    const name = state.draft.name.trim();
    if (!name) {
      state.error = options.t("profilesNameRequired");
      return;
    }
    state.saving = true;
    state.error = "";
    try {
      const payload: Record<string, unknown> = {
        authFlowMode: state.draft.authFlowMode,
        passwordPromptHint: state.draft.passwordPromptHint,
        totpPromptHint: state.draft.totpPromptHint,
        sudoUsePty: state.draft.sudoUsePty,
      };
      if (state.draft.id) payload.id = state.draft.id;
      payload.name = name;
      if (state.draft.sudoPassword) payload.sudoPassword = state.draft.sudoPassword;
      if (state.draft.totpSecret.trim()) payload.totpSecret = state.draft.totpSecret;
      await rpc("sudo/profiles/save", payload);
      state.editing = false;
      resetProfileDraft();
      await loadSudoProfiles();
      await options.refreshSettingsMeta();
      options.emitNotice(options.t("profilesSaved"));
    } catch (cause) {
      state.error = settingsErrorOf(cause);
    } finally {
      state.saving = false;
    }
  }

  async function removeProfile(profile: SudoProfileView) {
    if (!(await confirmDialog(options.t("profilesDeleteConfirm", { name: profile.name })))) return;

    try {
      await rpc("sudo/profiles/delete", { id: profile.id });
      if (options.settingsDraft.quickSudoProfileId === profile.id) options.settingsDraft.quickSudoProfileId = "";
      await loadSudoProfiles();
      await options.refreshSettingsMeta();
      options.emitNotice(options.t("profilesDeleted"));
    } catch (cause) {
      state.error = settingsErrorOf(cause);
    }
  }

  /// 取消内联 profile 编辑：关表单并清空草稿/错误（独立 profiles 弹窗、
  /// 设置弹窗内联 section 与 Esc 关闭链共用同一语义）。
  function cancelProfileEdit() {
    state.editing = false;
    resetProfileDraft();
    state.error = "";
  }

  return {
    state,
    // 控制器以普通对象跨组件传递：getter 保证模板拿到的是当前值而非 ComputedRef。
    get boundProfile() {
      return boundProfile.value;
    },
    loadSudoProfiles,
    resetProfileDraft,
    startProfileCreate,
    startProfileEdit,
    saveProfileDraft,
    removeProfile,
    cancelProfileEdit,
  };
}

export type SudoProfilesController = ReturnType<typeof useSudoProfiles>;
