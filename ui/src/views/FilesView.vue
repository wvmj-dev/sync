<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RouterLink } from "vue-router";
import { api, errorMessage, pickFiles, pickFolder } from "../api";
import type { RemoteItem } from "../api/types";
import { useAppStore } from "../stores/app";
import { formatBytes, formatDate } from "../utils/format";

const store = useAppStore();

const accountId = ref<string>("");
/** Pasta atual, relativa a Syncly/ ("" = raiz). */
const path = ref("");
const items = ref<RemoteItem[]>([]);
const loading = ref(false);
const busy = ref<string | null>(null);
const notice = ref<{ kind: "ok" | "danger"; text: string } | null>(null);

const crumbs = computed(() => {
  const parts = path.value ? path.value.split("/") : [];
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
});

// Seleciona a primeira conta (de preferência Google) assim que as contas carregarem.
watch(
  () => store.accounts,
  (accounts) => {
    if (!accounts.some((a) => a.id === accountId.value)) {
      accountId.value = (accounts.find((a) => a.provider === "google_drive") ?? accounts[0])?.id ?? "";
    }
  },
  { immediate: true },
);

watch([accountId, path], () => load(), { immediate: true });

async function load() {
  if (!accountId.value) return;
  loading.value = true;
  try {
    items.value = await api.listRemote(accountId.value, path.value);
  } catch (e) {
    items.value = [];
    notice.value = { kind: "danger", text: errorMessage(e) };
  } finally {
    loading.value = false;
  }
}

function open(folder: string) {
  notice.value = null;
  path.value = folder;
}

function changeAccount() {
  notice.value = null;
  path.value = "";
}

async function upload() {
  const files = await pickFiles("Enviar para Syncly/" + path.value);
  for (const file of files) {
    busy.value = `Enviando ${file.split("/").pop()}…`;
    try {
      const item = await api.uploadFile(accountId.value, file, path.value);
      notice.value = { kind: "ok", text: `Enviado e verificado: Syncly/${item.path}` };
    } catch (e) {
      notice.value = { kind: "danger", text: errorMessage(e) };
      break;
    }
  }
  busy.value = null;
  await load();
}

async function download(item: RemoteItem) {
  const dir = await pickFolder(`Salvar "${item.name}" em…`);
  if (!dir) return;
  busy.value = `Baixando ${item.name}…`;
  try {
    const saved = await api.downloadFile(accountId.value, item.path, dir);
    notice.value = { kind: "ok", text: `Baixado e verificado: ${saved}` };
  } catch (e) {
    notice.value = { kind: "danger", text: errorMessage(e) };
  } finally {
    busy.value = null;
  }
}
</script>

<template>
  <header class="page-head">
    <h1>Arquivos</h1>
  </header>

  <p v-if="!store.accounts.length" class="card muted">
    Nenhuma conta ainda. <RouterLink to="/contas">Conecte o Google Drive</RouterLink> ou adicione
    uma pasta local.
  </p>

  <section v-else class="card">
    <div class="files-toolbar">
      <select v-model="accountId" aria-label="Conta" @change="changeAccount">
        <option v-for="a in store.accounts" :key="a.id" :value="a.id">
          {{ a.label }} ({{ a.provider === "google_drive" ? "Google Drive" : "local" }})
        </option>
      </select>
      <div class="row">
        <button class="btn" type="button" :disabled="loading || !!busy" @click="load">Atualizar</button>
        <button class="btn btn-primary" type="button" :disabled="!!busy" @click="upload">
          Enviar arquivo…
        </button>
      </div>
    </div>

    <nav class="crumbs" aria-label="Pasta atual">
      <button class="btn btn-ghost" type="button" @click="open('')">Syncly</button>
      <template v-for="c in crumbs" :key="c.path">
        <span class="muted">/</span>
        <button class="btn btn-ghost" type="button" @click="open(c.path)">{{ c.name }}</button>
      </template>
    </nav>

    <div v-if="notice" class="banner" :class="`banner-${notice.kind}`">
      <span>{{ notice.text }}</span>
      <button class="btn btn-ghost" type="button" @click="notice = null">Fechar</button>
    </div>
    <p v-if="busy" class="muted">{{ busy }}</p>

    <p v-if="loading" class="muted">Carregando…</p>
    <p v-else-if="!items.length" class="muted">Pasta vazia. Use "Enviar arquivo…" para começar.</p>
    <table v-else class="table">
      <thead>
        <tr>
          <th>Nome</th>
          <th>Tamanho</th>
          <th>Modificado</th>
          <th>Por</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in items" :key="item.path">
          <td>
            <button
              v-if="item.kind === 'folder'"
              class="btn btn-ghost folder-link"
              type="button"
              @click="open(item.path)"
            >
              📁 {{ item.name }}
            </button>
            <span v-else>{{ item.name }}</span>
          </td>
          <td>{{ item.size != null ? formatBytes(item.size) : "" }}</td>
          <td>{{ formatDate(item.modifiedAt) }}</td>
          <td class="muted">{{ item.modifiedBy ?? "" }}</td>
          <td>
            <button
              v-if="item.kind === 'file'"
              class="btn"
              type="button"
              :disabled="!item.downloadable || !!busy"
              :title="item.downloadable ? '' : 'Documentos nativos do Google não podem ser baixados'"
              @click="download(item)"
            >
              Baixar
            </button>
          </td>
        </tr>
      </tbody>
    </table>
  </section>
</template>
