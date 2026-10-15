<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ApiError, api, confirmDanger, errorMessage, onRunProgress, pickFolder } from "../api";
import type {
  ExtractResult,
  JobSummary,
  PlanPreview as Preview,
  RunProgress,
  RunReport,
  VersionItem,
} from "../api/types";
import {
  deliveryLabel,
  formatBytes,
  formatDate,
  runStatusLabel,
  scheduleStateLabel,
} from "../utils/format";
import PlanPreview from "./PlanPreview.vue";
import StatusPanel from "./StatusPanel.vue";
import VersionList from "./VersionList.vue";

const props = defineProps<{ job: JobSummary }>();
const emit = defineEmits<{ edit: []; remove: []; changed: [] }>();

type PreviewState =
  | { status: "loading" }
  | { status: "ready"; data: Preview }
  | { status: "error"; message: string };

type VersionsState =
  | { status: "loading" }
  | { status: "ready"; data: VersionItem[] }
  | { status: "error"; message: string };

const preview = ref<PreviewState | null>(null);
const versions = ref<VersionsState | null>(null);
const running = ref(false);
/** O que está rodando: a execução da tarefa ou uma restauração. */
const activity = ref<"run" | "restore">("run");
const restored = ref<ExtractResult | null>(null);
const showStatus = ref(false);
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
const isArchive = computed(() => props.job.sync?.delivery.kind === "archive");
const scheduleNote = computed(() =>
  props.job.sync ? scheduleStateLabel[props.job.sync.scheduleState] : "",
);
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

async function loadVersions() {
  versions.value = { status: "loading" };
  try {
    versions.value = { status: "ready", data: await api.listVersions(props.job.id) };
  } catch (e) {
    versions.value = { status: "error", message: errorMessage(e) };
  }
}

function toggleVersions() {
  if (versions.value) versions.value = null;
  else void loadVersions();
}

function startActivity(kind: "run" | "restore") {
  activity.value = kind;
  running.value = true;
  progress.value = null;
  result.value = null;
  restored.value = null;
  runError.value = null;
}

/** RN-57: sempre numa pasta nova dentro da pasta escolhida. */
async function restore(version: VersionItem) {
  const dir = await pickFolder(`Restaurar ${version.name} em…`);
  if (!dir) return;
  startActivity("restore");
  try {
    restored.value = await api.restoreVersion(props.job.id, version.id, dir);
  } catch (e) {
    runError.value = errorMessage(e);
  } finally {
    running.value = false;
    emit("changed");
  }
}

