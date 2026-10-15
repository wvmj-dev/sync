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
  SyncMode,
} from "../api/types";
import { archiveFormats } from "../utils/format";

const props = defineProps<{ job: JobSummary | null; accounts: AccountSummary[] }>();
const emit = defineEmits<{ save: [draft: JobDraft]; cancel: [] }>();

// RN-54: mesmos padrões do core.
const DEFAULT_RETENTION: Retention = { keep_last: 3, daily: 7, weekly: 4, monthly: 6 };
const delivery = props.job?.sync?.delivery;
const archived = delivery?.kind === "archive" ? delivery : null;

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
});

const usableAccounts = computed(() => props.accounts);
const retentionValid = computed(() => {
  const r = form.retention;
  const counts = [r.keep_last, r.daily, r.weekly, r.monthly];
  return counts.every((n) => Number.isInteger(n) && n >= 0 && n <= 1000) && r.keep_last >= 1;
});
const canSave = computed(
  () =>
    form.name.trim() &&
    form.sources.length > 0 &&
    form.accountId &&
    form.path.trim() &&
    (form.delivery === "files" || retentionValid.value),
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

function submit() {
  emit("save", {
    id: props.job?.id ?? null,
    name: form.name,
    enabled: form.enabled,
    sources: form.sources,
    exclude: form.exclude.split("\n"),
    includeSensitive: form.includeSensitive,
    sync: {
      accountId: form.accountId,
      path: form.path,
      delivery:
        form.delivery === "archive"
          ? { kind: "archive", format: form.format, retention: { ...form.retention } }
          : { kind: "files" },
      mode: form.mode,
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

    <div class="field">
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
        <span>Conta de destino</span>
        <select v-model="form.accountId" required>
          <option v-if="!usableAccounts.length" value="" disabled>Nenhuma conta disponível</option>
          <option v-for="a in usableAccounts" :key="a.id" :value="a.id">{{ a.label }}</option>
        </select>
        <small v-if="!usableAccounts.length" class="muted">
          Conecte o Google Drive ou adicione uma pasta em <RouterLink to="/contas">Contas</RouterLink>.
        </small>
      </label>
      <label class="field">
        <span>Pasta no destino</span>
        <input v-model="form.path" placeholder="Ex.: Backup/Documentos" required />
      </label>
    </div>

    <fieldset class="field">
      <legend>Como enviar</legend>
      <label class="choice">
        <input v-model="form.delivery" type="radio" value="files" />
        <span><strong>Arquivo por arquivo</strong> — o destino fica igual às pastas; da segunda vez em diante, só vai o que mudou.</span>
      </label>
      <label class="choice">
        <input v-model="form.delivery" type="radio" value="archive" />
        <span>
          <strong>Compactado</strong> — tudo vira um único arquivo com data no nome
          (ex.: <code>{{ form.name || "Tarefa" }}_2026-10-14_02-00-05.zip</code>), mantendo versões
          anteriores. Bom para backup de pastas com muitos arquivos pequenos.
        </span>
      </label>
    </fieldset>

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

    <fieldset v-else class="field">
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
