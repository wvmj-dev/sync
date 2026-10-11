<script setup lang="ts">
import { ref } from "vue";
import { confirmDanger } from "../api";
import type { JobDraft, JobSummary } from "../api/types";
import JobCard from "../components/JobCard.vue";
import JobForm from "../components/JobForm.vue";
import { useAppStore } from "../stores/app";

const store = useAppStore();

/** `undefined` = formulário fechado; `null` = nova tarefa. */
const editing = ref<JobSummary | null | undefined>(undefined);

async function save(draft: JobDraft) {
  if (await store.saveJob(draft)) editing.value = undefined;
}

async function remove(job: JobSummary) {
  const message = `Excluir a tarefa "${job.name}"? Nenhum arquivo será apagado.`;
  if (await confirmDanger(message, "Excluir")) {
    await store.deleteJob(job.id);
  }
}
</script>

<template>
  <header class="page-head">
    <h1>Tarefas</h1>
    <button
      v-if="editing === undefined"
      class="btn btn-primary"
      type="button"
      @click="editing = null"
    >
      Nova tarefa
    </button>
  </header>

  <JobForm
    v-if="editing !== undefined"
    :key="editing?.id ?? 'new'"
    :job="editing"
    :accounts="store.accounts"
    @save="save"
    @cancel="editing = undefined"
  />

  <p v-if="!store.jobs.length && editing === undefined" class="card muted">
    Nenhuma tarefa ainda. Crie uma para escolher as pastas e o destino.
  </p>

  <JobCard
    v-for="job in store.jobs"
    :key="job.id"
    :job="job"
    @edit="editing = job"
    @remove="remove(job)"
    @changed="store.refresh()"
  />
</template>
