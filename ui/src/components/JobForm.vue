<script setup lang="ts">
import { computed, reactive } from "vue";
import { RouterLink } from "vue-router";
import { pickFiles, pickFolder } from "../api";
import type { AccountSummary, JobDraft, JobSummary, SyncMode } from "../api/types";

const props = defineProps<{ job: JobSummary | null; accounts: AccountSummary[] }>();
const emit = defineEmits<{ save: [draft: JobDraft]; cancel: [] }>();

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
});

const usableAccounts = computed(() => props.accounts);
const canSave = computed(
  () => form.name.trim() && form.sources.length > 0 && form.accountId && form.path.trim(),
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
    sync: { accountId: form.accountId, path: form.path, mode: form.mode },
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
