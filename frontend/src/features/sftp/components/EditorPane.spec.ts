// @vitest-environment happy-dom
// EditorPane 组件测试：页签条渲染/无障碍属性、激活/关闭/保存事件、脏点、
// 文本 change 上抛、图片缩放、加载/错误态。异步 TextPreview 以同步桩替换
// （动态 import 仍走真实 createPreviewAsyncComponent 包装）。
import { describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import EditorPane from "./EditorPane.vue";
import type { EditorTab } from "../composables/useEditorTabs";

vi.mock("./TextPreview.vue", async () => {
  const { defineComponent, h } = await import("vue");
  return {
    default: defineComponent({
      name: "TextPreviewStub",
      props: {
        text: { type: String, default: "" },
        fileName: { type: String, default: "" },
        editable: { type: Boolean, default: false },
        appearance: { type: Object, default: undefined },
      },
      emits: ["change"],
      setup(props, { emit }) {
        return () =>
          h("textarea", {
            class: "text-preview-stub",
            value: props.text,
            readonly: !props.editable,
            onInput: (event: Event) => emit("change", (event.target as HTMLTextAreaElement).value),
          });
      },
    }),
  };
});

function appearance(): DbxPluginAppearance {
  return {
    colorScheme: "light",
    colors: {
      background: "#ffffff",
      foreground: "#111111",
      muted: "#f0f0f0",
      mutedForeground: "#777777",
      accent: "#e0e0ff",
      accentForeground: "#000000",
      border: "#cccccc",
      destructive: "#cc0000",
    },
    terminal: { fontFamily: "monospace", fontSize: 12 },
  };
}

function tab(overrides: Partial<EditorTab> = {}): EditorTab {
  return {
    id: "t1",
    path: "/home/notes.txt",
    name: "notes.txt",
    kind: "text",
    text: "hello",
    baseline: "hello",
    dirty: false,
    editable: true,
    truncated: false,
    loading: false,
    saving: false,
    size: 5,
    ...overrides,
  };
}

async function mountPane(tabs: EditorTab[], activeId = tabs[0]?.id ?? "") {
  const wrapper = mount(EditorPane, {
    props: {
      tabs,
      activeId,
      t: (key: string) => key,
      locale: "en",
      appearance: appearance(),
    },
  });
  await flushPromises();
  await nextTick();
  return wrapper;
}

describe("EditorPane", () => {
  it("渲染页签条：角色、选中态、名称、脏点与关闭按钮", async () => {
    const wrapper = await mountPane([
      tab({ id: "t1", name: "a.txt" }),
      tab({
        id: "t2",
        name: "b.png",
        kind: "image",
        text: "",
        baseline: "",
        dirty: true,
        editable: false,
        imageUrl: "data:image/png;base64,aGk=",
      }),
    ], "t2");

    expect(wrapper.get(".editor-tabs").attributes("role")).toBe("tablist");
    const rendered = wrapper.findAll('[role="tab"]');
    expect(rendered).toHaveLength(2);
    expect(rendered[0].attributes("aria-selected")).toBe("false");
    expect(rendered[1].attributes("aria-selected")).toBe("true");
    expect(rendered[0].text()).toContain("a.txt");
    expect(rendered[1].text()).toContain("b.png");
    expect(wrapper.findAll(".editor-tab-icon")).toHaveLength(2);
    expect(wrapper.findAll(".editor-tab-close")).toHaveLength(2);
    expect(wrapper.findAll(".editor-tab-close")[0].attributes("title")).toBe("close");
    const dirty = wrapper.get(".editor-tab-dirty");
    expect(dirty.attributes("title")).toBe("editSave.unsaved");
    expect(rendered[1].find(".editor-tab-dirty").exists()).toBe(true);
  });

  it("激活页签、关闭页签、保存页签分别上抛事件", async () => {
    const wrapper = await mountPane([
      tab({ id: "t1", dirty: true }),
      tab({ id: "t2", path: "/home/other.txt", name: "other.txt" }),
    ], "t1");

    await wrapper.findAll('[role="tab"]')[1].trigger("click");
    expect(wrapper.emitted("activate")).toEqual([["t2"]]);

    await wrapper.findAll(".editor-tab-close")[1].trigger("click");
    expect(wrapper.emitted("close")).toEqual([["t2"]]);

    const save = wrapper.get(".editor-save");
    expect(save.attributes("disabled")).toBeUndefined();
    await save.trigger("click");
    expect(wrapper.emitted("save")).toEqual([["t1"]]);
  });

  it("保存按钮在干净/保存中禁用，且只对可编辑文本页签出现", async () => {
    const clean = await mountPane([tab({ dirty: false })]);
    expect(clean.get(".editor-save").attributes("disabled")).toBeDefined();

    const saving = await mountPane([tab({ dirty: true, saving: true })]);
    expect(saving.get(".editor-save").attributes("disabled")).toBeDefined();
    expect(saving.find(".editor-save .spinning").exists()).toBe(true);

    const readOnly = await mountPane([tab({ editable: false })]);
    expect(readOnly.find(".editor-save").exists()).toBe(false);

    const image = await mountPane([
      tab({ kind: "image", editable: false, text: "", baseline: "", imageUrl: "data:image/png;base64,aGk=" }),
    ]);
    expect(image.find(".editor-save").exists()).toBe(false);
  });

  it("文本编辑通过 update:text 上抛当前页签 id 与全文", async () => {
    const wrapper = await mountPane([tab({ id: "t9", text: "before" })]);
    const editor = wrapper.get(".text-preview-stub");
    expect((editor.element as HTMLTextAreaElement).value).toBe("before");

    await editor.setValue("after");

    expect(wrapper.emitted("update:text")).toEqual([[{ id: "t9", text: "after" }]]);
  });

  it("图片页签渲染 img 并支持点击缩放切换", async () => {
    const wrapper = await mountPane([
      tab({
        id: "img",
        name: "photo.png",
        kind: "image",
        text: "",
        baseline: "",
        editable: false,
        imageUrl: "data:image/png;base64,aGk=",
      }),
    ]);

    const image = wrapper.get("img.editor-image");
    expect(image.attributes("src")).toBe("data:image/png;base64,aGk=");
    expect(image.attributes("alt")).toBe("photo.png");
    expect(image.attributes("title")).toBe("imagePreview.zoomIn");
    expect(image.classes()).not.toContain("is-zoomed");

    await image.trigger("click");
    expect(image.classes()).toContain("is-zoomed");
    expect(image.attributes("title")).toBe("imagePreview.zoomOut");
  });

  it("加载态显示 loading，不渲染正文", async () => {
    const wrapper = await mountPane([tab({ loading: true, text: "" })]);

    const state = wrapper.get(".editor-state");
    expect(state.attributes("role")).toBe("status");
    expect(state.text()).toContain("loading");
    expect(state.find(".spinning").exists()).toBe(true);
    expect(wrapper.find(".text-preview-stub").exists()).toBe(false);
    expect(wrapper.find("img.editor-image").exists()).toBe(false);
  });

  it("错误态展示页签错误信息，不渲染正文", async () => {
    const wrapper = await mountPane([tab({ error: "read failed", text: "" })]);

    const state = wrapper.get(".editor-state--error");
    expect(state.attributes("role")).toBe("alert");
    expect(state.text()).toContain("read failed");
    expect(wrapper.find(".text-preview-stub").exists()).toBe(false);
  });
});
