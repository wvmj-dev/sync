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
  formatBytes,
  formatDate,
  formatRelative,
  kindLabel,
  plural,
  runStatusLabel,
  runTone,
} from "../utils/format";
import Icon from "./Icon.vue";
import type { IconName } from "./icons";
import Notice from "./Notice.vue";
import PlanPreview from "./PlanPreview.vue";
import StatusPanel from "./StatusPanel.vue";
import VersionList from "./VersionList.vue";
import { RouterLink } from "vue-router";
import { useAppStore } from "../stores/app";

const props = defineProps<{ job: JobSummary }>();
const emit = defineEmits<{ edit: []; remove: []; changed: [] }>();

type Loadable<T> =
  | { status: "loading" }
  | { status: "ready"; data: T }
  | { status: "error"; message: string };

/** Painel aberto embaixo da tarefa. */
const view = ref<"preview" | "status" | "versions" | null>(null);
const preview = ref<Loadable<Preview> | null>(null);
const versions = ref<Loadable<VersionItem[]> | null>(null);
const running = ref(false);
/** O que está rodando: a execução da tarefa ou uma restauração. */
const activity = ref<"run" | "restore">("run");
const restored = ref<ExtractResult | null>(null);
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

const sync = computed(() => props.job.sync);
const store = useAppStore();
/** RN-06: a conta de destino precisa ser reconectada. */
const reconnect = computed(() =>
  store.accounts.find((a) => a.id === sync.value?.accountId && a.needsReconnectSince),
);
const kind = computed(() => sync.value?.delivery.kind);
const canRun = computed(() => !!sync.value && props.job.enabled && !running.value);
const percent = computed(() => {
  const p = progress.value;
  if (!p) return 0;
  if (p.bytesTotal > 0) return Math.min(100, (p.bytesDone / p.bytesTotal) * 100);
  return p.total > 0 ? (p.done / p.total) * 100 : 0;
});

const state = computed<{ icon: IconName; tone: string; label: string }>(() => {
  const last = props.job.lastRun;
  if (!props.job.enabled) return { icon: "circle", tone: "neutral", label: "Desativada" };
  if (!last) return { icon: "circle", tone: "neutral", label: "Nunca executada" };
  const tone = runTone(last.status);
  const icon: IconName = tone === "ok" ? "check" : tone === "danger" ? "error" : "alert";
  return { icon, tone, label: runStatusLabel[last.status] };
});

/** Próxima execução, ou por que não há uma. */
const next = computed<{ when: string; detail: string }>(() => {
  const s = sync.value;
  if (!s) return { when: "—", detail: "" };
  if (s.nextRun) return { when: formatRelative(s.nextRun), detail: s.schedule };
  switch (s.scheduleState) {
    case "manual":
      return { when: "Só quando você executar", detail: "" };
    case "waiting_first_run":
      return { when: "Depois da 1ª execução manual", detail: s.schedule };
    case "disabled":
      return { when: "Pausada", detail: "A tarefa está desativada" };
    case "paused":
      return { when: "Pausada", detail: "Reconecte a conta em Contas" };
    case "not_installed":
      return { when: "Agendamento não instalado", detail: "Veja os logs" };
    default:
      return { when: s.schedule, detail: "" };
  }
});

const resultKind = computed(() => {
  const tone = result.value ? runTone(result.value.status) : "neutral";
  return tone === "neutral" ? "info" : tone;
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

function toggle(target: "preview" | "status" | "versions") {
  if (view.value === target) {
    view.value = null;
    return;
  }
  view.value = target;
  if (target === "preview") void loadPreview();
  if (target === "versions") void loadVersions();
}

function startActivity(next: "run" | "restore") {
  activity.value = next;
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
          const s = props.job.sync;
          const where = `"${s?.accountLabel}: ${s?.path}"`;
          const what =
            s?.delivery.kind === "receive"
              ? `A versão mais recente de ${where} será baixada e descompactada numa pasta nova em "${s.delivery.target}". Nada existente é substituído.`
              : `${s?.delivery.kind === "archive" ? "Um arquivo compactado será enviado" : "Os arquivos serão enviados"} para ${where}. Nada é apagado no seu computador.`;
          if (!(await confirmDanger(`${e.message}\n\n${what}`, "Executar"))) break;
          confirm = true;
          continue;
        }
        runError.value = errorMessage(e);
        break;
      }
    }
  } finally {
    running.value = false;
    if (view.value === "preview") preview.value = null;
    if (view.value === "versions") void loadVersions();
    emit("changed");
  }
}

async function cancel() {
  await api.cancelRun(props.job.id);
}
</script>

