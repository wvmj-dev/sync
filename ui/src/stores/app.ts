import { defineStore } from "pinia";
import { ref } from "vue";
import { api, errorMessage } from "../api";
import type { AccountSummary, AppInfo, JobDraft, JobSummary } from "../api/types";

export const useAppStore = defineStore("app", () => {
  const info = ref<AppInfo | null>(null);
  const accounts = ref<AccountSummary[]>([]);
  const jobs = ref<JobSummary[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function refresh() {
    loading.value = true;
    try {
      const [i, a, j] = await Promise.all([api.appInfo(), api.listAccounts(), api.listJobs()]);
      info.value = i;
      accounts.value = a;
      jobs.value = j;
    } catch (e) {
      error.value = errorMessage(e);
    } finally {
      loading.value = false;
    }
  }

  /** Executa uma alteração e recarrega tudo; erros vão para o banner global. */
  async function mutate(action: () => Promise<unknown>): Promise<boolean> {
    error.value = null;
    try {
      await action();
      await refresh();
      return true;
    } catch (e) {
      error.value = errorMessage(e);
      return false;
    }
  }

  return {
    info,
    accounts,
    jobs,
    loading,
    error,
    refresh,
    saveJob: (draft: JobDraft) => mutate(() => api.saveJob(draft)),
    deleteJob: (id: string) => mutate(() => api.deleteJob(id)),
    addLocalAccount: (label: string, root: string) => mutate(() => api.addLocalAccount(label, root)),
    removeAccount: (id: string) => mutate(() => api.removeAccount(id)),
    connectGoogle: () => mutate(() => api.connectGoogle()),
  };
});
