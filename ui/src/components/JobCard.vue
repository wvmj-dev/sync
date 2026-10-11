<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ApiError, api, confirmDanger, errorMessage, onRunProgress } from "../api";
import type { JobSummary, PlanPreview as Preview, RunProgress, RunReport } from "../api/types";
import { formatBytes, formatDate, modeLabel, runStatusLabel } from "../utils/format";
import PlanPreview from "./PlanPreview.vue";

const props = defineProps<{ job: JobSummary }>();
const emit = defineEmits<{ edit: []; remove: []; changed: [] }>();

type PreviewState =
  | { status: "loading" }
  | { status: "ready"; data: Preview }
  | { status: "error"; message: string };

const preview = ref<PreviewState | null>(null);
const running = ref(false);
const progress = ref<RunProgress | null>(null);
const result = ref<RunReport | null>(null);
const runError = ref<string | null>(null);

// Prévia antiga não vale mais se a tarefa mudou (fontes, destino, exclusões…).
const signature = computed(() =>
  JSON.stringify([props.job.sources, props.job.sync, props.job.exclude, props.job.includeSensitive]),
);
watch(signature, () => (preview.value = null));

let stopListening: (() => void) | null = null;
onMounted(async () => {
  stopListening = await onRunProgress((p) => {
    if (p.jobId === props.job.id) progress.value = p;
  });
});
onUnmounted(() => stopListening?.());

const canRun = computed(() => !!props.job.sync && props.job.enabled && !running.value);
const percent = computed(() => {
  const p = progress.value;
  if (!p) return 0;
  if (p.bytesTotal > 0) return Math.min(100, (p.bytesDone / p.bytesTotal) * 100);
  return p.total > 0 ? (p.done / p.total) * 100 : 0;
});

const resultKind = computed(() => {
  switch (result.value?.status) {
    case "success":
      return "ok";
    case "partial_success":
    case "cancelled":
      return "warning";
    default:
      return "danger";
  }
});

async function loadPreview() {
  preview.value = { status: "loading" };
  try {
    preview.value = { status: "ready", data: await api.previewJob(props.job.id) };
  } catch (e) {
    preview.value = { status: "error", message: errorMessage(e) };
  }
}

/** RN-36/RN-42: se o plano exigir confirmação, pergunta antes de executar de verdade. */
async function execute() {
  running.value = true;
  progress.value = null;
  result.value = null;
  runError.value = null;
  let confirm = false;
  try {
    for (;;) {
      try {
        result.value = await api.runJob(props.job.id, confirm);
        break;
      } catch (e) {
        if (e instanceof ApiError && e.code === "needs_confirmation" && !confirm) {
          const message = `${e.message}\n\nOs arquivos serão enviados para "${props.job.sync?.accountLabel}: ${props.job.sync?.path}". Nada é apagado no seu computador.`;
          if (!(await confirmDanger(message, "Executar"))) break;
          confirm = true;
          continue;
        }
        runError.value = errorMessage(e);
        break;
      }
    }
  } finally {
    running.value = false;
    preview.value = null;
    emit("changed");
  }
}

async function cancel() {
  await api.cancelRun(props.job.id);
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
          :disabled="!job.sync || running || preview?.status === 'loading'"
          @click="loadPreview"
        >
          Pré-visualizar
        </button>
        <button class="btn btn-primary" type="button" :disabled="!canRun" @click="execute">
          Executar
        </button>
        <button class="btn btn-ghost" type="button" :disabled="running" @click="emit('edit')">
          Editar
        </button>
        <button class="btn btn-ghost" type="button" :disabled="running" @click="emit('remove')">
          Excluir
        </button>
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

    <section v-if="running" class="run" aria-live="polite">
      <div class="run-head">
        <strong>Executando…</strong>
        <button class="btn btn-ghost" type="button" @click="cancel">Cancelar</button>
      </div>
      <div class="progress" role="progressbar" :aria-valuenow="Math.round(percent)">
        <div :style="{ width: `${percent}%` }"></div>
      </div>
      <p v-if="progress" class="muted small">
        {{ progress.done }} de {{ progress.total }} itens ·
        {{ formatBytes(progress.bytesDone) }} de {{ formatBytes(progress.bytesTotal) }}
      </p>
      <p v-if="progress?.current" class="muted small run-current">
        <code>{{ progress.current }}</code>
      </p>
      <p v-else class="muted small">Varrendo arquivos e calculando o plano…</p>
    </section>

    <section v-if="result" class="run">
      <div class="banner" :class="`banner-${resultKind}`">
        <div>
          <strong>{{ runStatusLabel[result.status] }}</strong>
          em {{ (result.durationMs / 1000).toFixed(1) }} s —
          {{ result.uploaded }} enviados, {{ result.updated }} atualizados,
          {{ result.foldersCreated }} pastas criadas<template v-if="result.trashed">,
            {{ result.trashed }} na lixeira</template>
          ({{ formatBytes(result.bytesSent) }}).
          <p v-if="result.aborted" class="small">Motivo: {{ result.aborted }}</p>
          <p v-if="result.unstable" class="small">
            {{ result.unstable }} arquivo(s) mudaram durante o envio e serão enviados de novo na
            próxima execução.
          </p>
        </div>
        <button class="btn btn-ghost" type="button" @click="result = null">Fechar</button>
      </div>
      <details v-if="result.failureCount">
        <summary>{{ result.failureCount }} falha(s)</summary>
        <ul class="small">
          <li v-for="f in result.failures" :key="f.path">
            <code>{{ f.path }}</code>: {{ f.error }}
          </li>
        </ul>
      </details>
    </section>
    <div v-if="runError" class="banner banner-danger">
      <span>{{ runError }}</span>
      <button class="btn btn-ghost" type="button" @click="runError = null">Fechar</button>
    </div>

    <template v-if="!running">
      <p v-if="preview?.status === 'loading'" class="muted">Calculando prévia…</p>
      <PlanPreview v-else-if="preview?.status === 'ready'" :preview="preview.data" />
      <div v-else-if="preview?.status === 'error'" class="banner banner-danger">
        {{ preview.message }}
      </div>
    </template>
  </article>
</template>