<template>
  <article class="task boxed" :aria-label="`Tarefa ${job.name}`">
    <header class="task-head">
      <Icon class="task-state" :class="`tone-${state.tone}`" :name="state.icon" :size="24" />
      <div class="task-title">
        <h2>{{ job.name }}</h2>
        <div class="task-kind">
          <span v-if="sync">{{ kindLabel(sync) }}</span>
          <span v-if="!job.enabled" class="chip">Desativada</span>
        </div>
      </div>
      <div class="task-actions">
        <button class="btn btn-primary" type="button" :disabled="!canRun" @click="execute">
          <Icon name="play" :size="16" />
          Executar
        </button>
        <button
          class="btn btn-flat btn-icon"
          type="button"
          title="Editar"
          aria-label="Editar tarefa"
          :disabled="running"
          @click="emit('edit')"
        >
          <Icon name="pencil" />
        </button>
        <button
          class="btn btn-danger btn-icon"
          type="button"
          title="Excluir"
          aria-label="Excluir tarefa"
          :disabled="running"
          @click="emit('remove')"
        >
          <Icon name="trash" />
        </button>
      </div>
    </header>

    <div class="task-body">
      <Notice v-if="reconnect" kind="warn">
        <p>
          <strong>A conta {{ reconnect.label }} precisa ser reconectada.</strong> Até lá, esta tarefa
          não roda<template v-if="sync?.scheduleState === 'paused'"> e o agendamento fica pausado</template>.
        </p>
        <p><RouterLink to="/contas">Abrir Contas</RouterLink></p>
      </Notice>
      <dl class="facts">
        <div v-if="sync">
          <dt>{{ kind === "receive" ? "Origem" : "Destino" }}</dt>
          <dd>
            {{ sync.accountLabel ?? "conta removida" }}
            <span class="path block">{{ sync.path }}</span>
          </dd>
        </div>
        <div v-if="sync?.delivery.kind === 'receive'">
          <dt>Pasta no computador</dt>
          <dd class="path">{{ sync.delivery.target }}</dd>
        </div>
        <div>
          <dt>Última execução</dt>
          <dd v-if="job.lastRun" :title="formatDate(job.lastRun.startedAt)">
            <span :class="`tone-text-${state.tone}`">{{ state.label }}</span>,
            {{ formatRelative(job.lastRun.startedAt) }}
            <span v-if="job.lastRun.items != null || job.lastRun.bytes" class="block muted small num">
              {{ plural(job.lastRun.items ?? 0, "item", "itens") }}<template v-if="job.lastRun.bytes">,
                {{ formatBytes(job.lastRun.bytes) }}</template>
            </span>
            <!-- RN-84: por que foi adiada e quando tenta de novo. -->
            <span v-else-if="job.lastRun.status === 'postponed' && job.lastRun.error" class="block muted small">
              {{ job.lastRun.error }}
            </span>
          </dd>
          <dd v-else class="muted">Nunca</dd>
        </div>
        <div>
          <dt>Próxima execução</dt>
          <dd :title="sync?.nextRun ? formatDate(sync.nextRun) : undefined">
            {{ next.when }}
            <span v-if="next.detail" class="block muted small">{{ next.detail }}</span>
          </dd>
        </div>
        <div v-if="job.sources.length" class="wide">
          <dt>Fontes</dt>
          <dd>
            <ul class="paths">
              <li v-for="s in job.sources" :key="s" class="path">{{ s }}</li>
            </ul>
          </dd>
        </div>
      </dl>

      <section v-if="running" class="run" aria-live="polite">
        <div class="run-head">
          <strong>{{ activity === "restore" ? "Restaurando…" : "Executando…" }}</strong>
          <button class="btn btn-flat btn-small" type="button" @click="cancel">
            <Icon name="stop" :size="14" />
            Cancelar
          </button>
        </div>
        <div class="progress" role="progressbar" :aria-valuenow="Math.round(percent)">
          <div :style="{ width: `${percent}%` }"></div>
        </div>
        <p class="run-detail muted small num">
          <template v-if="progress">
            {{ progress.done }} de {{ progress.total }} itens,
            {{ formatBytes(progress.bytesDone)
            }}<template v-if="progress.bytesTotal"> de {{ formatBytes(progress.bytesTotal) }}</template>
          </template>
          <template v-else>
            {{ activity === "restore" ? "Preparando…" : "Varrendo arquivos e calculando o plano…" }}
          </template>
        </p>
        <p v-if="progress?.current" class="path ellipsis">{{ progress.current }}</p>
      </section>

      <Notice v-if="result" :kind="resultKind" dismissible @close="result = null">
        <p>
          <strong>{{ runStatusLabel[result.status] }}</strong>
          em {{ (result.durationMs / 1000).toFixed(1) }} s.
        </p>
        <p v-if="result.receive?.unchanged">
          A versão mais recente ({{ result.receive.name }}) já tinha sido recebida; nada a fazer.
        </p>
        <p v-else-if="result.receive">
          {{ result.receive.name }} recebida e descompactada em
          <span class="path">{{ result.receive.dest }}</span>: {{ result.receive.files }} arquivo(s),
          {{ formatBytes(result.receive.bytes) }}<template v-if="result.receive.skippedCount">;
            {{ result.receive.skippedCount }} entrada(s) ignorada(s) por segurança</template>.
        </p>
        <p v-else-if="result.archive?.unchanged">Nada mudou desde a última versão; nada foi enviado.</p>
        <p v-else-if="result.archive">
          {{ result.archive.name }} enviado: {{ result.archive.files }} arquivo(s),
          {{ formatBytes(result.archive.bytesIn) }} compactados em
          {{ formatBytes(result.archive.bytesOut) }}. {{ result.archive.versions }} versão(ões) no
          destino<template v-if="result.trashed">; {{ result.trashed }} antiga(s) foram para a
            lixeira</template>.
        </p>
        <p v-else>
          {{ result.uploaded }} enviados, {{ result.updated }} atualizados,
          {{ result.foldersCreated }} pastas criadas<template v-if="result.trashed">,
            {{ result.trashed }} na lixeira</template>; {{ formatBytes(result.bytesSent) }}.
        </p>
        <p v-if="result.aborted">Motivo: {{ result.aborted }}</p>
        <p v-if="result.unstable">
          {{ result.unstable }} arquivo(s) mudaram durante o envio e vão de novo na próxima execução.
        </p>
        <details v-if="result.failureCount">
          <summary>{{ result.failureCount }} falha(s)</summary>
          <ul class="small">
            <li v-for="f in result.failures" :key="f.path">
              <span class="path">{{ f.path }}</span>: {{ f.error }}
            </li>
          </ul>
        </details>
      </Notice>

      <Notice
        v-if="restored"
        :kind="restored.skippedCount ? 'warn' : 'ok'"
        dismissible
        @close="restored = null"
      >
        <p>
          <strong>Restaurado</strong> em <span class="path">{{ restored.dest }}</span>:
          {{ restored.files }} arquivo(s), {{ restored.folders }} pasta(s),
          {{ formatBytes(restored.bytes) }}.
        </p>
        <details v-if="restored.skippedCount">
          <summary>{{ restored.skippedCount }} entrada(s) ignorada(s) por segurança</summary>
          <ul class="small">
            <li v-for="e in restored.skipped" :key="e.name">
              <span class="path">{{ e.name }}</span>: {{ e.reason }}
            </li>
          </ul>
        </details>
      </Notice>

      <Notice v-if="runError" kind="danger" dismissible @close="runError = null">
        {{ runError }}
      </Notice>

      <div v-if="sync" class="segmented" role="group" aria-label="Detalhes da tarefa">
        <button
          type="button"
          :aria-pressed="view === 'preview'"
          :disabled="running"
          @click="toggle('preview')"
        >
          <Icon name="eye" :size="16" />
          Prévia
        </button>
        <button
          v-if="kind === 'files'"
          type="button"
          :aria-pressed="view === 'status'"
          @click="toggle('status')"
        >
          <Icon name="status" :size="16" />
          Status
        </button>
        <button
          v-if="kind === 'archive'"
          type="button"
          :aria-pressed="view === 'versions'"
          @click="toggle('versions')"
        >
          <Icon name="versions" :size="16" />
          Versões
        </button>
      </div>
    </div>

    <div v-if="view && !running" class="task-panel">
      <template v-if="view === 'preview'">
        <p v-if="preview?.status === 'loading'" class="muted">Calculando a prévia…</p>
        <PlanPreview v-else-if="preview?.status === 'ready'" :preview="preview.data" />
        <Notice v-else-if="preview?.status === 'error'" kind="danger">{{ preview.message }}</Notice>
      </template>
      <StatusPanel
        v-else-if="view === 'status'"
        :key="job.lastRun?.startedAt"
        :job-id="job.id"
      />
      <template v-else-if="view === 'versions'">
        <p v-if="versions?.status === 'loading'" class="muted">Carregando as versões…</p>
        <VersionList
          v-else-if="versions?.status === 'ready'"
          :versions="versions.data"
          :busy="running"
          @restore="restore"
        />
        <Notice v-else-if="versions?.status === 'error'" kind="danger">
          {{ versions.message }}
        </Notice>
      </template>
    </div>
  </article>
