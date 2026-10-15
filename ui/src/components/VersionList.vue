<script setup lang="ts">
import type { VersionItem } from "../api/types";
import { formatBytes, formatDate } from "../utils/format";

defineProps<{ versions: VersionItem[]; busy: boolean }>();
const emit = defineEmits<{ restore: [version: VersionItem] }>();
</script>

<template>
  <div class="versions">
    <table v-if="versions.length" class="table">
      <thead>
        <tr>
          <th>Versão</th>
          <th>Enviada em</th>
          <th>Tamanho</th>
          <th>Destino</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="v in versions" :key="v.id">
          <td><code>{{ v.name }}</code></td>
          <td>{{ formatDate(v.createdAt) }}</td>
          <td>{{ formatBytes(v.size) }}</td>
          <td class="muted">{{ v.accountLabel ?? "conta removida" }}: {{ v.path }}</td>
          <td>
            <button
              class="btn"
              type="button"
              :disabled="busy || !v.accountLabel"
              @click="emit('restore', v)"
            >
              Restaurar…
            </button>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-else class="muted">Nenhuma versão enviada ainda — execute a tarefa.</p>
    <p class="muted small">
      Restaurar baixa a versão, confere se é a mesma que foi enviada e descompacta numa
      <strong>pasta nova</strong> — nada existente é substituído.
    </p>
  </div>
</template>
