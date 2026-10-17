<script setup lang="ts">
import { onMounted, ref } from "vue";
import { RouterLink, RouterView } from "vue-router";
import logo from "./assets/syncly.svg";
import Icon from "./components/Icon.vue";
import type { IconName } from "./components/icons";
import Motion from "./components/Motion.vue";
import Notice from "./components/Notice.vue";
import { useAppStore } from "./stores/app";

const store = useAppStore();
onMounted(store.refresh);

/** Menu recolhido (só ícones). Preferência de exibição deste computador — se o armazenamento
 * do navegador não estiver disponível, o menu só volta aberto na próxima vez. */
const COLLAPSED_KEY = "syncly.sidebar.collapsed";
function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}
const collapsed = ref(readCollapsed());
/** Por um instante depois de expandir: os nomes entram deslizando (ver `.expanding`). Os
 * 400 ms passam do movimento e acabam antes da animação do CSS: o fim é marcado aqui. */
const expanding = ref(false);
let expandTimer: ReturnType<typeof setTimeout> | undefined;
function toggleSidebar() {
  collapsed.value = !collapsed.value;
  clearTimeout(expandTimer);
  expanding.value = !collapsed.value;
  if (expanding.value) expandTimer = setTimeout(() => (expanding.value = false), 400);
  try {
    localStorage.setItem(COLLAPSED_KEY, collapsed.value ? "1" : "0");
  } catch {
    // Sem armazenamento: vale só nesta sessão.
  }
}

const links: { to: string; label: string; icon: IconName }[] = [
  { to: "/", label: "Visão geral", icon: "overview" },
  { to: "/tarefas", label: "Tarefas", icon: "tasks" },
  { to: "/contas", label: "Contas", icon: "accounts" },
  { to: "/arquivos", label: "Arquivos", icon: "files" },
  { to: "/compactar", label: "Compactar", icon: "archive" },
  { to: "/preferencias", label: "Preferências", icon: "preferences" },
];
</script>

<template>
  <div class="shell" :class="{ collapsed, expanding }">
    <aside class="sidebar">
      <div class="brand">
        <img class="brand-logo" :src="logo" alt="" width="30" height="30" />
        <span class="brand-name">Syncly</span>
      </div>
      <nav class="nav" aria-label="Seções">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          :to="link.to"
          class="nav-row"
          :title="collapsed ? link.label : undefined"
        >
          <Icon :name="link.icon" :size="20" />
          <span class="nav-label">{{ link.label }}</span>
        </RouterLink>
      </nav>
      <div class="sidebar-foot">
        <button
          class="nav-row collapse-toggle"
          type="button"
          :aria-expanded="!collapsed"
          :title="collapsed ? 'Expandir o menu' : undefined"
          @click="toggleSidebar"
        >
          <Icon :name="collapsed ? 'sidebarExpand' : 'sidebarCollapse'" :size="20" />
          <span class="nav-label">{{ collapsed ? "Expandir menu" : "Recolher menu" }}</span>
        </button>
        <p class="version">Versão {{ store.info?.version ?? "…" }}</p>
      </div>
    </aside>

    <main class="main">
      <Motion name="slide">
        <Notice v-if="store.error" kind="danger" dismissible class="app-error" @close="store.error = null">
          {{ store.error }}
        </Notice>
      </Motion>
      <!-- Troca de tela: a anterior esmaece e a nova sobe de leve. -->
      <RouterView v-slot="{ Component, route }">
        <Motion name="swap" mode="out-in">
          <div :key="route.path">
            <component :is="Component" />
          </div>
        </Motion>
      </RouterView>
    </main>
  </div>
</template>

<style scoped>
.app-error {
  margin-bottom: 20px;
}
</style>