</template>

<style scoped>
.task-head {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 18px 14px;
}

.task-state {
  flex: none;
}

.task-title {
  flex: 1;
  min-width: 0;
}

.task-title h2 {
  font-size: 1.12rem;
}

.task-kind {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 2px;
  color: var(--fg-2);
  font-size: 0.88rem;
}

.task-actions {
  display: flex;
  flex: none;
  align-items: center;
  gap: 4px;
}

.task-actions .btn-primary {
  margin-right: 6px;
}

.boxed > .task-body {
  border-top: 0;
}

.task-body {
  display: grid;
  gap: 16px;
  padding: 2px 18px 16px 56px;
}

.paths {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.run {
  display: grid;
  gap: 8px;
}

.run-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.task-panel {
  padding: 18px;
}

.tone-ok {
  color: var(--success);
}

.tone-warn {
  color: var(--warning);
}

.tone-danger {
  color: var(--danger);
}

.tone-neutral {
  color: var(--fg-3);
}

.tone-text-danger {
  color: var(--danger);
}

.tone-text-warn {
  color: var(--warning);
}

.segmented {
  justify-self: start;
}

@media (max-width: 760px) {
  .task-head {
    flex-wrap: wrap;
  }

  .task-title {
    flex-basis: calc(100% - 40px);
  }

  .task-actions {
    flex-basis: 100%;
    padding-left: 38px;
  }

  .task-body {
    padding-left: 18px;
  }
}
</style>
