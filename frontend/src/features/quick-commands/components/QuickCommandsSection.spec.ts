// @vitest-environment happy-dom
// QuickCommandsSection：树形管理页的分组/搜索/行内建组/编辑器/上下文菜单/导出。
// 菜单是组件自有的 Teleport 单实例（右键与 ⋯ 共用），测试直接驱动 DOM 菜单项。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import QuickCommandsSection from "./QuickCommandsSection.vue";
import { resolveConfirmDialog } from "../../../shared/lib/confirmDialog";
import { workbenchMessage } from "../../../shared/lib/i18n";
import type { QuickCommand, QuickCommandGroup } from "../lib/quickCommands";

const writeClipboardText = vi.hoisted(() => vi.fn(async (_text: string, _deps?: unknown) => "bridge" as const));
vi.mock("../../../shared/lib/clipboardBridge", () => ({ writeClipboardText }));

const t = (key: string, values?: Record<string, string | number>) => workbenchMessage("zh-CN", key, values);

const groups: QuickCommandGroup[] = [
  { id: "ops", name: "运维" },
  { id: "k8s", name: "k8s" },
  { id: "empty", name: "待整理" },
];

const groupedCommands: QuickCommand[] = [
  { id: "q1", name: "磁盘", command: "df -h", groupId: "ops", groupName: "运维" },
  { id: "q2", name: "Pods", command: "kubectl get pods", groupId: "k8s", groupName: "k8s" },
];

const flatCommands: QuickCommand[] = [
  { id: "q1", name: "disk free", command: "df -h" },
  { id: "q2", name: "ports", command: "ss -tlnp" },
];

let active: VueWrapper | null = null;

function mountSection(options: {
  commands?: QuickCommand[];
  groups?: QuickCommandGroup[];
  connected?: boolean;
} = {}): VueWrapper {
  active = mount(QuickCommandsSection, {
    props: {
      commands: options.commands ?? groupedCommands,
      groups: options.groups ?? groups,
      connected: options.connected ?? true,
      saving: false,
      importing: false,
      limit: 200,
      t,
    },
  });
  return active;
}

async function click(wrapper: VueWrapper, selector: string, text?: string) {
  const candidates = wrapper.findAll(selector);
  const target = text ? candidates.find((node) => node.text().includes(text)) : candidates[0];
  expect(target, `点击目标存在: ${selector} ${text ?? ""}`).toBeTruthy();
  await target!.trigger("click");
  await flushPromises();
  return target!;
}

/** 行尾 ⋯ 按钮（命令行前面还有运行按钮）。 */
async function openRowMenu(wrapper: VueWrapper, selector: string, index = 0) {
  const row = wrapper.findAll(selector)[index];
  const buttons = row.findAll(".qc-row-more");
  await buttons[buttons.length - 1].trigger("click");
  await flushPromises();
}

async function openOverflow(wrapper: VueWrapper) {
  const tools = wrapper.findAll(".qc-toolbar .qc-tool");
  await tools[tools.length - 1].trigger("click");
  await flushPromises();
}

function menuItems(wrapper: VueWrapper, kind: "group" | "command" | "overflow"): HTMLButtonElement[] {
  const menu = wrapper.find(`.qc-context-menu[data-menu="${kind}"]`);
  expect(menu.exists(), `菜单已打开: ${kind}`).toBe(true);
  return [...menu.element.querySelectorAll<HTMLButtonElement>("button")];
}

async function clickMenu(wrapper: VueWrapper, kind: "group" | "command" | "overflow", text: string) {
  const target = menuItems(wrapper, kind).find((node) => (node.textContent ?? "").includes(text));
  expect(target, `菜单项存在: ${text}`).toBeTruthy();
  target!.click();
  await flushPromises();
}

afterEach(() => {
  active?.unmount();
  active = null;
  vi.clearAllMocks();
  delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
  document.body.innerHTML = "";
});

