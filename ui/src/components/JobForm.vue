<script setup lang="ts">
import { computed, reactive } from "vue";
import { RouterLink } from "vue-router";
import { pickFiles, pickFolder } from "../api";
import type {
  AccountSummary,
  ArchiveFormat,
  Delivery,
  JobDraft,
  JobSummary,
  Retention,
  Schedule,
  SyncMode,
  Weekday,
} from "../api/types";
import { archiveFormats, everyHours, weekdays } from "../utils/format";
import Icon from "./Icon.vue";

const props = defineProps<{ job: JobSummary | null; accounts: AccountSummary[] }>();
const emit = defineEmits<{ save: [draft: JobDraft]; cancel: [] }>();

// RN-54: mesmos padrões do core.
const DEFAULT_RETENTION: Retention = { keep_last: 3, daily: 7, weekly: 4, monthly: 6 };
const delivery = props.job?.sync?.delivery;
const archived = delivery?.kind === "archive" ? delivery : null;
const schedule: Schedule = props.job?.sync?.scheduleSpec ?? { kind: "manual" };

const form = reactive({
  name: props.job?.name ?? "",
  enabled: props.job?.enabled ?? true,
  sources: [...(props.job?.sources ?? [])],
  exclude: (props.job?.exclude ?? []).join("\n"),
  includeSensitive: props.job?.includeSensitive ?? false,
  accountId:
    props.job?.sync?.accountId ?? props.accounts[0]?.id ?? "",
  path: props.job?.sync?.path ?? "",
  mode: (props.job?.sync?.mode ?? "upload") as SyncMode,
  delivery: (delivery?.kind ?? "files") as Delivery["kind"],
  format: (archived?.format ?? "zip") as ArchiveFormat,
  retention: { ...(archived?.retention ?? DEFAULT_RETENTION) },
  /** RN-107: pasta do computador onde as versões recebidas são descompactadas. */
  target: delivery?.kind === "receive" ? delivery.target : "",
  scheduleKind: schedule.kind as Schedule["kind"],
  hours: schedule.kind === "every_hours" ? schedule.hours : 6,
  at: schedule.kind === "daily" || schedule.kind === "weekly" ? schedule.at : "02:00",
  weekday: (schedule.kind === "weekly" ? schedule.weekday : "sun") as Weekday,
  expr: schedule.kind === "calendar" ? schedule.expr : "",
});

const receiving = computed(() => form.delivery === "receive");

const usableAccounts = computed(() => props.accounts);
const scheduleValid = computed(() => {
  switch (form.scheduleKind) {
    case "daily":
    case "weekly":
      return /^\d{2}:\d{2}$/.test(form.at);
    case "calendar":
      return form.expr.trim().length > 0 && form.expr.length <= 64;
    default:
      return true;
  }
});

const retentionValid = computed(() => {
  const r = form.retention;
  const counts = [r.keep_last, r.daily, r.weekly, r.monthly];
  return counts.every((n) => Number.isInteger(n) && n >= 0 && n <= 1000) && r.keep_last >= 1;
});
const canSave = computed(
  () =>
    form.name.trim() &&
    form.accountId &&
    form.path.trim() &&
    (receiving.value ? !!form.target : form.sources.length > 0) &&
    (form.delivery !== "archive" || retentionValid.value) &&
    scheduleValid.value,
);

function addSources(paths: string[]) {
  for (const p of paths) if (!form.sources.includes(p)) form.sources.push(p);
  if (!form.path && form.name) form.path = form.name;
}

async function addFolder() {
  const folder = await pickFolder("Adicionar pasta");
  if (folder) addSources([folder]);
}

async function addFiles() {
  addSources(await pickFiles("Adicionar arquivos"));
}

async function chooseTarget() {
  form.target = (await pickFolder("Descompactar as versões recebidas em…")) ?? form.target;
}

function scheduleDraft(): Schedule {
  switch (form.scheduleKind) {
    case "every_hours":
      return { kind: "every_hours", hours: form.hours };
    case "daily":
      return { kind: "daily", at: form.at };
    case "weekly":
      return { kind: "weekly", weekday: form.weekday, at: form.at };
    case "calendar":
      return { kind: "calendar", expr: form.expr.trim() };
    default:
      return { kind: "manual" };
  }
}

function deliveryDraft(): Delivery {
  switch (form.delivery) {
    case "archive":
      return { kind: "archive", format: form.format, retention: { ...form.retention } };
    case "receive":
      return { kind: "receive", target: form.target };
    default:
      return { kind: "files" };
  }
}

function submit() {
  emit("save", {
    id: props.job?.id ?? null,
    name: form.name,
    enabled: form.enabled,
    // RN-110: quem recebe não tem fontes.
    sources: receiving.value ? [] : form.sources,
    exclude: receiving.value ? [] : form.exclude.split("\n"),
    includeSensitive: receiving.value ? false : form.includeSensitive,
    sync: {
      accountId: form.accountId,
      path: form.path,
      delivery: deliveryDraft(),
      mode: form.mode,
      schedule: scheduleDraft(),
    },
  });
}
</script>

