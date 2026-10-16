import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Preferences } from "../api/types";
import { mountApp } from "../testing";
import PreferencesView from "./PreferencesView.vue";

vi.mock("../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api")>();
  return {
    ...actual,
    api: { ...actual.api, getPreferences: vi.fn(), savePreferences: vi.fn() },
  };
});

const stored: Preferences = { notify: "failures", minFreeSpaceMib: 1024, maxParallelTransfers: 4 };

beforeEach(() => {
  vi.mocked(api.getPreferences).mockResolvedValue(stored);
  vi.mocked(api.savePreferences).mockImplementation(async (p) => p);
});

describe("Preferências (RN-94, RN-46)", () => {
  async function mounted() {
    const wrapper = mountApp(PreferencesView);
    await flushPromises();
    const [parallel, reserve] = wrapper.findAll('input[type="number"]');
    const save = wrapper.findAll("button").find((b) => b.text() === "Salvar");
    if (!parallel || !reserve || !save) throw new Error("campos não encontrados");
    return { wrapper, parallel, reserve, save };
  }

  it("mostra a reserva em GiB e grava em MiB", async () => {
    const { reserve, save, wrapper } = await mounted();
    expect((reserve.element as HTMLInputElement).value).toBe("1");
    // Nada mudou: nada a salvar.
    expect(save.attributes("disabled")).toBeDefined();

    await reserve.setValue("2.5");
    await wrapper.find('input[value="always"]').setValue(true);
    expect(save.attributes("disabled")).toBeUndefined();
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(api.savePreferences).toHaveBeenCalledWith({
      notify: "always",
      maxParallelTransfers: 4,
      minFreeSpaceMib: 2560,
    });
    expect(wrapper.text()).toContain("Preferências salvas.");
  });

  it("valor fora dos limites não deixa salvar", async () => {
    const { parallel, save } = await mounted();
    await parallel.setValue("9");
    expect(save.attributes("disabled")).toBeDefined();
  });
});