describe("QuickCommandsSection tree", () => {
  it("renders groups with counts, including an empty group", () => {
    const wrapper = mountSection();
    const rows = wrapper.findAll(".qc-group-row");
    expect(rows).toHaveLength(3);
    expect(rows[0].text()).toContain("运维");
    expect(rows[0].text()).toContain("(1)");
    expect(rows[1].text()).toContain("k8s");
    expect(rows[2].text()).toContain("待整理");
    expect(rows[2].text()).toContain("(0)");
    expect(wrapper.findAll(".qc-command-row")).toHaveLength(2);
    expect(wrapper.text()).toContain("该分组暂无命令");
  });

  it("filters by command text and keeps group-name matches (even when empty)", async () => {
    const wrapper = mountSection();
    await wrapper.find(".qc-search input").setValue("kubectl");
    expect(wrapper.findAll(".qc-group-row")).toHaveLength(1);
    expect(wrapper.find(".qc-command-row").text()).toContain("Pods");

    await wrapper.find(".qc-search input").setValue("待整理");
    expect(wrapper.findAll(".qc-group-row")).toHaveLength(1);
    expect(wrapper.find(".qc-group-row").text()).toContain("待整理");

    await wrapper.find(".qc-search input").setValue("no-such-command");
    expect(wrapper.find(".qc-empty.compact").text()).toContain("没有匹配");
  });

  it("creates an empty group inline and refuses duplicate names", async () => {
    const wrapper = mountSection();
    const tools = wrapper.findAll(".qc-toolbar .qc-tool");
    await tools[1].trigger("click");
    await flushPromises();
    await wrapper.find(".qc-group-create input").setValue("common");
    await click(wrapper, ".qc-group-create .qc-mini", "创建");
    expect(wrapper.emitted("create-group")![0][0]).toBe("common");

    await tools[1].trigger("click");
    await flushPromises();
    await wrapper.find(".qc-group-create input").setValue("运维");
    await click(wrapper, ".qc-group-create .qc-mini", "创建");
    expect(wrapper.findAll(".qc-group-create")).toHaveLength(1);
    expect(wrapper.find(".qc-error").text()).toContain("已存在同名分组");
    expect(wrapper.emitted("create-group")).toHaveLength(1);
  });

  it("opens the editor pre-filled with the group behind the group menu", async () => {
    const wrapper = mountSection();
    await openRowMenu(wrapper, ".qc-group-row", 0);
    await clickMenu(wrapper, "group", "新建命令");
    expect((wrapper.find(".qc-select").element as HTMLSelectElement).value).toBe("ops");
    await wrapper.find(".quick-command-editor input").setValue("磁盘用量");
    await wrapper.find(".quick-command-editor textarea").setValue("df -h");
    await click(wrapper, ".quick-command-editor-actions .primary-button");
    expect(wrapper.emitted("save")![0][0]).toEqual({ id: undefined, name: "磁盘用量", command: "df -h", groupId: "ops", groupName: "运维" });
  });
});

