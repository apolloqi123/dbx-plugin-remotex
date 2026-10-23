// 预览异步组件兜底：加载态、失败态 + 重试重新执行 loader。
// @vitest-environment happy-dom

import { describe, expect, it } from "vitest";
import { defineComponent, nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";
import PreviewPaneFallback from "./PreviewPaneFallback.vue";
import { createPreviewAsyncComponent } from "../lib/asyncPreview";
describe("PreviewPaneFallback", () => {
  it("加载态：显示 spinner 与文案，不渲染重试按钮", () => {
    const wrapper = mount(PreviewPaneFallback, {
      props: { state: "loading", loadingLabel: "Loading preview" },
    });
    expect(wrapper.classes()).toContain("preview-editor");
    expect(wrapper.classes()).toContain("preview-fallback--loading");
    expect(wrapper.attributes("role")).toBe("status");
    expect(wrapper.text()).toContain("Loading preview");
    expect(wrapper.find(".preview-fallback-spinner").exists()).toBe(true);
    expect(wrapper.find(".preview-fallback-retry").exists()).toBe(false);
  });

  it("失败态：显示错误文案与重试按钮，点击上抛 retry", async () => {
    const wrapper = mount(PreviewPaneFallback, {
      props: {
        state: "error",
        errorLabel: "Preview failed to load.",
        retryLabel: "Retry",
      },
    });
    expect(wrapper.classes()).toContain("preview-fallback--error");
    expect(wrapper.attributes("role")).toBe("alert");
    expect(wrapper.text()).toContain("Preview failed to load.");
    const retry = wrapper.get(".preview-fallback-retry");
    expect(retry.text()).toContain("Retry");
    await retry.trigger("click");
    expect(wrapper.emitted("retry")).toHaveLength(1);
  });
});

describe("createPreviewAsyncComponent", () => {
  it("动态 import 失败渲染错误兜底，点重试重新执行 loader 并渲染成功结果", async () => {
    const LoadedPane = defineComponent({ template: "<div class=\"loaded-pane\">preview ready</div>" });
    let attempts = 0;
    const AsyncPreview = createPreviewAsyncComponent(
      () => {
        attempts += 1;
        return attempts === 1 ? Promise.reject(new Error("chunk missing")) : Promise.resolve(LoadedPane);
      },
      () => ({ loading: "Loading preview", failed: "Preview failed to load.", retry: "Retry" }),
    );
    const Host = defineComponent({
      components: { AsyncPreview },
      template: "<AsyncPreview />",
    });

    const wrapper = mount(Host);
    await flushPromises();
    await nextTick();
    expect(attempts).toBe(1);
    expect(wrapper.text()).toContain("Preview failed to load.");
    expect(wrapper.find(".preview-fallback-retry").exists()).toBe(true);

    await wrapper.get(".preview-fallback-retry").trigger("click");
    await flushPromises();
    await nextTick();
    expect(attempts).toBe(2);
    expect(wrapper.find(".loaded-pane").exists()).toBe(true);
    expect(wrapper.text()).toContain("preview ready");
    expect(wrapper.text()).not.toContain("Preview failed to load.");
    wrapper.unmount();
  });

  it("加载成功后把属性/事件原样转发给真实面板", async () => {
    const LoadedPane = defineComponent({
      props: { text: { type: String, default: "" } },
      emits: ["change"],
      template: "<pre class=\"loaded-pane\">{{ text }}</pre>",
    });
    const AsyncPreview = createPreviewAsyncComponent(
      () => Promise.resolve(LoadedPane),
      () => ({ loading: "Loading preview", failed: "Preview failed to load.", retry: "Retry" }),
    );
    const Host = defineComponent({
      components: { AsyncPreview },
      data: () => ({ text: "file body" }),
      template: "<AsyncPreview :text=\"text\" />",
    });
    const wrapper = mount(Host);
    await flushPromises();
    await nextTick();
    expect(wrapper.get(".loaded-pane").text()).toBe("file body");
    wrapper.unmount();
  });
});
