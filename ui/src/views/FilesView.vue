<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { RouterLink } from "vue-router";
import { api, errorMessage, onArchiveProgress, pickFiles, pickFolder } from "../api";
import type { ArchiveFormat, ArchiveProgress, RemoteItem } from "../api/types";
import { useAppStore } from "../stores/app";
import Icon from "../components/Icon.vue";
import Notice from "../components/Notice.vue";
import { archiveFormats, formatBytes, formatDate, formatRelative } from "../utils/format";

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
/** RN-111: linha "Nova pasta" aberta. */
const creating = ref(false);
const newName = ref("");
const nameInput = ref<HTMLInputElement | null>(null);

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
  creating.value = false;
  path.value = folder;
}

function changeAccount() {
  notice.value = null;
  creating.value = false;
  path.value = "";
}

async function startCreate() {
  newName.value = "";
  creating.value = true;
  await nextTick();
  nameInput.value?.focus();
}

/** RN-111: cria na pasta atual; se o nome já existir, nada muda. */
async function createFolder() {
  const name = newName.value.trim();
  if (!name) return;
  busy.value = `Criando ${name}…`;
  try {
    const item = await api.createRemoteFolder(accountId.value, path.value, name);
    creating.value = false;
    notice.value = { kind: "ok", text: `Pasta criada: Syncly/${item.path}` };
    await load();
  } catch (e) {
    notice.value = { kind: "danger", text: errorMessage(e) };
  } finally {
    busy.value = null;
  }
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
    <div>
      <h1>Arquivos</h1>
      <p class="page-desc">O que está na pasta <code>Syncly/</code> de cada conta. Envie e baixe arquivos avulsos.</p>
    </div>
  </header>

  <div v-if="!store.accounts.length" class="boxed empty">
    <Icon name="accounts" :size="28" />
    <strong>Nenhuma conta ainda</strong>
    <p>Conecte o Google Drive ou adicione uma pasta local para ver os arquivos.</p>
    <RouterLink class="btn btn-primary" to="/contas">Abrir Contas</RouterLink>
  </div>

  <div v-else class="stack">
    <div class="toolbar">
      <span class="select account">
        <select v-model="accountId" aria-label="Conta" @change="changeAccount">
          <option v-for="a in store.accounts" :key="a.id" :value="a.id">
            {{ a.label }} ({{ a.provider === "google_drive" ? "Google Drive" : "pasta local" }})
          </option>
        </select>
      </span>
      <span class="spacer"></span>
      <button
        class="btn btn-flat btn-icon"
        type="button"
        title="Atualizar"
        aria-label="Atualizar a lista"
        :disabled="loading || !!busy"
        @click="load"
      >
        <Icon name="refresh" />
      </button>
      <button class="btn" type="button" :disabled="!!busy" @click="upload">
        <Icon name="upload" :size="16" />
        Enviar arquivo…
      </button>
      <span class="compressed">
        <button
          class="btn"
          type="button"
          :disabled="!!busy"
          title="Compacta a pasta e envia um único arquivo (arquivos sensíveis ficam de fora)"
          @click="uploadFolder"
        >
          <Icon name="archive" :size="16" />
          Enviar pasta compactada…
        </button>
        <span class="select format">
          <select v-model="format" aria-label="Formato da pasta compactada" :disabled="!!busy">
            <option v-for="f in archiveFormats" :key="f.value" :value="f.value">{{ f.label }}</option>
          </select>
        </span>
      </span>
    </div>

    <Notice v-if="notice" :kind="notice.kind === 'warning' ? 'warn' : notice.kind" dismissible @close="notice = null">
      {{ notice.text }}
    </Notice>

    <section v-if="packing" class="boxed pad run" aria-live="polite">
      <div class="run-head">
        <strong>{{ busy }}</strong>
        <button class="btn btn-flat btn-small" type="button" @click="api.cancelArchive()">
          <Icon name="stop" :size="14" />
          Cancelar
        </button>
      </div>
      <div class="progress" role="progressbar">
        <div :style="{ width: `${percent ?? 100}%` }" :class="{ indeterminate: percent === null }"></div>
      </div>
      <p v-if="progress" class="muted small num">
        {{ formatBytes(progress.bytesDone) }}<template v-if="progress.bytesTotal">
          de {{ formatBytes(progress.bytesTotal) }}</template>
      </p>
      <p v-if="progress?.current" class="path ellipsis">{{ progress.current }}</p>
    </section>
    <p v-else-if="busy" class="muted">{{ busy }}</p>

    <div class="boxed">
      <div class="files-head">
        <nav class="crumbs" aria-label="Pasta atual">
          <button type="button" @click="open('')">Syncly</button>
          <template v-for="c in crumbs" :key="c.path">
            <span class="sep" aria-hidden="true">/</span>
            <button type="button" @click="open(c.path)">{{ c.name }}</button>
          </template>
        </nav>
        <button class="btn btn-flat btn-small" type="button" :disabled="!!busy || creating" @click="startCreate">
          <Icon name="newFolder" :size="16" />
          Nova pasta
        </button>
      </div>

      <form v-if="creating" class="new-folder" @submit.prevent="createFolder" @keydown.esc="creating = false">
        <Icon name="folder" class="muted" />
        <input
          ref="nameInput"
          v-model="newName"
          aria-label="Nome da nova pasta"
          placeholder="Nome da pasta"
          maxlength="255"
          autocomplete="off"
        />
        <button class="btn btn-primary btn-small" type="submit" :disabled="!newName.trim() || !!busy">Criar</button>
        <button class="btn btn-flat btn-small" type="button" @click="creating = false">Cancelar</button>
      </form>

      <p v-if="loading" class="empty">Carregando…</p>
      <div v-else-if="!items.length && !creating" class="empty">
        <strong>Pasta vazia</strong>
        <p>Envie um arquivo ou uma pasta compactada, ou crie uma pasta.</p>
      </div>
      <div v-else class="table-wrap">
        <table class="table">
          <thead>
            <tr>
              <th>Nome</th>
              <th>Tamanho</th>
              <th>Modificado</th>
              <th><span class="visually-hidden">Ações</span></th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in items" :key="item.path">
              <td>
                <button
                  v-if="item.kind === 'folder'"
                  class="link-button"
                  type="button"
                  @click="open(item.path)"
                >
                  <Icon name="folder" class="muted" />
                  {{ item.name }}
                </button>
                <span v-else class="name"><Icon name="file" class="muted" />{{ item.name }}</span>
              </td>
              <td class="muted">{{ item.size != null ? formatBytes(item.size) : "" }}</td>
              <td :title="formatDate(item.modifiedAt)">
                {{ formatRelative(item.modifiedAt) }}
                <div v-if="item.modifiedBy" class="muted small">por {{ item.modifiedBy }}</div>
              </td>
              <td class="actions">
                <template v-if="item.kind === 'file'">
                  <button
                    v-if="item.downloadable && ARCHIVE_NAME.test(item.name)"
                    class="btn btn-flat btn-small"
                    type="button"
                    :disabled="!!busy"
                    @click="downloadExtract(item)"
                  >
                    Baixar e descompactar
                  </button>
                  <button
                    class="btn btn-small"
                    type="button"
                    :disabled="!item.downloadable || !!busy"
                    :title="item.downloadable ? 'Baixar' : 'Documentos nativos do Google não podem ser baixados'"
                    @click="download(item)"
                  >
                    <Icon name="download" :size="16" />
                    Baixar
                  </button>
                </template>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.account {
  width: min(100%, 340px);
}

.compressed {
  display: inline-flex;
  gap: 4px;
}

.format {
  width: 110px;
}

.files-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
}

.new-folder {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px 12px 20px;
}

.new-folder input {
  flex: 1;
  min-width: 0;
}

.name {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.run {
  display: grid;
  gap: 8px;
}

.run-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
</style>
