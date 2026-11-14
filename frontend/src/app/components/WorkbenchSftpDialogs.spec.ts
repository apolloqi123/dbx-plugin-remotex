// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import WorkbenchSftpDialogs from "./WorkbenchSftpDialogs.vue";

/** 删除确认弹窗只依赖 t 与少量状态；其余弹窗用默认关闭态占位。 */
function baseProps() {
  return {
    operationDialog: null,
    operationDraft: "",
    symlinkDialog: null,
    symlinkDraft: "",
    symlinkTargetDraft: "",
    symlinkSubmitting: false,
    watchModifiedQueue: [],
    watchModifiedPrompt: null,
    externalEditBusy: false,
    customEditorOpen: false,
    customEditorName: "",
    customEditorCommand: "",
    customEditorRemember: false,
    customEditorPattern: "",
    chmodTarget: undefined,
    chmodDraft: "",
    chmodSubmitting: false,
    deleteTarget: undefined,
    deleteSubmitting: false,
    batchDeleteOpen: false,
    selectedEntries: [],
    batchProgress: null,
    batchDeleteSubmitting: false,
    downloadPrompt: null,
    localCanSave: false,
    downloadConflictPrompt: null,
    uploadDuplicatePrompt: null,
    uploadDuplicateApplyAll: false,
    newFileDialog: false,
    newFileDraft: "",
    newFileSubmitting: false,
    attrsTarget: undefined,
    attrsLoading: false,
    attrsInfo: undefined,
    attrsMode: "",
    attrsSubmitting: false,
    attrsOwnerDisplay: "",
    canWrite: true,
    permRoles: [],
    permColumns: [],
    permBit: () => false,
    batchProgressPercent: () => 0,
    formatModified: () => "",
    t: (key: string) => key,
  };
}

function mountDialogs(overrides: Record<string, unknown>) {
  return mount(WorkbenchSftpDialogs, {
    props: { ...baseProps(), ...overrides },
    attachTo: document.body,
  });
}

describe("WorkbenchSftpDialogs delete confirmation", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("lists every selected batch entry with its kind label", async () => {
    mountDialogs({
      batchDeleteOpen: true,
      selectedEntries: [
        { name: "alpha.txt", uri: "remote://s/home/alpha.txt", kind: "file" },
        { name: "logs", uri: "remote://s/home/logs", kind: "directory" },
        { name: "latest", uri: "remote://s/home/latest", kind: "symlink" },
      ],
    });
    await nextTick();

    expect(document.body.textContent).toContain("sftpBatch.deleteListTitle");
    const list = document.querySelector(".delete-list");
    // 键盘可达：列表可聚焦并滚动，无障碍名与标题一致。
    expect(list?.getAttribute("tabindex")).toBe("0");
    expect(list?.getAttribute("aria-label")).toBe("sftpBatch.deleteListTitle");
    const rows = Array.from(document.querySelectorAll(".delete-list li"));
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain("alpha.txt");
    expect(rows[0].textContent).toContain("sftpAttrs.kind.file");
    expect(rows[1].textContent).toContain("logs");
    expect(rows[1].textContent).toContain("sftpAttrs.kind.directory");
    expect(rows[2].textContent).toContain("latest");
    expect(rows[2].textContent).toContain("sftpAttrs.kind.symlink");
  });

  it("shows the kind of a single delete target", async () => {
    mountDialogs({
      deleteTarget: { name: "archive", kind: "directory" },
    });
    await nextTick();

    const target = document.querySelector(".delete-target");
    expect(target?.textContent).toContain("archive");
    expect(target?.textContent).toContain("sftpAttrs.kind.directory");
    expect(target?.querySelector("svg")).not.toBeNull();
  });
});
