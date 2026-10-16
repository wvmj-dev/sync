<script setup lang="ts">
import { computed } from "vue";
import { RouterLink } from "vue-router";
import type { JobSummary } from "../api/types";
import Icon from "../components/Icon.vue";
import type { IconName } from "../components/icons";
import Notice from "../components/Notice.vue";
import { useAppStore } from "../stores/app";
import { deliveryLabel, formatRelative, runTone } from "../utils/format";

const store = useAppStore();

const steps = computed(() => [
  {
    done: store.info?.googleClientConfigured ?? false,
    title: "Configurar o projeto do Google Cloud",
    hint: "Siga docs/CONFIGURAR_GOOGLE.md e salve o google-client.json.",
    to: null,
  },
  {
    done: store.accounts.length > 0,
    title: "Adicionar uma conta de destino",
    hint: "Google Drive ou uma pasta local, como um HD externo.",
    to: "/contas",
  },
  {
    done: store.jobs.length > 0,
    title: "Criar a primeira tarefa",
    hint: "Escolha as pastas e o destino; a prévia mostra o que vai acontecer.",
    to: "/tarefas",
  },
]);
const setupDone = computed(() => steps.value.every((s) => s.done));

/** O que pede atenção numa tarefa ativa, em uma frase (`null` = nada). */
function attention(job: JobSummary): string | null {
  if (!job.enabled) return null;
  if (job.sync?.scheduleState === "not_installed") {
    return "está com o agendamento desligado: o systemd não aceitou o timer (veja os logs).";
  }
  switch (job.lastRun?.status) {
    case "failed":
      return "falhou na última execução.";
    case "aborted":
      return "foi bloqueada na última execução por uma proteção.";
    case "awaiting_confirmation":
      return "aguarda a sua confirmação: execute-a uma vez manualmente.";
    case "partial_success":
      return "terminou com falhas em alguns itens.";
    default:
      return null;
  }
}

const needAttention = computed(() => store.jobs.filter((j) => attention(j)));

const lastRun = computed(() =>
  store.jobs
    .filter((j) => j.lastRun)
    .sort((a, b) => (b.lastRun!.startedAt > a.lastRun!.startedAt ? 1 : -1))[0],
);
const nextRun = computed(() =>
  store.jobs
    .filter((j) => j.sync?.nextRun)
    .sort((a, b) => (a.sync!.nextRun! > b.sync!.nextRun! ? 1 : -1))[0],
);

const headline = computed(() => {
  const n = needAttention.value.length;
  if (!store.jobs.length) return { icon: "info" as IconName, tone: "neutral", text: "Nenhuma tarefa ainda." };
  if (n === 1) return { icon: "alert" as IconName, tone: "warn", text: "1 tarefa precisa de atenção." };
  if (n > 1) return { icon: "alert" as IconName, tone: "warn", text: `${n} tarefas precisam de atenção.` };
  return { icon: "check" as IconName, tone: "ok", text: "Tudo em dia." };
});

function jobIcon(job: JobSummary): { icon: IconName; tone: string } {
  if (!job.lastRun) return { icon: "circle", tone: "neutral" };
  const tone = runTone(job.lastRun.status);
  const icon: IconName = tone === "ok" ? "check" : tone === "danger" ? "error" : "alert";
  return { icon, tone };
}

function destination(job: JobSummary): string {
  const sync = job.sync;
  if (!sync) return "sem destino";
  const where = `${sync.accountLabel ?? "conta removida"}: ${sync.path}`;
  return sync.delivery.kind === "receive"
    ? `Recebe de ${where}`
    : `${deliveryLabel(sync)} para ${where}`;
}
</script>

