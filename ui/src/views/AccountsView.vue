<script setup lang="ts">
import { ref } from "vue";
import { confirmDanger, pickFolder } from "../api";
import { useAppStore } from "../stores/app";

const store = useAppStore();
const label = ref("");
const root = ref<string | null>(null);
const saving = ref(false);

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

async function remove(id: string, name: string, jobs: number) {
  const warning = jobs
    ? `\n\n${jobs} tarefa(s) usam esta conta e serão desativadas (não apagadas).`
    : "";
  const message = `Remover a conta "${name}"? Nenhum arquivo será apagado.${warning}`;
  if (await confirmDanger(message, "Remover")) {
    await store.removeAccount(id);
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
      <code>drive.file</code>). Fotos, documentos e o resto do seu Drive ficam inacessíveis.
    </p>
    <p v-if="store.info?.googleClientConfigured" class="ok">✓ Projeto do Google Cloud configurado.</p>
    <p v-else class="muted">
      Primeiro configure o projeto do Google Cloud — veja <code>docs/CONFIGURAR_GOOGLE.md</code>.
    </p>
    <button class="btn" type="button" disabled title="Disponível na fase 1">
      Conectar Google Drive (fase 1)
    </button>
  </section>

  <section class="card">
    <h2>Pastas locais</h2>
    <p class="muted">
      HD externo, NAS montado ou outra pasta. O Syncly cria um marcador
      <code>.syncly-root</code> nela: se o disco for desmontado, nada é gravado no lugar errado.
    </p>

    <ul v-if="store.accounts.length" class="list">
      <li v-for="account in store.accounts" :key="account.id" class="list-row">
        <div>
          <strong>{{ account.label }}</strong>
          <span class="badge">{{ account.provider === "local" ? "local" : "Google Drive" }}</span>
          <div class="muted small">{{ account.detail }} · {{ account.jobCount }} tarefa(s)</div>
        </div>
        <button
          class="btn btn-ghost"
          type="button"
          @click="remove(account.id, account.label, account.jobCount)"
        >
          Remover
        </button>
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