<template>
  <form class="job-form boxed" @submit.prevent="submit">
    <header class="form-head">
      <h2>{{ job ? `Editar "${job.name}"` : "Nova tarefa" }}</h2>
      <button
        class="btn btn-flat btn-icon"
        type="button"
        aria-label="Fechar sem salvar"
        @click="emit('cancel')"
      >
        <Icon name="close" />
      </button>
    </header>

    <div class="form form-body">
      <div class="fields">
        <label class="field">
          <span class="label">Nome</span>
          <input v-model="form.name" maxlength="64" placeholder="Ex.: Documentos" required />
        </label>
        <label class="field switch-field">
          <span class="label">Tarefa ativa</span>
          <input v-model="form.enabled" class="switch" type="checkbox" role="switch" />
        </label>
      </div>

      <fieldset class="group">
        <legend>O que a tarefa faz</legend>
        <div class="boxed">
          <label class="option">
            <input v-model="form.delivery" type="radio" value="files" />
            <span class="option-main">
              <span class="option-title">Enviar arquivo por arquivo</span>
              <span class="option-desc">
                O destino fica igual às pastas; da segunda vez em diante, só vai o que mudou.
              </span>
            </span>
          </label>
          <label class="option">
            <input v-model="form.delivery" type="radio" value="archive" />
            <span class="option-main">
              <span class="option-title">Enviar compactado</span>
              <span class="option-desc">
                Tudo vira um arquivo com data no nome, guardando versões anteriores. Bom para
                pastas com muitos arquivos pequenos.
              </span>
            </span>
          </label>
          <label class="option">
            <input v-model="form.delivery" type="radio" value="receive" />
            <span class="option-main">
              <span class="option-title">Receber e descompactar</span>
              <span class="option-desc">
                Baixa a versão mais recente de uma pasta do destino e a descompacta numa pasta
                nova aqui. Nada existente é substituído.
              </span>
            </span>
          </label>
        </div>
      </fieldset>

      <div v-if="!receiving" class="group">
        <span class="group-title">Fontes</span>
        <div class="boxed">
          <div v-for="(src, i) in form.sources" :key="src" class="item source">
            <Icon class="item-icon" name="folder" />
            <span class="item-main path">{{ src }}</span>
            <button
              class="btn btn-flat btn-small btn-icon"
              type="button"
              :aria-label="`Remover ${src}`"
              @click="form.sources.splice(i, 1)"
            >
              <Icon name="close" :size="16" />
            </button>
          </div>
          <div class="item source-actions">
            <span v-if="!form.sources.length" class="item-main muted small">
              Escolha as pastas e os arquivos que vão para o destino.
            </span>
            <span v-else class="item-main"></span>
            <button class="btn btn-small" type="button" @click="addFolder">
              <Icon name="plus" :size="16" />
              Pasta
            </button>
            <button class="btn btn-small" type="button" @click="addFiles">
              <Icon name="plus" :size="16" />
              Arquivos
            </button>
          </div>
        </div>
      </div>

      <div class="group">
        <span class="group-title">{{ receiving ? "De onde receber" : "Destino" }}</span>
        <div class="fields">
          <label class="field">
            <span class="label">{{ receiving ? "Conta de origem" : "Conta" }}</span>
            <span class="select">
              <select v-model="form.accountId" required>
                <option v-if="!usableAccounts.length" value="" disabled>
                  Nenhuma conta disponível
                </option>
                <option v-for="a in usableAccounts" :key="a.id" :value="a.id">{{ a.label }}</option>
              </select>
            </span>
            <span v-if="!usableAccounts.length" class="hint">
              Conecte o Google Drive ou adicione uma pasta em
              <RouterLink to="/contas">Contas</RouterLink>.
            </span>
          </label>
          <label class="field">
            <span class="label">{{ receiving ? "Pasta onde estão as versões" : "Pasta no destino" }}</span>
            <input v-model="form.path" placeholder="Ex.: Backup/Documentos" required />
          </label>
        </div>
        <div v-if="receiving" class="field">
          <span class="label">Pasta no computador</span>
          <div class="row">
            <button class="btn btn-small" type="button" @click="chooseTarget">
              <Icon name="folder" :size="16" />
              Escolher pasta…
            </button>
            <span v-if="form.target" class="path">{{ form.target }}</span>
            <span v-else class="hint">Nenhuma pasta escolhida.</span>
          </div>
          <span class="hint">
            Cada versão recebida vira uma pasta nova aqui dentro, com o nome da versão.
          </span>
        </div>
      </div>

      <div v-if="form.delivery === 'archive'" class="group">
        <span class="group-title">Versões</span>
        <div class="fields">
          <label class="field">
            <span class="label">Formato</span>
            <span class="select">
              <select v-model="form.format">
                <option v-for="f in archiveFormats" :key="f.value" :value="f.value">
                  {{ f.label }}: {{ f.hint }}
                </option>
              </select>
            </span>
          </label>
        </div>
        <div class="fields retention">
          <label class="field">
            <span class="label">Últimas</span>
            <input v-model.number="form.retention.keep_last" type="number" min="1" max="1000" />
          </label>
          <label class="field">
            <span class="label">Diárias</span>
            <input v-model.number="form.retention.daily" type="number" min="0" max="1000" />
          </label>
          <label class="field">
            <span class="label">Semanais</span>
            <input v-model.number="form.retention.weekly" type="number" min="0" max="1000" />
          </label>
          <label class="field">
            <span class="label">Mensais</span>
            <input v-model.number="form.retention.monthly" type="number" min="0" max="1000" />
          </label>
        </div>
        <span v-if="!retentionValid" class="error-text">Mantenha pelo menos a última versão.</span>
        <span v-else class="hint">
          As que passarem disso vão para a lixeira do destino, só depois que a versão nova for
          enviada e conferida. Se nada mudou, nada é enviado.
        </span>
      </div>

      <fieldset v-if="form.delivery === 'files'" class="group">
        <legend>Modo</legend>
        <div class="boxed">
          <label class="option">
            <input v-model="form.mode" type="radio" value="upload" />
            <span class="option-main">
              <span class="option-title">Envio (recomendado)</span>
              <span class="option-desc">Envia novos e alterados; nunca apaga nada no destino.</span>
            </span>
          </label>
          <label class="option">
            <input v-model="form.mode" type="radio" value="mirror" />
            <span class="option-main">
              <span class="option-title">Espelho</span>
              <span class="option-desc">
                O que você apagar aqui vai para a lixeira do destino, com proteção contra
                exclusões em massa.
              </span>
            </span>
          </label>
        </div>
      </fieldset>

      <div class="group">
        <span class="group-title">Agendamento</span>
        <div class="row schedule">
          <span class="select">
            <select v-model="form.scheduleKind" aria-label="Quando executar">
              <option value="manual">Só quando eu clicar em Executar</option>
              <option value="every_hours">A cada…</option>
              <option value="daily">Todo dia</option>
              <option value="weekly">Toda semana</option>
              <option value="calendar">Personalizado (expressão do systemd)</option>
            </select>
          </span>
          <span v-if="form.scheduleKind === 'every_hours'" class="select">
            <select v-model.number="form.hours" aria-label="Intervalo">
              <option v-for="h in everyHours" :key="h" :value="h">
                {{ h === 1 ? "1 hora" : `${h} horas` }}
              </option>
            </select>
          </span>
          <span v-if="form.scheduleKind === 'weekly'" class="select">
            <select v-model="form.weekday" aria-label="Dia da semana">
              <option v-for="d in weekdays" :key="d.value" :value="d.value">{{ d.label }}</option>
            </select>
          </span>
          <template v-if="form.scheduleKind === 'daily' || form.scheduleKind === 'weekly'">
            <span class="muted">às</span>
            <input v-model="form.at" class="time" type="time" aria-label="Horário" required />
          </template>
          <input
            v-if="form.scheduleKind === 'calendar'"
            v-model="form.expr"
            class="expr"
            maxlength="64"
            placeholder="Ex.: Mon..Fri *-*-* 18:30:00"
            aria-label="Expressão OnCalendar"
          />
        </div>
        <span v-if="form.scheduleKind !== 'manual'" class="hint">
          Roda sozinho pelo systemd, mesmo com o app fechado; se o computador estiver desligado no
          horário, roda ao ligar. Começa depois da primeira execução manual bem-sucedida e nunca
          confirma nada sozinho: se precisar, avisa por notificação.
        </span>
      </div>

      <div v-if="!receiving" class="group">
        <span class="group-title">Avançado</span>
        <label class="field">
          <span class="label">Exclusões (uma por linha, como no .gitignore)</span>
          <textarea v-model="form.exclude" rows="3" placeholder="node_modules/&#10;*.iso" />
        </label>
        <label class="field switch-field">
          <span>
            <span class="label">Incluir arquivos sensíveis</span>
            <span class="hint">
              <code>.ssh</code>, <code>.env</code>, chaves e afins. Não recomendado.
            </span>
          </span>
          <input v-model="form.includeSensitive" class="switch" type="checkbox" role="switch" />
        </label>
      </div>
    </div>

    <footer class="form-foot">
      <button class="btn btn-flat" type="button" @click="emit('cancel')">Cancelar</button>
      <button class="btn btn-primary" type="submit" :disabled="!canSave">
        {{ job ? "Salvar alterações" : "Criar tarefa" }}
      </button>
    </footer>
  </form>
</template>

<style scoped>
.form-head,
.form-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px;
}

.form-foot {
  justify-content: flex-end;
}

.boxed > .form-body {
  border-top: 0;
}

.form-body {
  padding: 4px 18px 22px;
}

.switch-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 0 4px;
}

.switch-field > span {
  display: grid;
  gap: 2px;
}

.source {
  min-height: 48px;
  padding: 8px 10px 8px 18px;
}

.source-actions {
  min-height: 52px;
  padding: 8px 10px 8px 18px;
}

.retention {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.schedule .select {
  min-width: 180px;
}

.schedule .time {
  width: 120px;
}

.schedule .expr {
  flex: 1;
  min-width: 220px;
  font-family: var(--mono);
}

@media (max-width: 760px) {
  .retention {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
