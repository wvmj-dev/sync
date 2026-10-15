<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, errorMessage } from "../api";
import type { ItemStatus, JobStatus } from "../api/types";
import { formatBytes, formatDate, situationLabel } from "../utils/format";

const props = defineProps<{ jobId: string }>();

/** Pasta mostrada, relativa ao destino da tarefa ("" = raiz). */
const folder = ref("");
const status = ref<JobStatus | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const crumbs = computed(() => {
  const parts = folder.value ? folder.value.split("/") : [];
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
});

const counts = computed(() => {
  const c = status.value?.counts;
  if (!c) return [];
  return [
    { kind: "synced", label: "sincronizados", value: c.synced },
    { kind: "pending", label: "pendentes", value: c.pending },
    { kind: "locally_changed", label: "alterados aqui", value: c.locallyChanged },
    { kind: "remotely_changed", label: "alterados no destino", value: c.remotelyChanged },
    { kind: "conflict", label: "conflitos", value: c.conflict },
    { kind: "ignored", label: "ignorados", value: c.ignored },
  ].filter((c) => c.value > 0);
});

/** RN-103: `remote` consulta o destino agora; sem ele, só computador + último sync. */
async function load(path: string, remote = false) {
  loading.value = true;
  error.value = null;
  try {
    status.value = remote
      ? await api.refreshJobStatus(props.jobId, path)
      : await api.jobStatus(props.jobId, path);
    folder.value = path;
  } catch (e) {
    error.value = errorMessage(e);
  } finally {
    loading.value = false;
  }
}

function open(item: ItemStatus) {
  if (item.kind === "folder") void load(item.path);
}

function size(item: ItemStatus): string {
  return item.kind === "folder"
    ? `${item.files} arquivo(s) · ${formatBytes(item.size)}`
    : formatBytes(item.size);
}

onMounted(() => load(""));
</script>

<template>
  <section class="run status-panel">
    <div class="run-head">
      <h3>Status</h3>
      <button class="btn" type="button" :disabled="loading" @click="load(folder, true)">
        Atualizar do destino
      </button>
    </div>

    <div v-if="error" class="banner banner-danger">{{ error }}</div>

    <template v-if="status">
      <p v-if="!status.perFile" class="muted">
        Tarefas compactadas e que recebem não têm status por arquivo: veja as
        <strong>Versões</strong> e a última execução.
      </p>
      <template v-else>
        <p class="muted small">
          No computador: {{ status.localFiles }} arquivo(s), {{ formatBytes(status.localBytes) }} ·
          no último sync: {{ status.syncedFiles }} arquivo(s), {{ formatBytes(status.syncedBytes) }}
          ·
          <template v-if="status.remoteCheckedAt">
            destino consultado em {{ formatDate(status.remoteCheckedAt) }}
          </template>
          <template v-else>destino: dados do último sync</template>
        </p>
        <div class="row">
          <span v-for="c in counts" :key="c.kind" class="tag" :class="`tag-${c.kind}`">
            {{ c.value }} {{ c.label }}
          </span>
        </div>

        <nav class="crumbs" aria-label="Pasta">
          <button class="btn btn-ghost" type="button" @click="load('')">Tarefa</button>
          <template v-for="c in crumbs" :key="c.path">
            <span class="muted">/</span>
            <button class="btn btn-ghost" type="button" @click="load(c.path)">{{ c.name }}</button>
          </template>
        </nav>

        <p v-if="loading" class="muted">Carregando…</p>
        <p v-else-if="!status.items.length" class="muted">Pasta vazia.</p>
        <table v-else class="table">
          <thead>
            <tr>
              <th>Nome</th>
              <th>Situação</th>
              <th>Modificado aqui</th>
              <th>No destino</th>
              <th>Último sync</th>
              <th>Tamanho</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in status.items" :key="item.path">
              <td>
                <button
                  v-if="item.kind === 'folder'"
                  class="btn btn-ghost folder-link"
                  type="button"
                  @click="open(item)"
                >
                  📁 {{ item.name }}
                </button>
                <span v-else>{{ item.name }}</span>
              </td>
              <td>
                <span class="tag" :class="`tag-${item.situation}`">
                  {{ situationLabel[item.situation] }}
                </span>
                <span v-if="item.detail" class="muted small"> {{ item.detail }}</span>
              </td>
              <td>{{ formatDate(item.localModified) }}</td>
              <td>
                {{ formatDate(item.remoteModified) }}
                <span v-if="item.remoteModifiedBy" class="muted small">
                  por {{ item.remoteModifiedBy }}</span>
              </td>
              <td>{{ formatDate(item.syncedAt) }}</td>
              <td class="muted">{{ size(item) }}</td>
            </tr>
          </tbody>
        </table>
        <p v-if="status.totalItems > status.items.length" class="muted small">
          Mostrando {{ status.items.length }} de {{ status.totalItems }} itens.
        </p>
      </template>
    </template>
    <p v-else-if="loading" class="muted">Carregando…</p>
  </section>
</template>
