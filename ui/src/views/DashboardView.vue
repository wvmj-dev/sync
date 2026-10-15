<script setup lang="ts">
import { computed } from "vue";
import { RouterLink } from "vue-router";
import { useAppStore } from "../stores/app";
import { formatDate, runStatusLabel } from "../utils/format";

const store = useAppStore();

const steps = computed(() => [
  {
    done: store.info?.googleClientConfigured ?? false,
    title: "Projeto do Google Cloud",
    hint: "Siga docs/CONFIGURAR_GOOGLE.md e salve o google-client.json.",
  },
  {
    done: store.accounts.length > 0,
    title: "Conta de destino",
    hint: "Conecte o Google Drive ou adicione uma pasta local (HD externo).",
    to: "/contas",
  },
  {
    done: store.jobs.length > 0,
    title: "Primeira tarefa",
    hint: "Escolha as pastas e o destino, depois veja a prévia.",
    to: "/tarefas",
  },
]);

const activeJobs = computed(() => store.jobs.filter((j) => j.enabled).length);
const archiveJobs = computed(
  () => store.jobs.filter((j) => j.sync?.delivery.kind === "archive").length,
);
</script>

<template>
  <header class="page-head">
    <h1>Visão geral</h1>
  </header>

  <div v-if="store.info?.configProblem" class="banner banner-danger">
    Configuração com problema: {{ store.info.configProblem }}
  </div>
  <div v-if="store.info?.googleClientProblem" class="banner banner-warning">
    {{ store.info.googleClientProblem }}
  </div>

  <section class="grid-3">
    <div class="card stat">
      <span class="stat-value">{{ activeJobs }}</span>
      <span class="muted">tarefas ativas</span>
    </div>
    <div class="card stat">
      <span class="stat-value">{{ store.accounts.length }}</span>
      <span class="muted">contas</span>
    </div>
    <div class="card stat">
      <span class="stat-value">{{ archiveJobs }}</span>
      <span class="muted">enviam compactado</span>
    </div>
  </section>

  <section class="card">
    <h2>Primeiros passos</h2>
    <ol class="steps">
      <li v-for="step in steps" :key="step.title" :class="{ done: step.done }">
        <span class="step-mark" aria-hidden="true">{{ step.done ? "✓" : "○" }}</span>
        <div>
          <strong>
            <RouterLink v-if="step.to && !step.done" :to="step.to">{{ step.title }}</RouterLink>
            <template v-else>{{ step.title }}</template>
          </strong>
          <p class="muted">{{ step.hint }}</p>
        </div>
      </li>
    </ol>
  </section>

  <section v-if="store.jobs.length" class="card">
    <h2>Tarefas</h2>
    <table class="table">
      <thead>
        <tr>
          <th>Nome</th>
          <th>Agendamento</th>
          <th>Última execução</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="job in store.jobs" :key="job.id">
          <td>
            {{ job.name }}
            <span v-if="!job.enabled" class="badge">desativada</span>
          </td>
          <td>
            {{ job.sync?.schedule ?? "—" }}
            <span v-if="job.sync?.nextRun" class="muted small">
              · próxima {{ formatDate(job.sync.nextRun) }}</span>
          </td>
          <td>
            <template v-if="job.lastRun">
              {{ runStatusLabel[job.lastRun.status] }} · {{ formatDate(job.lastRun.startedAt) }}
            </template>
            <span v-else class="muted">nunca</span>
          </td>
        </tr>
      </tbody>
    </table>
  </section>

  <footer class="muted small paths">
    <div>Configuração: <code>{{ store.info?.configFile }}</code></div>
    <div>Dados: <code>{{ store.info?.dataDir }}</code></div>
    <div>Logs: <code>{{ store.info?.logDir }}</code></div>
  </footer>
</template>
