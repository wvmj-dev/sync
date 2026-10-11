import { createRouter, createWebHashHistory } from "vue-router";
import DashboardView from "./views/DashboardView.vue";
import JobsView from "./views/JobsView.vue";
import AccountsView from "./views/AccountsView.vue";
import ArchiveView from "./views/ArchiveView.vue";
import FilesView from "./views/FilesView.vue";

export const router = createRouter({
  // Hash history: funciona com o protocolo customizado do Tauri sem servidor.
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "dashboard", component: DashboardView },
    { path: "/tarefas", name: "jobs", component: JobsView },
    { path: "/contas", name: "accounts", component: AccountsView },
    { path: "/arquivos", name: "files", component: FilesView },
    { path: "/compactar", name: "archive", component: ArchiveView },
  ],
});