describe("QuickCommandsSection actions", () => {
  it("runs/pastes from the row actions and the command menu", async () => {
    const wrapper = mountSection();
    const row = wrapper.findAll(".qc-command-row")[0];
    await row.findAll(".qc-command-actions .qc-row-more")[0].trigger("click");
    expect(wrapper.emitted("run")![0][0]).toMatchObject({ id: "q1" });

    await openRowMenu(wrapper, ".qc-command-row", 0);
    await clickMenu(wrapper, "command", "粘贴");
    expect(wrapper.emitted("paste")![0][0]).toMatchObject({ id: "q1" });
  });

  it("disables run/paste when the session is disconnected", async () => {
    const wrapper = mountSection({ connected: false });
    const play = wrapper.findAll(".qc-command-row")[0].findAll(".qc-command-actions .qc-row-more")[0];
    expect(play.attributes("disabled")).toBeDefined();
    await play.trigger("click");
    expect(wrapper.emitted("run")).toBeUndefined();

    await openRowMenu(wrapper, ".qc-command-row", 0);
    const runItem = menuItems(wrapper, "command").find((node) => (node.textContent ?? "").includes("运行"));
    expect(runItem?.disabled).toBe(true);
  });

  it("deletes a command after the confirm gate", async () => {
    const wrapper = mountSection();
    await openRowMenu(wrapper, ".qc-command-row", 0);
    await clickMenu(wrapper, "command", "删除");
    expect(wrapper.emitted("delete")).toBeUndefined();
    resolveConfirmDialog(false);
    await flushPromises();
    expect(wrapper.emitted("delete")).toBeUndefined();

    await openRowMenu(wrapper, ".qc-command-row", 0);
    await clickMenu(wrapper, "command", "删除");
    resolveConfirmDialog(true);
    await flushPromises();
    expect(wrapper.emitted("delete")![0][0]).toBe("q1");
  });

  it("renames and dissolves a group from its context menu", async () => {
    const wrapper = mountSection();
    await openRowMenu(wrapper, ".qc-group-row", 0);
    await clickMenu(wrapper, "group", "编辑");
    await wrapper.find(".qc-group-rename input").setValue("SRE");
    await click(wrapper, ".qc-group-rename .qc-mini", "保存");
    expect(wrapper.emitted("rename-group")![0][0]).toEqual({ groupId: "ops", name: "SRE" });

    await openRowMenu(wrapper, ".qc-group-row", 0);
    await clickMenu(wrapper, "group", "删除");
    resolveConfirmDialog(true);
    await flushPromises();
    expect(wrapper.emitted("dissolve-group")![0][0]).toBe("ops");
  });

  it("copies and exports the selected command/all via the overflow menu", async () => {
    const wrapper = mountSection();
    await wrapper.findAll(".qc-command-row")[0].trigger("click");
    await openOverflow(wrapper);
    await clickMenu(wrapper, "overflow", "导出当前命令到剪贴板");
    expect(writeClipboardText).toHaveBeenCalledWith(
      JSON.stringify([{ name: "磁盘", command: "df -h", group: "运维" }], null, 2),
      expect.anything(),
    );
    expect(wrapper.emitted("notice")![0][0]).toContain("已复制 1 条命令");

    await openOverflow(wrapper);
    await clickMenu(wrapper, "overflow", "导出全部到剪贴板");
    expect(JSON.parse(writeClipboardText.mock.calls[1][0])).toHaveLength(2);
  });

  it("uses the host save bridge for file export", async () => {
    const saveFile = vi.fn(async () => ({ path: "/tmp/quick-commands.json" }));
    (window as unknown as { dbxPlugin: unknown }).dbxPlugin = { saveFile };
    const wrapper = mountSection();
    await openOverflow(wrapper);
    await clickMenu(wrapper, "overflow", "导出全部到文件");
    expect(saveFile).toHaveBeenCalledWith(
      expect.objectContaining({ fileName: expect.stringMatching(/^sshx-quick-commands-.*\.json$/), contentType: "application/json" }),
      expect.any(String),
    );
    expect(wrapper.emitted("notice")![0][0]).toContain("/tmp/quick-commands.json");
  });

  it("opens the context menu from right-click at the pointer position", async () => {
    const wrapper = mountSection();
    const row = wrapper.findAll(".qc-command-row")[1];
    await row.trigger("contextmenu", { clientX: 42, clientY: 64 });
    await flushPromises();
    const menu = wrapper.find('.qc-context-menu[data-menu="command"]');
    expect(menu.exists()).toBe(true);
    const el = menu.element as HTMLElement;
    expect(el.style.left).toBe("42px");
    expect(el.style.top).toBe("64px");
  });
});

describe("QuickCommandsSection editor/import", () => {
  it("creates a command through the editor view and emits the trimmed payload", async () => {
    const wrapper = mountSection({ commands: flatCommands, groups: [] });
    await click(wrapper, ".quick-manage-actions .link-button", "新建");
    const name = wrapper.find(".quick-command-editor input");
    const command = wrapper.find(".quick-command-editor textarea");
    await name.setValue("uptime");
    await command.setValue("  uptime  ");
    await wrapper.find(".quick-command-editor-actions .primary-button").trigger("click");
    expect(wrapper.emitted("save")![0][0]).toEqual({ id: undefined, name: "uptime", command: "uptime" });
    await wrapper.setProps({ commands: [...flatCommands, { id: "q3", name: "uptime", command: "uptime" }] });
    expect(wrapper.find(".quick-command-editor").exists()).toBe(false);
  });

  it("previews pasted JSON and emits only accepted items (with groups) on import", async () => {
    const wrapper = mountSection({ commands: flatCommands, groups: [] });
    await click(wrapper, ".quick-manage-actions .link-button", "导入");
    const textarea = wrapper.find(".quick-command-editor textarea");
    await textarea.setValue(JSON.stringify([{ name: "disk free", command: "df -h" }, { name: "who", command: "who", group: "运维" }]));
    await flushPromises();
    expect(wrapper.text()).toContain("可导入 1 条");
    expect(wrapper.text()).toContain("跳过同名 1");
    await click(wrapper, ".quick-command-editor-actions .primary-button");
    expect(wrapper.emitted("import")![0][0]).toEqual([{ name: "who", command: "who", groupName: "运维" }]);
    await wrapper.setProps({ commands: [...flatCommands, { id: "q3", name: "who", command: "who" }] });
    expect(wrapper.find(".quick-import-file").exists()).toBe(false);
  });
});
