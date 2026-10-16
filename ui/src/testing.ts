// Apoio aos testes do frontend (`pnpm test`). Não entra no app: só os `*.test.ts` importam.
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { Component } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import type { AccountSummary, JobSummary, RunSummary, SyncSummary } from "./api/types";

const at = "2026-10-16T12:00:00Z";

export function account(over: Partial<AccountSummary> = {}): AccountSummary {
  return {
    id: "a1",
    label: "eu@exemplo.com",
    provider: "google_drive",
    detail: "eu@exemplo.com",
    jobCount: 1,
    needsReconnectSince: null,
    ...over,
  };
}

export function job(over: Partial<JobSummary> = {}, sync: Partial<SyncSummary> = {}): JobSummary {
  return {
    id: "j1",
    name: "Documentos",
    enabled: true,
    sources: ["/home/usuario/Documentos"],
    exclude: [],
    includeSensitive: false,
    sync: {
      accountId: "a1",
      accountLabel: "eu@exemplo.com",
      path: "Backup/Documentos",
      delivery: { kind: "files" },
      mode: "upload",
      schedule: "Diário às 02:30",
      scheduleSpec: { kind: "daily", at: "02:30" },
      scheduleState: "active",
      nextRun: at,
      ...sync,
    },
    lastRun: null,
    ...over,
  };
}

export function run(over: Partial<RunSummary> = {}): RunSummary {
  return {
    kind: "sync",
    trigger: "scheduled",
    status: "success",
    startedAt: at,
    finishedAt: at,
    error: null,
    items: 3,
    bytes: 1024,
    ...over,
  };
}

/** Monta com Pinia e um roteador em memória — como no app, sem janela nem Rust. */
export function mountApp(component: Component, props: Record<string, unknown> = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const empty = { render: () => null };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: empty },
      { path: "/contas", component: empty },
    ],
  });
  return mount(component, { props, global: { plugins: [pinia, router] } });
}
