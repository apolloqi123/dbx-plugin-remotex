// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { nextTick, ref, shallowRef } from "vue";
import { trackExposedRef } from "./trackExposedRef";

describe("trackExposedRef", () => {
  it("mirrors later changes of the exposed ref", async () => {
    const exposed = ref<HTMLElement>();
    const target = shallowRef<HTMLElement>();
    const stop = trackExposedRef(() => exposed.value, target);

    expect(target.value).toBeUndefined();

    const node = document.createElement("section");
    exposed.value = node;
    await nextTick();
    expect(target.value).toBe(node);

    exposed.value = undefined;
    await nextTick();
    expect(target.value).toBeUndefined();

    stop();
  });

  it("stops mirroring after stop", async () => {
    const exposed = ref<HTMLElement>();
    const target = shallowRef<HTMLElement>();
    const stop = trackExposedRef(() => exposed.value, target);
    stop();

    exposed.value = document.createElement("section");
    await nextTick();
    expect(target.value).toBeUndefined();
  });
});
