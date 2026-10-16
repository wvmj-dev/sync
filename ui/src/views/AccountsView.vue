<script setup lang="ts">
import { computed, ref } from "vue";
import { RouterLink } from "vue-router";
import { confirmDanger, pickFolder } from "../api";
import type { AccountSummary } from "../api/types";
import Icon from "../components/Icon.vue";
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

function tasks(n: number): string {
  return n === 1 ? "1 tarefa" : `${n} tarefas`;
}
</script>

<template>
  <header class="page-head">
    <div>
      <h1>Contas</h1>
      <p class="page-desc">Para onde as tarefas enviam: o Google Drive ou uma pasta deste computador.</p>
    </div>
  </header>

  <section class="section first">
    <div class="section-head">
      <div>
        <h2>Google Drive</h2>
        <p class="section-desc">
          O Syncly só enxerga os arquivos que ele mesmo cria, dentro da pasta
          <code>Syncly/</code>. O resto do seu Drive fica inacessível.
        </p>
      </div>
    </div>
    <div class="boxed">
      <div v-for="account in googleAccounts" :key="account.id" class="item">
        <Icon class="item-icon" name="cloud" :size="22" />
        <div class="item-main">
          <div class="item-title">{{ account.label }}</div>
          <div class="item-sub">{{ tasks(account.jobCount) }}</div>
        </div>
        <div class="item-end">
          <RouterLink class="btn btn-flat btn-small" to="/arquivos">Ver arquivos</RouterLink>
          <button class="btn btn-danger btn-small" type="button" @click="remove(account)">
            Remover
          </button>
        </div>
      </div>
      <div v-if="store.info?.googleClientConfigured" class="item">
        <div class="item-main">
          <div class="item-sub">
            <template v-if="connecting">Autorize no navegador que abriu (até 5 minutos)…</template>
            <template v-else>
              O login abre no seu navegador. Enquanto o app estiver em "Testando" no Google Cloud,
              ele vale por 7 dias; depois é só reconectar.
            </template>
          </div>
        </div>
        <div class="item-end">
          <button class="btn btn-primary btn-small" type="button" :disabled="connecting" @click="connectGoogle">
            {{ googleAccounts.length ? "Reconectar ou outra conta" : "Conectar Google Drive" }}
          </button>
        </div>
      </div>
      <div v-else class="item">
        <Icon class="item-icon" name="info" :size="22" />
        <div class="item-main item-sub">
          Primeiro configure o projeto do Google Cloud, conforme
          <code>docs/CONFIGURAR_GOOGLE.md</code>.
        </div>
      </div>
    </div>
  </section>

  <section class="section">
    <div class="section-head">
      <div>
        <h2>Pastas locais</h2>
        <p class="section-desc">
          HD externo, NAS montado ou outra pasta. O Syncly deixa um marcador
          <code>.syncly-root</code> nela: com o disco desmontado, nada é gravado no lugar errado.
        </p>
      </div>
    </div>
    <div class="boxed">
      <div v-for="account in localAccounts" :key="account.id" class="item">
        <Icon class="item-icon" name="drive" :size="22" />
        <div class="item-main">
          <div class="item-title">{{ account.label }}</div>
          <div class="item-sub">
            <span class="path">{{ account.detail }}</span>
            <span class="count">{{ tasks(account.jobCount) }}</span>
          </div>
        </div>
        <div class="item-end">
          <button class="btn btn-danger btn-small" type="button" @click="remove(account)">
            Remover
          </button>
        </div>
      </div>
      <form class="item add-local" @submit.prevent="addLocal">
        <button class="btn btn-small" type="button" @click="chooseRoot">
          <Icon name="folder" :size="16" />
          Escolher pasta…
        </button>
        <span class="item-main path ellipsis">{{ root ?? "" }}</span>
        <input
          v-model="label"
          class="name"
          placeholder="Nome, ex.: HD externo"
          aria-label="Nome da conta"
          maxlength="64"
          required
        />
        <button class="btn btn-primary btn-small" type="submit" :disabled="!root || saving">
          Adicionar
        </button>
      </form>
    </div>
  </section>
</template>

<style scoped>
.section.first {
  margin-top: 0;
}

.count {
  margin-left: 12px;
}

.add-local {
  flex-wrap: wrap;
}

.add-local .name {
  width: 220px;
}
</style>
