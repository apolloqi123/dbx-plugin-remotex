// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent, h, nextTick, onBeforeUnmount, onMounted } from "vue";
import TerminalLayoutTree from "./TerminalLayoutTree.vue";
import { addTerminalTab, createTerminalLayout, splitTerminalLayout, type TerminalLayoutNode } from "../lib/appearance/terminalLayout";

const lifecycle = { mounts: 0, unmounts: 0 };

const StubPane = defineComponent({
  name: "SplitTerminalPane",
  inheritAttrs: false,
  emits: ["cwd", "opened", "closed", "error", "new-tab", "split", "search-online"],
  setup(_, { emit }) {
    onMounted(() => {
      lifecycle.mounts += 1;
    });
    onBeforeUnmount(() => {
      lifecycle.unmounts += 1;
    });
    return () => h("button", { class: "emit-cwd", onClick: () => emit("cwd", "/var/log") }, "emit");
  },
});

function mountTree(node: TerminalLayoutNode = createTerminalLayout()) {
  return mount(TerminalLayoutTree, {
    props: {
      node,
      connectionId: "c", workbenchId: "w", mainSessionId: "m",
      theme: {}, fontFamily: "mono", fontSize: 13,
      t: (k: string) => k, locale: "en",
      canSplit: true, canCreateTab: true, engines: [],
      connected: true, canWrite: true, transferBusy: false,
    },
    global: { stubs: { SplitTerminalPane: StubPane } },
    slots: { main: "<div class='main-slot'>main</div>" },
  });
}

/** 宿主就位（placedTabIds 更新）后再挂载终端，需要等两次渲染。 */
async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await nextTick();
}

beforeEach(() => {
  lifecycle.mounts = 0;
  lifecycle.unmounts = 0;
});

describe("TerminalLayoutTree split pane lifecycle", () => {
  it("forwards a split tab cwd with its tab id", async () => {
    const layout = splitTerminalLayout(createTerminalLayout(), "main", "row", "B", "tabB");
    const wrapper = mountTree(layout);
    await settle();
    const buttons = wrapper.findAll(".emit-cwd");
    expect(buttons.length).toBe(1);
    await buttons[0].trigger("click");
    expect(wrapper.emitted("cwd")).toEqual([["tabB", "/var/log"]]);
    wrapper.unmount();
  });

  it("keeps existing split terminals mounted when the layout splits again", async () => {
    const initial = addTerminalTab(createTerminalLayout(), "main", "tabA");
    const wrapper = mountTree(initial);
    await settle();
    expect(lifecycle.mounts).toBe(1);

    // 主窗格再次分屏：tabA 的终端必须原地存活，只新增 tabB。
    await wrapper.setProps({ node: splitTerminalLayout(initial, "main", "row", "B", "tabB") });
    await settle();
    expect(lifecycle.unmounts).toBe(0);
    expect(lifecycle.mounts).toBe(2);

    // 在分屏 B 里继续分屏：A/B 都不重建，只新增 C。
    const split = splitTerminalLayout(initial, "main", "row", "B", "tabB");
    await wrapper.setProps({ node: splitTerminalLayout(split, "B", "column", "C", "tabC") });
    await settle();
    expect(lifecycle.unmounts).toBe(0);
    expect(lifecycle.mounts).toBe(3);
    expect(wrapper.findAll(".emit-cwd").length).toBe(3);
    wrapper.unmount();
  });

  it("unmounts a split terminal only when its tab leaves the layout", async () => {
    const initial = addTerminalTab(createTerminalLayout(), "main", "tabA");
    const wrapper = mountTree(initial);
    await settle();
    expect(lifecycle.mounts).toBe(1);

    // 关闭整个主窗格之外的新分屏（移除 B）→ 只卸载 tabB。
    const split = splitTerminalLayout(initial, "main", "row", "B", "tabB");
    await wrapper.setProps({ node: split });
    await settle();
    expect(lifecycle.mounts).toBe(2);

    await wrapper.setProps({ node: initial });
    await settle();
    expect(lifecycle.unmounts).toBe(1);
    expect(wrapper.findAll(".emit-cwd").length).toBe(1);
    wrapper.unmount();
  });
});
