<script setup lang="ts">
import { onMounted } from "vue";
import { RouterLink, RouterView } from "vue-router";
import { useAppStore } from "./stores/app";

const store = useAppStore();
onMounted(store.refresh);

const links = [
  { to: "/", label: "Visão geral" },
  { to: "/tarefas", label: "Tarefas" },
  { to: "/contas", label: "Contas" },
];
</script>

<template>
  <div class="shell">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">⟳</span>
        <span>Syncly</span>
      </div>
      <nav>
        <RouterLink v-for="link in links" :key="link.to" :to="link.to" class="nav-link">
          {{ link.label }}
        </RouterLink>
      </nav>
      <p class="sidebar-foot">v{{ store.info?.version ?? "…" }}</p>
    </aside>

    <main class="content">
      <div v-if="store.error" class="banner banner-danger" role="alert">
        <span>{{ store.error }}</span>
        <button class="btn btn-ghost" type="button" @click="store.error = null">Fechar</button>
      </div>
      <RouterView />
    </main>
  </div>
</template>
