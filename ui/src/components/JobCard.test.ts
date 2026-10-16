import { describe, expect, it, vi } from "vitest";
import { useAppStore } from "../stores/app";
import { account, job, mountApp, run } from "../testing";
import JobCard from "./JobCard.vue";

// Sem Tauri nos testes: o progresso das execuções vira uma inscrição vazia.
vi.mock("../api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api")>()),
  onRunProgress: vi.fn(async () => () => {}),
}));

describe("JobCard", () => {
  it("RN-06, RN-85: conta a reconectar avisa e explica o agendamento pausado", () => {
    const wrapper = mountApp(JobCard, { job: job({}, { scheduleState: "paused", nextRun: null }) });
    useAppStore().accounts = [account({ needsReconnectSince: "2026-10-15T08:00:00Z" })];
    return wrapper.vm.$nextTick().then(() => {
      const text = wrapper.text();
      expect(text).toContain("A conta eu@exemplo.com precisa ser reconectada.");
      expect(text).toContain("esta tarefa não roda e o agendamento fica pausado.");
      expect(text).toContain("Pausada");
      expect(wrapper.find('a[href="/contas"]').exists()).toBe(true);
    });
  });

  it("tarefa manual não fala de agendamento pausado", async () => {
    const wrapper = mountApp(JobCard, {
      job: job({}, { scheduleState: "manual", scheduleSpec: { kind: "manual" }, nextRun: null }),
    });
    useAppStore().accounts = [account({ needsReconnectSince: "2026-10-15T08:00:00Z" })];
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toContain("esta tarefa não roda.");
    expect(wrapper.text()).not.toContain("agendamento fica pausado");
  });

  it("conta em dia: nenhum aviso", async () => {
    const wrapper = mountApp(JobCard, { job: job() });
    useAppStore().accounts = [account()];
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).not.toContain("reconectada");
  });

  it("RN-84: execução adiada mostra o motivo e quando tenta de novo", () => {
    const reason = "sem conexão com a internet; nova tentativa em 15 min";
    const wrapper = mountApp(JobCard, {
      job: job({ lastRun: run({ status: "postponed", error: reason, items: null, bytes: null }) }),
    });
    expect(wrapper.text()).toContain("Adiada");
    expect(wrapper.text()).toContain(reason);
  });
});
