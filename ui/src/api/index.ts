// ÚNICO lugar do frontend que fala com o Rust. Componentes usam `api.*`, nunca `invoke`.
import { invoke } from "@tauri-apps/api/core";
import { ask, open } from "@tauri-apps/plugin-dialog";
import type {
  AccountSummary,
  ApiErrorCode,
  AppInfo,
  JobDraft,
  JobSummary,
  PlanPreview,
} from "./types";

export class ApiError extends Error {
  constructor(
    readonly code: ApiErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

function isApiErrorPayload(e: unknown): e is { code: ApiErrorCode; message: string } {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    if (isApiErrorPayload(e)) throw new ApiError(e.code, e.message);
    throw new ApiError("internal", String(e));
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  listAccounts: () => call<AccountSummary[]>("list_accounts"),
  addLocalAccount: (label: string, root: string) =>
    call<AccountSummary>("add_local_account", { label, root }),
  removeAccount: (id: string) => call<void>("remove_account", { id }),
  listJobs: () => call<JobSummary[]>("list_jobs"),
  saveJob: (draft: JobDraft) => call<JobSummary>("save_job", { draft }),
  deleteJob: (id: string) => call<void>("delete_job", { id }),
  previewJob: (id: string) => call<PlanPreview>("preview_job", { id }),
};

export async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

export async function pickFiles(title: string): Promise<string[]> {
  const picked = await open({ directory: false, multiple: true, title });
  if (Array.isArray(picked)) return picked;
  return picked ? [picked] : [];
}

/** Confirmação nativa (o `confirm()` do navegador não é garantido em todas as webviews). */
export function confirmDanger(message: string, okLabel: string): Promise<boolean> {
  return ask(message, { title: "Syncly", kind: "warning", okLabel, cancelLabel: "Cancelar" });
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
