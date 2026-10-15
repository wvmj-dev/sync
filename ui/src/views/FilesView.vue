<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { RouterLink } from "vue-router";
import { api, errorMessage, onArchiveProgress, pickFiles, pickFolder } from "../api";
import type { ArchiveFormat, ArchiveProgress, RemoteItem } from "../api/types";
import { useAppStore } from "../stores/app";
import { archiveFormats, formatBytes, formatDate } from "../utils/format";

const store = useAppStore();

const accountId = ref<string>("");
/** Pasta atual, relativa a Syncly/ ("" = raiz). */
const path = ref("");
const items = ref<RemoteItem[]>([]);
const loading = ref(false);
const busy = ref<string | null>(null);
const notice = ref<{ kind: "ok" | "warning" | "danger"; text: string } | null>(null);
/** Compactar/descompactar em andamento: mostra progresso e permite cancelar. */
const packing = ref(false);
const progress = ref<ArchiveProgress | null>(null);
const format = ref<ArchiveFormat>("zip");

const percent = computed(() => {
  const p = progress.value;
  return p && p.bytesTotal > 0 ? Math.min(100, (p.bytesDone / p.bytesTotal) * 100) : null;
});

let stopListening: (() => void) | null = null;
onMounted(async () => {
  stopListening = await onArchiveProgress((p) => {
    if (packing.value) progress.value = p;
  });
});
onUnmounted(() => stopListening?.());

const ARCHIVE_NAME = /\.(zip|tar|tar\.gz|tgz|tar\.zst|tzst)$/i;

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

/** Roda uma operação de compactação com progresso; erros viram aviso. */
async function packed<T>(label: string, op: () => Promise<T>): Promise<T | null> {
  busy.value = label;
  packing.value = true;
  progress.value = null;
  notice.value = null;
  try {
    return await op();
  } catch (e) {
    notice.value = { kind: "danger", text: errorMessage(e) };
    return null;
  } finally {
    busy.value = null;
    packing.value = false;
  }
}

/** RN-109: a pasta vira um único arquivo; nunca sobrescreve; sensíveis ficam de fora. */
async function uploadFolder() {
  const folder = await pickFolder("Compactar e enviar para Syncly/" + path.value);
  if (!folder) return;
  const name = folder.split("/").pop() ?? folder;
  const item = await packed(`Compactando e enviando ${name}…`, () =>
    api.uploadCompressed({
      accountId: accountId.value,
      inputs: [folder],
      remoteDir: path.value,
      format: format.value,
      name: null,
    }),
  );
  if (item) {
    notice.value = { kind: "ok", text: `Compactado, enviado e verificado: Syncly/${item.path}` };
    await load();
  }
}

/** RN-108: descompacta numa pasta nova, nunca por cima. */
async function downloadExtract(item: RemoteItem) {
  const dir = await pickFolder(`Descompactar "${item.name}" em…`);
  if (!dir) return;
  const result = await packed(`Baixando e descompactando ${item.name}…`, () =>
    api.downloadAndExtract(accountId.value, item.path, dir),
  );
  if (!result) return;
  const skipped = result.skippedCount
    ? ` ${result.skippedCount} entrada(s) ignorada(s) por segurança.`
    : "";
  notice.value = {
    kind: result.skippedCount ? "warning" : "ok",
    text: `Descompactado em ${result.dest} — ${result.files} arquivo(s).${skipped}`,
  };
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
        <select v-model="format" aria-label="Formato da pasta compactada" :disabled="!!busy">
          <option v-for="f in archiveFormats" :key="f.value" :value="f.value">{{ f.label }}</option>
        </select>
        <button
          class="btn"
          type="button"
          :disabled="!!busy"
          title="Compacta a pasta e envia um único arquivo (arquivos sensíveis ficam de fora)"
          @click="uploadFolder"
        >
          Enviar pasta compactada…
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
    <section v-if="packing" class="run" aria-live="polite">
      <div class="run-head">
        <strong>{{ busy }}</strong>
        <button class="btn btn-ghost" type="button" @click="api.cancelArchive()">Cancelar</button>
      </div>
      <div class="progress" role="progressbar">
        <div :style="{ width: `${percent ?? 100}%` }" :class="{ indeterminate: percent === null }"></div>
      </div>
      <p v-if="progress" class="muted small">
        {{ formatBytes(progress.bytesDone) }}<template v-if="progress.bytesTotal">
          de {{ formatBytes(progress.bytesTotal) }}</template>
      </p>
      <p v-if="progress?.current" class="muted small run-current">
        <code>{{ progress.current }}</code>
      </p>
    </section>
    <p v-else-if="busy" class="muted">{{ busy }}</p>

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
            <button
              v-if="item.kind === 'file' && item.downloadable && ARCHIVE_NAME.test(item.name)"
              class="btn"
              type="button"
              :disabled="!!busy"
              @click="downloadExtract(item)"
            >
              Baixar e descompactar
            </button>
          </td>
        </tr>
      </tbody>
    </table>
  </section>
</template>
