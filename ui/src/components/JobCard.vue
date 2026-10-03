<script setup lang="ts">
import { ref } from "vue";
import { api, errorMessage } from "../api";
import type { JobSummary, PlanPreview as Preview } from "../api/types";
import { formatDate, modeLabel, runStatusLabel } from "../utils/format";
import PlanPreview from "./PlanPreview.vue";

const props = defineProps<{ job: JobSummary }>();
const emit = defineEmits<{ edit: []; remove: [] }>();

type PreviewState =
  | { status: "loading" }
  | { status: "ready"; data: Preview }
  | { status: "error"; message: string };

const preview = ref<PreviewState | null>(null);

async function loadPreview() {
  preview.value = { status: "loading" };
  try {
    preview.value = { status: "ready", data: await api.previewJob(props.job.id) };
  } catch (e) {
    preview.value = { status: "error", message: errorMessage(e) };
  }
}
</script>

<template>
  <article class="card">
    <div class="job-head">
      <div>
        <h2>
          {{ job.name }}
          <span v-if="!job.enabled" class="badge">desativada</span>
        </h2>
        <p v-if="job.sync" class="muted">
          {{ modeLabel[job.sync.mode] }} →
          {{ job.sync.accountLabel ?? "conta removida" }}: <code>{{ job.sync.path }}</code>
          · {{ job.sync.schedule }}
        </p>
      </div>
      <div class="row">
        <button
          class="btn"
          type="button"
          :disabled="!job.sync || preview?.status === 'loading'"
          @click="loadPreview"
        >
          Pré-visualizar
        </button>
        <button class="btn" type="button" disabled title="Disponível na fase 1">Executar</button>
        <button class="btn btn-ghost" type="button" @click="emit('edit')">Editar</button>
        <button class="btn btn-ghost" type="button" @click="emit('remove')">Excluir</button>
      </div>
    </div>

    <ul class="sources">
      <li v-for="s in job.sources" :key="s"><code>{{ s }}</code></li>
    </ul>
    <p class="muted small">
      Última execução:
      <template v-if="job.lastRun">
        {{ runStatusLabel[job.lastRun.status] }} em {{ formatDate(job.lastRun.startedAt) }}
      </template>
      <template v-else>nunca</template>
    </p>

    <p v-if="preview?.status === 'loading'" class="muted">Calculando prévia…</p>
    <PlanPreview v-else-if="preview?.status === 'ready'" :preview="preview.data" />
    <div v-else-if="preview?.status === 'error'" class="banner banner-danger">
      {{ preview.message }}
    </div>
  </article>
</template>
