import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, api } from "../api";
import type { RemoteItem } from "../api/types";
import { useAppStore } from "../stores/app";
import { account, mountApp } from "../testing";
import FilesView from "./FilesView.vue";

vi.mock("../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api")>();
  return {
    ...actual,
    onArchiveProgress: vi.fn(async () => () => {}),
    api: { ...actual.api, listRemote: vi.fn(), createRemoteFolder: vi.fn() },
  };
});

const fotos: RemoteItem = {
  name: "Fotos",
  path: "Fotos",
  kind: "folder",
  size: null,
  modifiedAt: "2026-10-16T12:00:00Z",
  modifiedBy: "Você",
  downloadable: false,
};

beforeEach(() => {
  vi.mocked(api.listRemote).mockReset().mockResolvedValue([]);
  vi.mocked(api.createRemoteFolder).mockReset().mockResolvedValue(fotos);
});

async function opened() {
  const wrapper = mountApp(FilesView);
  useAppStore().accounts = [account()];
  await flushPromises();
  const button = wrapper.findAll("button").find((b) => b.text() === "Nova pasta");
  if (!button) throw new Error("botão Nova pasta não encontrado");
  await button.trigger("click");
  return wrapper;
}

describe("Arquivos → Nova pasta (RN-111)", () => {
  it("cria na pasta atual, sem espaços nas pontas, e recarrega a lista", async () => {
    const wrapper = await opened();
    vi.mocked(api.listRemote).mockResolvedValue([fotos]);
    await wrapper.find('input[aria-label="Nome da nova pasta"]').setValue("  Fotos  ");
    await wrapper.find("form.new-folder").trigger("submit");
    await flushPromises();

    expect(api.createRemoteFolder).toHaveBeenCalledWith("a1", "", "Fotos");
    expect(wrapper.text()).toContain("Pasta criada: Syncly/Fotos");
    expect(wrapper.find("form.new-folder").exists()).toBe(false);
    expect(wrapper.text()).toContain("Fotos");
  });

  it("nome em branco não chama o Rust", async () => {
    const wrapper = await opened();
    await wrapper.find('input[aria-label="Nome da nova pasta"]').setValue("   ");
    await wrapper.find("form.new-folder").trigger("submit");
    expect(api.createRemoteFolder).not.toHaveBeenCalled();
  });

  it("nome repetido: mostra o motivo e mantém a linha aberta (RN-104)", async () => {
    vi.mocked(api.createRemoteFolder).mockRejectedValue(
      new ApiError("invalid", "já existe 'Fotos' em Syncly/ — nada foi sobrescrito"),
    );
    const wrapper = await opened();
    await wrapper.find('input[aria-label="Nome da nova pasta"]').setValue("Fotos");
    await wrapper.find("form.new-folder").trigger("submit");
    await flushPromises();
    expect(wrapper.text()).toContain("já existe 'Fotos'");
    expect(wrapper.find("form.new-folder").exists()).toBe(true);
  });

  it("Esc fecha sem criar", async () => {
    const wrapper = await opened();
    await wrapper.find("form.new-folder").trigger("keydown", { key: "Escape" });
    expect(wrapper.find("form.new-folder").exists()).toBe(false);
    expect(api.createRemoteFolder).not.toHaveBeenCalled();
  });
});