/** RN-36/RN-42: se o plano exigir confirmação, pergunta antes de executar de verdade. */
async function execute() {
  startActivity("run");
  let confirm = false;
  try {
    for (;;) {
      try {
        result.value = await api.runJob(props.job.id, confirm);
        break;
      } catch (e) {
        if (e instanceof ApiError && e.code === "needs_confirmation" && !confirm) {
          const sync = props.job.sync;
          const where = `"${sync?.accountLabel}: ${sync?.path}"`;
          const what =
            sync?.delivery.kind === "receive"
              ? `A versão mais recente de ${where} será baixada e descompactada numa pasta nova em "${sync.delivery.target}". Nada existente é substituído.`
              : `${sync?.delivery.kind === "archive" ? "Um arquivo compactado será enviado" : "Os arquivos serão enviados"} para ${where}. Nada é apagado no seu computador.`;
          const message = `${e.message}\n\n${what}`;
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
    if (versions.value) void loadVersions();
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
        <p v-if="job.sync?.delivery.kind === 'receive'" class="muted">
          Recebe de {{ job.sync.accountLabel ?? "conta removida" }}:
          <code>{{ job.sync.path }}</code> → <code>{{ job.sync.delivery.target }}</code>
          · {{ job.sync.schedule }}<template v-if="scheduleNote"> ({{ scheduleNote }})</template>
        </p>
        <p v-else-if="job.sync" class="muted">
          {{ deliveryLabel(job.sync) }} →
          {{ job.sync.accountLabel ?? "conta removida" }}: <code>{{ job.sync.path }}</code>
          · {{ job.sync.schedule }}<template v-if="scheduleNote"> ({{ scheduleNote }})</template>
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
        <button
          v-if="isArchive"
          class="btn"
          type="button"
          :aria-expanded="!!versions"
          @click="toggleVersions"
        >
          Versões
        </button>
        <button
          v-if="job.sync?.delivery.kind === 'files'"
          class="btn"
          type="button"
          :aria-expanded="showStatus"
          @click="showStatus = !showStatus"
        >
          Status
        </button>
        <button class="btn btn-ghost" type="button" :disabled="running" @click="emit('edit')">
          Editar
        </button>
        <button class="btn btn-ghost" type="button" :disabled="running" @click="emit('remove')">
          Excluir
        </button>
      </div>
    </div>

    <ul v-if="job.sources.length" class="sources">
      <li v-for="s in job.sources" :key="s"><code>{{ s }}</code></li>
    </ul>
    <p class="muted small">
      Última execução:
      <template v-if="job.lastRun">
        {{ runStatusLabel[job.lastRun.status] }} em {{ formatDate(job.lastRun.startedAt)
        }}<template v-if="job.lastRun.items != null">, {{ job.lastRun.items }} item(ns)</template
        ><template v-if="job.lastRun.bytes">, {{ formatBytes(job.lastRun.bytes) }}</template>
      </template>
      <template v-else>nunca</template>
      <template v-if="job.sync?.nextRun">
        · Próxima: {{ formatDate(job.sync.nextRun) }}
      </template>
    </p>

    <StatusPanel v-if="showStatus && !running" :key="job.lastRun?.startedAt" :job-id="job.id" />

    <section v-if="running" class="run" aria-live="polite">
      <div class="run-head">
        <strong>{{ activity === "restore" ? "Restaurando…" : "Executando…" }}</strong>
        <button class="btn btn-ghost" type="button" @click="cancel">Cancelar</button>
      </div>
      <div class="progress" role="progressbar" :aria-valuenow="Math.round(percent)">
        <div :style="{ width: `${percent}%` }"></div>
      </div>
      <p v-if="progress" class="muted small">
        {{ progress.done }} de {{ progress.total }} itens ·
        {{ formatBytes(progress.bytesDone)
        }}<template v-if="progress.bytesTotal"> de {{ formatBytes(progress.bytesTotal) }}</template>
      </p>
      <p v-if="progress?.current" class="muted small run-current">
        <code>{{ progress.current }}</code>
      </p>
      <p v-else class="muted small">
        {{ activity === "restore" ? "Preparando…" : "Varrendo arquivos e calculando o plano…" }}
      </p>
    </section>

    <section v-if="result" class="run">
      <div class="banner" :class="`banner-${resultKind}`">
        <div>
          <strong>{{ runStatusLabel[result.status] }}</strong>
          em {{ (result.durationMs / 1000).toFixed(1) }} s —
          <template v-if="result.receive?.unchanged">
            <code>{{ result.receive.name }}</code> já tinha sido recebida; nada a fazer.
          </template>
          <template v-else-if="result.receive">
            <code>{{ result.receive.name }}</code> recebida e descompactada em
            <code>{{ result.receive.dest }}</code> ({{ result.receive.files }} arquivo(s),
            {{ formatBytes(result.receive.bytes) }})<template v-if="result.receive.skippedCount">;
              {{ result.receive.skippedCount }} entrada(s) ignorada(s) por segurança</template>.
          </template>
          <template v-else-if="result.archive?.unchanged">
            nada mudou desde a última versão; nada foi enviado.
          </template>
          <template v-else-if="result.archive">
            <code>{{ result.archive.name }}</code> enviado ({{ result.archive.files }} arquivo(s),
            {{ formatBytes(result.archive.bytesIn) }} → {{ formatBytes(result.archive.bytesOut) }}).
            {{ result.archive.versions }} versão(ões) no destino<template v-if="result.trashed">,
              {{ result.trashed }} antiga(s) na lixeira</template>.
          </template>
          <template v-else>
            {{ result.uploaded }} enviados, {{ result.updated }} atualizados,
            {{ result.foldersCreated }} pastas criadas<template v-if="result.trashed">,
              {{ result.trashed }} na lixeira</template>
            ({{ formatBytes(result.bytesSent) }}).
          </template>
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
    <section v-if="restored" class="run">
      <div class="banner" :class="restored.skippedCount ? 'banner-warning' : 'banner-ok'">
        <div>
          <strong>Restaurado</strong> em <code>{{ restored.dest }}</code> —
          {{ restored.files }} arquivo(s), {{ restored.folders }} pasta(s)
          ({{ formatBytes(restored.bytes) }}).
        </div>
        <button class="btn btn-ghost" type="button" @click="restored = null">Fechar</button>
      </div>
      <details v-if="restored.skippedCount">
        <summary>{{ restored.skippedCount }} entrada(s) ignorada(s) por segurança</summary>
        <ul class="small">
          <li v-for="e in restored.skipped" :key="e.name">
            <code>{{ e.name }}</code>: {{ e.reason }}
          </li>
        </ul>
      </details>
    </section>
    <div v-if="runError" class="banner banner-danger">
      <span>{{ runError }}</span>
      <button class="btn btn-ghost" type="button" @click="runError = null">Fechar</button>
    </div>

    <section v-if="versions" class="run">
      <h3>Versões</h3>
      <p v-if="versions.status === 'loading'" class="muted">Carregando versões…</p>
      <VersionList
        v-else-if="versions.status === 'ready'"
        :versions="versions.data"
        :busy="running"
        @restore="restore"
      />
      <div v-else class="banner banner-danger">{{ versions.message }}</div>
    </section>

    <template v-if="!running">
      <p v-if="preview?.status === 'loading'" class="muted">Calculando prévia…</p>
      <PlanPreview v-else-if="preview?.status === 'ready'" :preview="preview.data" />
      <div v-else-if="preview?.status === 'error'" class="banner banner-danger">
        {{ preview.message }}
      </div>
    </template>
  </article>
</template>
