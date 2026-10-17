import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import { h } from "vue";
import Motion from "./Motion.vue";

afterEach(() => document.documentElement.removeAttribute("style"));

describe("Motion", () => {
  it("encerra a transição pela duração dos tokens, antes do compositor", () => {
    document.documentElement.style.setProperty("--motion-in", "180ms");
    document.documentElement.style.setProperty("--motion-out", "0.12s");
    const wrapper = mount(Motion, {
      props: { name: "swap", mode: "out-in" },
      slots: { default: () => h("div", "conteúdo") },
    });
    const transition = wrapper.findComponent({ name: "Transition" });
    expect(transition.props("name")).toBe("swap");
    expect(transition.props("mode")).toBe("out-in");
    expect(transition.props("duration")).toEqual({ enter: 180, leave: 120 });
    expect(wrapper.text()).toContain("conteúdo");
  });

  it("sem tokens (movimento reduzido), troca na hora", () => {
    const wrapper = mount(Motion, {
      props: { name: "slide" },
      slots: { default: () => h("p", "aviso") },
    });
    expect(wrapper.findComponent({ name: "Transition" }).props("duration")).toEqual({ enter: 0, leave: 0 });
  });
});
