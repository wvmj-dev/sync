<script setup lang="ts">
import { ref } from "vue";
import { confirmDanger } from "../api";
import type { JobDraft, JobSummary } from "../api/types";
import JobCard from "../components/JobCard.vue";
import Icon from "../components/Icon.vue";
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
    <div>
      <h1>Tarefas</h1>
      <p class="page-desc">O que vai para onde, e quando. Use a prévia para ver o que vai acontecer.</p>
    </div>
    <button v-if="editing === undefined" class="btn btn-primary" type="button" @click="editing = null">
      <Icon name="plus" :size="16" />
      Nova tarefa
    </button>
  </header>

  <div class="stack">
    <JobForm
      v-if="editing !== undefined"
      :key="editing?.id ?? 'new'"
      :job="editing"
      :accounts="store.accounts"
      @save="save"
      @cancel="editing = undefined"
    />

    <div v-if="!store.jobs.length && editing === undefined" class="boxed empty">
      <Icon name="tasks" :size="28" />
      <strong>Nenhuma tarefa ainda</strong>
      <p>Uma tarefa diz quais pastas vão para qual destino, e como.</p>
      <button class="btn btn-primary" type="button" @click="editing = null">
        <Icon name="plus" :size="16" />
        Criar a primeira tarefa
      </button>
    </div>

    <JobCard
      v-for="job in store.jobs"
      :key="job.id"
      :job="job"
      @edit="editing = job"
      @remove="remove(job)"
      @changed="store.refresh()"
    />
  </div>
</template>