<template>
  <header class="page-head">
    <div>
      <h1>Visão geral</h1>
    </div>
  </header>

  <Notice v-if="store.info?.configProblem" kind="danger">
    Configuração com problema: {{ store.info.configProblem }}
  </Notice>
  <Notice v-if="store.info?.googleClientProblem" kind="warn">
    {{ store.info.googleClientProblem }}
  </Notice>

  <section class="health" :class="`tone-${headline.tone}`" aria-live="polite">
    <Icon class="health-icon" :name="headline.icon" :size="34" />
    <div>
      <p class="health-text">{{ headline.text }}</p>
      <dl v-if="store.jobs.length" class="health-facts">
        <template v-if="needAttention.length">
          <div v-for="job in needAttention" :key="job.id" class="attention">
            <dt class="visually-hidden">{{ job.name }}</dt>
            <dd>
              <RouterLink to="/tarefas"><strong>{{ job.name }}</strong></RouterLink>
              {{ attention(job) }}
            </dd>
          </div>
        </template>
        <template v-else>
          <div v-if="lastRun?.lastRun">
            <dt>Última execução</dt>
            <dd>{{ lastRun.name }}, {{ formatRelative(lastRun.lastRun.startedAt) }}</dd>
          </div>
          <div v-if="nextRun?.sync?.nextRun">
            <dt>Próxima</dt>
            <dd>{{ nextRun.name }}, {{ formatRelative(nextRun.sync.nextRun) }}</dd>
          </div>
        </template>
      </dl>
    </div>
  </section>

  <section v-if="!setupDone" class="section">
    <div class="section-head"><h2>Primeiros passos</h2></div>
    <div class="boxed">
      <component
        :is="step.to && !step.done ? RouterLink : 'div'"
        v-for="step in steps"
        :key="step.title"
        :to="step.to ?? undefined"
        class="item"
      >
        <Icon
          class="item-icon"
          :class="{ 'step-done': step.done }"
          :name="step.done ? 'check' : 'circle'"
          :size="22"
        />
        <div class="item-main">
          <div class="item-title">{{ step.title }}</div>
          <div class="item-sub">{{ step.hint }}</div>
        </div>
        <Icon v-if="step.to && !step.done" class="item-icon" name="chevron" />
      </component>
    </div>
  </section>

  <section v-if="store.jobs.length" class="section">
    <div class="section-head">
      <h2>Tarefas</h2>
      <RouterLink class="btn btn-flat btn-small" to="/tarefas">Abrir tarefas</RouterLink>
    </div>
    <div class="boxed">
      <RouterLink v-for="job in store.jobs" :key="job.id" to="/tarefas" class="item">
        <Icon
          class="item-icon"
          :class="`tone-${jobIcon(job).tone}`"
          :name="jobIcon(job).icon"
          :size="22"
        />
        <div class="item-main">
          <div class="item-title">
            {{ job.name }}
            <span v-if="!job.enabled" class="chip">Desativada</span>
          </div>
          <div class="item-sub ellipsis">{{ destination(job) }}</div>
        </div>
        <div class="item-end muted small">
          <span v-if="job.sync?.nextRun">Próxima {{ formatRelative(job.sync.nextRun) }}</span>
          <span v-else-if="job.lastRun">{{ formatRelative(job.lastRun.startedAt) }}</span>
          <span v-else>Nunca executada</span>
          <Icon name="chevron" />
        </div>
      </RouterLink>
    </div>
  </section>

  <details class="section app-paths">
    <summary>Onde o Syncly guarda os dados</summary>
    <dl class="facts">
      <div class="wide">
        <dt>Configuração</dt>
        <dd class="path">{{ store.info?.configFile }}</dd>
      </div>
      <div class="wide">
        <dt>Estado e histórico</dt>
        <dd class="path">{{ store.info?.dataDir }}</dd>
      </div>
      <div class="wide">
        <dt>Logs</dt>
        <dd class="path">{{ store.info?.logDir }}</dd>
      </div>
    </dl>
  </details>
</template>

<style scoped>
/* O elemento marcante da tela: uma frase com o estado dos backups. */
.health {
  display: flex;
  align-items: flex-start;
  gap: 18px;
  padding: 26px 28px;
  border-radius: var(--radius-l);
  background: var(--view);
  border: 1px solid var(--line);
}

.health-icon {
  flex: none;
  margin-top: 2px;
}

.health-text {
  font-size: 1.75rem;
  font-weight: 600;
  letter-spacing: -0.015em;
  line-height: 1.2;
}

.health-facts {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 32px;
  margin: 12px 0 0;
}

.health-facts dt {
  color: var(--fg-2);
  font-size: 0.8rem;
}

.health-facts dd {
  margin: 2px 0 0;
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

.health .health-text,
.health dd {
  color: var(--fg);
}

.attention {
  flex-basis: 100%;
}

.attention a {
  color: var(--fg);
}

.step-done {
  color: var(--success);
}

.item-title .chip {
  margin-left: 8px;
}

.app-paths {
  margin-top: 40px;
  padding: 0 4px;
  font-size: 0.9rem;
}
</style>
