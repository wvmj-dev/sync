<script setup lang="ts">
import { computed, ref } from "vue";
import { RouterLink } from "vue-router";
import { confirmDanger, pickFolder } from "../api";
import type { AccountSummary } from "../api/types";
import { useAppStore } from "../stores/app";

const store = useAppStore();
const label = ref("");
const root = ref<string | null>(null);
const saving = ref(false);
const connecting = ref(false);

const googleAccounts = computed(() => store.accounts.filter((a) => a.provider === "google_drive"));
const localAccounts = computed(() => store.accounts.filter((a) => a.provider === "local"));

async function connectGoogle() {
  connecting.value = true;
  await store.connectGoogle();
  connecting.value = false;
}

async function chooseRoot() {
  root.value = (await pickFolder("Pasta raiz da conta local")) ?? root.value;
  if (root.value && !label.value) label.value = root.value.split("/").pop() ?? "";
}

async function addLocal() {
  if (!root.value) return;
  saving.value = true;
  if (await store.addLocalAccount(label.value, root.value)) {
    label.value = "";
    root.value = null;
  }
  saving.value = false;
}

async function remove(account: AccountSummary) {
  const google =
    account.provider === "google_drive"
      ? "\n\nO acesso do Syncly à sua conta Google será revogado."
      : "";
  const jobs = account.jobCount
    ? `\n\n${account.jobCount} tarefa(s) usam esta conta e serão desativadas (não apagadas).`
    : "";
  const message = `Remover a conta "${account.label}"? Nenhum arquivo será apagado.${google}${jobs}`;
  if (await confirmDanger(message, "Remover")) {
    await store.removeAccount(account.id);
  }
}
</script>

<template>
  <header class="page-head">
    <h1>Contas</h1>
  </header>

  <section class="card">
    <h2>Google Drive</h2>
    <p>
      O app acessa <strong>apenas os arquivos que ele mesmo cria</strong> (escopo
      <code>drive.file</code>), dentro da pasta <code>Syncly/</code>. Fotos, documentos e o resto
      do seu Drive ficam inacessíveis.
    </p>

    <ul v-if="googleAccounts.length" class="list">
      <li v-for="account in googleAccounts" :key="account.id" class="list-row">
        <div>
          <strong>{{ account.label }}</strong>
          <span class="badge">Google Drive</span>
          <div class="muted small">{{ account.jobCount }} tarefa(s)</div>
        </div>
        <div class="row">
          <RouterLink class="btn btn-ghost" to="/arquivos">Arquivos</RouterLink>
          <button class="btn btn-ghost" type="button" @click="remove(account)">Remover</button>
        </div>
      </li>
    </ul>

    <template v-if="store.info?.googleClientConfigured">
      <div class="row">
        <button class="btn btn-primary" type="button" :disabled="connecting" @click="connectGoogle">
          {{ googleAccounts.length ? "Reconectar / outra conta" : "Conectar Google Drive" }}
        </button>
        <span v-if="connecting" class="muted small">
          Autorize no navegador que abriu (até 5 minutos)…
        </span>
      </div>
      <p class="muted small">
        Enquanto o app estiver em "Testando" no Google Cloud, o login vale por 7 dias; depois é
        só reconectar.
      </p>
    </template>
    <p v-else class="muted">
      Primeiro configure o projeto do Google Cloud — veja <code>docs/CONFIGURAR_GOOGLE.md</code>.
    </p>
  </section>

  <section class="card">
    <h2>Pastas locais</h2>
    <p class="muted">
      HD externo, NAS montado ou outra pasta. O Syncly cria um marcador
      <code>.syncly-root</code> nela: se o disco for desmontado, nada é gravado no lugar errado.
    </p>

    <ul v-if="localAccounts.length" class="list">
      <li v-for="account in localAccounts" :key="account.id" class="list-row">
        <div>
          <strong>{{ account.label }}</strong>
          <span class="badge">local</span>
          <div class="muted small">{{ account.detail }} · {{ account.jobCount }} tarefa(s)</div>
        </div>
        <button class="btn btn-ghost" type="button" @click="remove(account)">Remover</button>
      </li>
    </ul>

    <form class="inline-form" @submit.prevent="addLocal">
      <button class="btn" type="button" @click="chooseRoot">Escolher pasta…</button>
      <span class="muted small path-preview">{{ root ?? "nenhuma pasta escolhida" }}</span>
      <input v-model="label" placeholder="Nome (ex.: HD externo)" maxlength="64" required />
      <button class="btn btn-primary" type="submit" :disabled="!root || saving">Adicionar</button>
    </form>
  </section>
</template>
