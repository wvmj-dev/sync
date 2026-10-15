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
  <form class="card form" @submit.prevent="submit">
    <h2>{{ job ? "Editar tarefa" : "Nova tarefa" }}</h2>

    <label class="field">
      <span>Nome</span>
      <input v-model="form.name" maxlength="64" placeholder="Ex.: Documentos" required />
    </label>

    <fieldset class="field">
      <legend>O que a tarefa faz</legend>
      <label class="choice">
        <input v-model="form.delivery" type="radio" value="files" />
        <span>
          <strong>Enviar arquivo por arquivo</strong> — o destino fica igual às pastas; da segunda
          vez em diante, só vai o que mudou.
        </span>
      </label>
      <label class="choice">
        <input v-model="form.delivery" type="radio" value="archive" />
        <span>
          <strong>Enviar compactado</strong> — tudo vira um único arquivo com data no nome
          (ex.: <code>{{ form.name || "Tarefa" }}_2026-10-14_02-00-05.zip</code>), mantendo versões
          anteriores. Bom para backup de pastas com muitos arquivos pequenos.
        </span>
      </label>
      <label class="choice">
        <input v-model="form.delivery" type="radio" value="receive" />
        <span>
          <strong>Receber e descompactar</strong> — baixa a versão mais recente de uma pasta do
          destino (enviada pelo Syncly, deste ou de outro computador) e a descompacta numa
          <strong>pasta nova</strong> aqui. Nada existente é substituído.
        </span>
      </label>
    </fieldset>

    <div v-if="!receiving" class="field">
      <span>Fontes (pastas e arquivos)</span>
      <ul v-if="form.sources.length" class="list compact">
        <li v-for="(src, i) in form.sources" :key="src" class="list-row">
          <code>{{ src }}</code>
          <button class="btn btn-ghost" type="button" @click="form.sources.splice(i, 1)">
            Remover
          </button>
        </li>
      </ul>
      <div class="row">
        <button class="btn" type="button" @click="addFolder">+ Pasta</button>
        <button class="btn" type="button" @click="addFiles">+ Arquivos</button>
      </div>
    </div>

    <div class="grid-2">
      <label class="field">
        <span>{{ receiving ? "Conta de origem" : "Conta de destino" }}</span>
        <select v-model="form.accountId" required>
          <option v-if="!usableAccounts.length" value="" disabled>Nenhuma conta disponível</option>
          <option v-for="a in usableAccounts" :key="a.id" :value="a.id">{{ a.label }}</option>
        </select>
        <small v-if="!usableAccounts.length" class="muted">
          Conecte o Google Drive ou adicione uma pasta em <RouterLink to="/contas">Contas</RouterLink>.
        </small>
      </label>
      <label class="field">
        <span>{{ receiving ? "Pasta no destino (onde estão as versões)" : "Pasta no destino" }}</span>
        <input v-model="form.path" placeholder="Ex.: Backup/Documentos" required />
      </label>
    </div>

    <div v-if="receiving" class="field">
      <span>Pasta no computador</span>
      <div class="row">
        <button class="btn" type="button" @click="chooseTarget">Escolher pasta…</button>
        <code v-if="form.target">{{ form.target }}</code>
        <span v-else class="muted small">nenhuma pasta escolhida</span>
      </div>
      <small class="muted">
        Cada versão recebida vira uma pasta nova aqui dentro, com o nome da versão. Se a mais
        recente já foi recebida, nada é feito.
      </small>
    </div>

    <template v-if="form.delivery === 'archive'">
      <label class="field">
        <span>Formato</span>
        <select v-model="form.format">
          <option v-for="f in archiveFormats" :key="f.value" :value="f.value">
            {{ f.label }} — {{ f.hint }}
          </option>
        </select>
      </label>
      <fieldset class="field">
        <legend>Versões mantidas no destino</legend>
        <div class="grid-4">
          <label class="field">
            <span>Últimas</span>
            <input v-model.number="form.retention.keep_last" type="number" min="1" max="1000" />
          </label>
          <label class="field">
            <span>Diárias</span>
            <input v-model.number="form.retention.daily" type="number" min="0" max="1000" />
          </label>
          <label class="field">
            <span>Semanais</span>
            <input v-model.number="form.retention.weekly" type="number" min="0" max="1000" />
          </label>
          <label class="field">
            <span>Mensais</span>
            <input v-model.number="form.retention.monthly" type="number" min="0" max="1000" />
          </label>
        </div>
        <small class="muted">
          As que passarem disso vão para a <em>lixeira</em> do destino (recuperáveis), só depois que
          a versão nova for enviada e conferida. Se nada mudou, nada é enviado.
        </small>
        <small v-if="!retentionValid" class="error">Mantenha ao menos a última versão.</small>
      </fieldset>
    </template>

    <fieldset v-else-if="form.delivery === 'files'" class="field">
      <legend>Modo</legend>
      <label class="choice">
        <input v-model="form.mode" type="radio" value="upload" />
        <span><strong>Envio</strong> (recomendado) — envia novos e alterados; nunca apaga nada no destino.</span>
      </label>
      <label class="choice">
        <input v-model="form.mode" type="radio" value="mirror" />
        <span>
          <strong>Espelho</strong> — o que você apagar aqui vai para a <em>lixeira</em> do destino,
          com proteção contra exclusões em massa.
        </span>
      </label>
    </fieldset>

    <template v-if="!receiving">
      <label class="field">
        <span>Exclusões (uma por linha, sintaxe do .gitignore)</span>
        <textarea v-model="form.exclude" rows="3" placeholder="node_modules/&#10;*.iso" />
      </label>

      <label class="choice">
        <input v-model="form.includeSensitive" type="checkbox" />
        <span>
          Incluir arquivos sensíveis (<code>.ssh</code>, <code>.env</code>, chaves…).
          <strong>Não recomendado.</strong>
        </span>
      </label>
    </template>

    <fieldset class="field">
      <legend>Agendamento</legend>
      <div class="row">
        <select v-model="form.scheduleKind" aria-label="Quando executar">
          <option value="manual">Só quando eu clicar em Executar</option>
          <option value="every_hours">A cada…</option>
          <option value="daily">Todo dia</option>
          <option value="weekly">Toda semana</option>
          <option value="calendar">Personalizado (systemd)</option>
        </select>
        <select v-if="form.scheduleKind === 'every_hours'" v-model.number="form.hours" aria-label="Intervalo">
          <option v-for="h in everyHours" :key="h" :value="h">{{ h === 1 ? "1 hora" : `${h} horas` }}</option>
        </select>
        <select v-if="form.scheduleKind === 'weekly'" v-model="form.weekday" aria-label="Dia da semana">
          <option v-for="d in weekdays" :key="d.value" :value="d.value">{{ d.label }}</option>
        </select>
        <template v-if="form.scheduleKind === 'daily' || form.scheduleKind === 'weekly'">
          <span>às</span>
          <input v-model="form.at" type="time" aria-label="Horário" required />
        </template>
        <input
          v-if="form.scheduleKind === 'calendar'"
          v-model="form.expr"
          maxlength="64"
          placeholder="Ex.: Mon..Fri *-*-* 18:30:00"
          aria-label="Expressão OnCalendar"
        />
      </div>
      <small v-if="form.scheduleKind !== 'manual'" class="muted">
        Roda sozinho pelo systemd, mesmo com o app fechado; se o computador estiver desligado no
        horário, roda ao ligar. Só começa depois da <strong>primeira execução manual</strong>
        bem-sucedida, e nunca confirma nada sozinho — se precisar, avisa por notificação.
      </small>
    </fieldset>

    <label class="choice">
      <input v-model="form.enabled" type="checkbox" />
      <span>Tarefa ativa</span>
    </label>

    <div class="row end">
      <button class="btn btn-ghost" type="button" @click="emit('cancel')">Cancelar</button>
      <button class="btn btn-primary" type="submit" :disabled="!canSave">Salvar</button>
    </div>
  </form>
</template>
