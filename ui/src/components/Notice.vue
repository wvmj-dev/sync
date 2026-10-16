<script setup lang="ts">
import { computed } from "vue";
import Icon from "./Icon.vue";
import type { IconName } from "./icons";

const props = withDefaults(
  defineProps<{ kind?: "ok" | "warn" | "danger" | "info"; dismissible?: boolean }>(),
  { kind: "info", dismissible: false },
);
const emit = defineEmits<{ close: [] }>();

const icon = computed<IconName>(
  () => ({ ok: "check", warn: "alert", danger: "error", info: "info" })[props.kind] as IconName,
);
</script>

<template>
  <div class="notice" :class="`notice-${kind}`" :role="kind === 'danger' ? 'alert' : 'status'">
    <Icon class="notice-icon" :name="icon" :size="20" />
    <div class="notice-body"><slot /></div>
    <button
      v-if="dismissible"
      class="btn btn-flat btn-small btn-icon"
      type="button"
      aria-label="Fechar aviso"
      @click="emit('close')"
    >
      <Icon name="close" :size="16" />
    </button>
  </div>
</template>
