<script setup lang="ts">
import { onMounted } from "vue";
import { RouterLink, RouterView } from "vue-router";
import Icon from "./components/Icon.vue";
import type { IconName } from "./components/icons";
import Notice from "./components/Notice.vue";
import { useAppStore } from "./stores/app";

const store = useAppStore();
onMounted(store.refresh);

const links: { to: string; label: string; icon: IconName }[] = [
  { to: "/", label: "Visão geral", icon: "overview" },
  { to: "/tarefas", label: "Tarefas", icon: "tasks" },
  { to: "/contas", label: "Contas", icon: "accounts" },
  { to: "/arquivos", label: "Arquivos", icon: "files" },
  { to: "/compactar", label: "Compactar", icon: "archive" },
];
</script>

<template>
  <div class="shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true"><Icon name="sync" :size="18" /></span>
        <span>Syncly</span>
      </div>
      <nav class="nav" aria-label="Seções">
        <RouterLink v-for="link in links" :key="link.to" :to="link.to" class="nav-row">
          <Icon :name="link.icon" :size="20" />
          <span>{{ link.label }}</span>
        </RouterLink>
      </nav>
      <p class="sidebar-foot">Versão {{ store.info?.version ?? "…" }}</p>
    </aside>

    <main class="main">
      <Notice v-if="store.error" kind="danger" dismissible class="app-error" @close="store.error = null">
        {{ store.error }}
      </Notice>
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.app-error {
  margin-bottom: 20px;
}
</style>
