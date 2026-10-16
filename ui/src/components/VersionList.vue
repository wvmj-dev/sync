<script setup lang="ts">
import type { VersionItem } from "../api/types";
import { formatBytes, formatDate, formatRelative } from "../utils/format";
import Icon from "./Icon.vue";

defineProps<{ versions: VersionItem[]; busy: boolean }>();
const emit = defineEmits<{ restore: [version: VersionItem] }>();
</script>

<template>
  <div class="versions">
    <div v-if="versions.length" class="boxed">
      <div v-for="v in versions" :key="v.id" class="item">
        <Icon class="item-icon" name="archive" :size="20" />
        <div class="item-main">
          <div class="item-title path ellipsis">{{ v.name }}</div>
          <div class="item-sub" :title="formatDate(v.createdAt)">
            Enviada {{ formatRelative(v.createdAt) }}, {{ formatBytes(v.size) }}, em
            {{ v.accountLabel ?? "conta removida" }}
          </div>
        </div>
        <div class="item-end">
          <button
            class="btn btn-small"
            type="button"
            :disabled="busy || !v.accountLabel"
            @click="emit('restore', v)"
          >
            <Icon name="download" :size="16" />
            Restaurar…
          </button>
        </div>
      </div>
    </div>
    <p v-else class="muted">Nenhuma versão enviada ainda. Execute a tarefa para criar a primeira.</p>
    <p class="muted small">
      Restaurar baixa a versão, confere se é a mesma que foi enviada e a descompacta numa
      <strong>pasta nova</strong>; nada existente é substituído.
    </p>
  </div>
</template>

<style scoped>
.versions {
  display: grid;
  gap: 12px;
}

.versions .item-title.path {
  color: var(--fg);
  font-weight: 500;
}
</style>
