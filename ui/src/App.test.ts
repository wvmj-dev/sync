import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.vue";
import { mountApp } from "./testing";

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    api: {
      ...actual.api,
      appInfo: vi.fn(async () => ({ version: "0.1.0" })),
      listAccounts: vi.fn(async () => []),
      listJobs: vi.fn(async () => []),
    },
  };
});

beforeEach(() => localStorage.clear());

describe("menu lateral", () => {
  it("recolhe, lembra a escolha e mantém os nomes para leitores de tela", async () => {
    const wrapper = mountApp(App);
    await flushPromises();
    const toggle = () => wrapper.find("button.collapse-toggle");
    expect(wrapper.find(".shell").classes()).not.toContain("collapsed");
    expect(toggle().attributes("aria-expanded")).toBe("true");

    await toggle().trigger("click");
    expect(wrapper.find(".shell").classes()).toContain("collapsed");
    expect(toggle().attributes("aria-expanded")).toBe("false");
    // Os nomes continuam no HTML (escondidos só visualmente) e viram dica ao apontar.
    expect(wrapper.find("nav").text()).toContain("Preferências");
    expect(wrapper.find('a[title="Tarefas"]').exists()).toBe(true);

    // Abrir o app de novo: continua recolhido.
    const again = mountApp(App);
    await flushPromises();
    expect(again.find(".shell").classes()).toContain("collapsed");
    await again.find("button.collapse-toggle").trigger("click");
    expect(localStorage.getItem("syncly.sidebar.collapsed")).toBe("0");
  });
});
