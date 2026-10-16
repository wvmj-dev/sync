import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api, pickFile, pickFolder } from "../api";
import type { Preferences } from "../api/types";
import { mountApp } from "../testing";
import PreferencesView from "./PreferencesView.vue";

vi.mock("../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api")>();
  return {
    ...actual,
    pickFolder: vi.fn(),
    pickFile: vi.fn(),
    api: {
      ...actual.api,
      getPreferences: vi.fn(),
      savePreferences: vi.fn(),
      encryptionStatus: vi.fn(),
      createEncryptionKey: vi.fn(),
      activateEncryptionKey: vi.fn(),
      importEncryptionKey: vi.fn(),
    },
  };
});

const stored: Preferences = { notify: "failures", minFreeSpaceMib: 1024, maxParallelTransfers: 4 };

beforeEach(() => {
  vi.mocked(api.getPreferences).mockResolvedValue(stored);
  vi.mocked(api.savePreferences).mockImplementation(async (p) => p);
  vi.mocked(api.encryptionStatus).mockResolvedValue({ active: null, known: [] });
  vi.mocked(api.activateEncryptionKey).mockReset();
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

describe("Criptografia (RN-72)", () => {
  const key = {
    fingerprint: "3f9a1c0b7d2e4a65",
    publicKey: "age1publica",
    recoveryFile: "/media/pendrive/syncly-chave-3f9a1c0b7d2e4a65.txt",
  };

  function button(wrapper: ReturnType<typeof mountApp>, text: string) {
    const found = wrapper.findAll("button").find((b) => b.text().includes(text));
    if (!found) throw new Error(`botão "${text}" não encontrado`);
    return found;
  }

  it("só ativa depois de confirmar que o arquivo de recuperação foi guardado", async () => {
    vi.mocked(pickFolder).mockResolvedValue("/media/pendrive");
    vi.mocked(api.createEncryptionKey).mockResolvedValue(key);
    vi.mocked(api.activateEncryptionKey).mockResolvedValue({
      active: key.fingerprint,
      known: [key.fingerprint],
    });
    const wrapper = mountApp(PreferencesView);
    await flushPromises();
    expect(wrapper.text()).toContain("Nenhuma — as versões vão sem criptografia");

    await button(wrapper, "Criar chave").trigger("click");
    await flushPromises();
    expect(api.createEncryptionKey).toHaveBeenCalledWith("/media/pendrive");
    expect(wrapper.text()).toContain(key.recoveryFile);
    // A chave privada nunca chega à tela — só o caminho do arquivo.
    expect(wrapper.html()).not.toContain("AGE-SECRET-KEY");

    const activate = button(wrapper, "Ativar criptografia");
    expect(activate.attributes("disabled")).toBeDefined();
    await wrapper.find('input[role="switch"]').setValue(true);
    expect(activate.attributes("disabled")).toBeUndefined();
    await activate.trigger("click");
    await flushPromises();
    expect(api.activateEncryptionKey).toHaveBeenCalledWith("age1publica", true);
    // Impressão digital em blocos de 4, para conferir com o arquivo.
    expect(wrapper.text()).toContain("3f9a 1c0b 7d2e 4a65");
  });

  it("importar chave com uma já ativa a mantém como antiga", async () => {
    vi.mocked(api.encryptionStatus).mockResolvedValue({ active: "aaaa", known: ["aaaa"] });
    vi.mocked(pickFile).mockResolvedValue("/media/pendrive/chave.txt");
    vi.mocked(api.importEncryptionKey).mockResolvedValue({ active: "aaaa", known: ["aaaa", "bbbb"] });
    const wrapper = mountApp(PreferencesView);
    await flushPromises();
    await button(wrapper, "Importar chave").trigger("click");
    await flushPromises();
    expect(api.importEncryptionKey).toHaveBeenCalledWith("/media/pendrive/chave.txt");
    expect(wrapper.text()).toContain("1 chave para abrir versões anteriores");
    expect(wrapper.text()).toContain("voltam a abrir");
  });
});
