<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, errorMessage } from "../api";
import type { ItemStatus, JobStatus } from "../api/types";
import { formatBytes, formatDate, formatRelative, situationChip, situationLabel } from "../utils/format";
import Icon from "./Icon.vue";
import Notice from "./Notice.vue";

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
    { key: "synced", label: "sincronizados", value: c.synced },
    { key: "pending", label: "pendentes", value: c.pending },
    { key: "locally_changed", label: "alterados aqui", value: c.locallyChanged },
    { key: "remotely_changed", label: "alterados no destino", value: c.remotelyChanged },
    { key: "conflict", label: "em conflito", value: c.conflict },
    { key: "ignored", label: "ignorados", value: c.ignored },
  ].filter((x) => x.value > 0) as { key: keyof typeof situationChip; label: string; value: number }[];
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

/** O detalhe só aparece quando diz mais que o rótulo ("apagado no destino", "novo"…). */
function extraDetail(item: ItemStatus): boolean {
  return !!item.detail && !["alterado no computador", "alterado no destino"].includes(item.detail);
}

function size(item: ItemStatus): string {
  return item.kind === "folder"
    ? `${item.files} arquivo(s), ${formatBytes(item.size)}`
    : formatBytes(item.size);
}

onMounted(() => load(""));
</script>

<template>
  <div class="status">
    <Notice v-if="error" kind="danger">{{ error }}</Notice>

    <template v-if="status">
      <p v-if="!status.perFile" class="muted">
        Tarefas compactadas e as que recebem não têm status por arquivo: veja as
        <strong>Versões</strong> e a última execução.
      </p>
      <template v-else>
        <div class="status-head">
          <dl class="facts">
            <div>
              <dt>No computador</dt>
              <dd class="num">{{ status.localFiles }} arquivo(s), {{ formatBytes(status.localBytes) }}</dd>
            </div>
            <div>
              <dt>No último sync</dt>
              <dd class="num">{{ status.syncedFiles }} arquivo(s), {{ formatBytes(status.syncedBytes) }}</dd>
            </div>
            <div>
              <dt>Destino</dt>
              <dd v-if="status.remoteCheckedAt" :title="formatDate(status.remoteCheckedAt)">
                Consultado {{ formatRelative(status.remoteCheckedAt) }}
              </dd>
              <dd v-else class="muted">Dados do último sync</dd>
            </div>
          </dl>
          <button class="btn btn-small" type="button" :disabled="loading" @click="load(folder, true)">
            <Icon name="refresh" :size="16" />
            Atualizar do destino
          </button>
        </div>

        <div v-if="counts.length" class="row">
          <span v-for="c in counts" :key="c.key" class="chip num" :class="situationChip[c.key]">
            {{ c.value }} {{ c.label }}
          </span>
        </div>

        <nav class="crumbs" aria-label="Pasta">
          <button type="button" @click="load('')">Tarefa</button>
          <template v-for="c in crumbs" :key="c.path">
            <span class="sep" aria-hidden="true">/</span>
            <button type="button" @click="load(c.path)">{{ c.name }}</button>
          </template>
        </nav>

        <p v-if="loading" class="muted">Carregando…</p>
        <p v-else-if="!status.items.length" class="muted">Pasta vazia.</p>
        <div v-else class="table-wrap">
          <table class="table">
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
                    class="link-button"
                    type="button"
                    @click="load(item.path)"
                  >
                    <Icon name="folder" class="muted" />
                    {{ item.name }}
                  </button>
                  <span v-else class="name"><Icon name="file" class="muted" />{{ item.name }}</span>
                </td>
                <td>
                  <span class="chip" :class="situationChip[item.situation]">
                    {{ situationLabel[item.situation] }}
                  </span>
                  <div v-if="extraDetail(item)" class="muted small">{{ item.detail }}</div>
                </td>
                <td :title="formatDate(item.localModified)">{{ formatRelative(item.localModified) }}</td>
                <td :title="formatDate(item.remoteModified)">
                  {{ formatRelative(item.remoteModified) }}
                  <div v-if="item.remoteModifiedBy" class="muted small">
                    por {{ item.remoteModifiedBy }}
                  </div>
                </td>
                <td :title="formatDate(item.syncedAt)">{{ formatRelative(item.syncedAt) }}</td>
                <td class="muted">{{ size(item) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-if="status.totalItems > status.items.length" class="muted small">
          Mostrando {{ status.items.length }} de {{ status.totalItems }} itens.
        </p>
      </template>
    </template>
    <p v-else-if="loading" class="muted">Carregando…</p>
  </div>
</template>

<style scoped>
.status {
  display: grid;
  gap: 14px;
}

.status-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.status-head .facts {
  flex: 1;
}

.name {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

@media (max-width: 760px) {
  .status-head {
    flex-direction: column;
  }
}
</style>
