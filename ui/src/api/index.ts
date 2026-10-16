// ÚNICO lugar do frontend que fala com o Rust. Componentes usam `api.*`, nunca `invoke`.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, open } from "@tauri-apps/plugin-dialog";
import type {
  AccountSummary,
  Appearance,
  ArchiveProgress,
  CompressRequest,
  CompressResult,
  ExtractResult,
  ApiErrorCode,
  AppInfo,
  JobDraft,
  JobStatus,
  JobSummary,
  PlanPreview,
  Preferences,
  RemoteItem,
  RunProgress,
  RunReport,
  UploadCompressedRequest,
  VersionItem,
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

/** Fora da janela do app (ex.: aba do navegador em localhost:1420) não há ponte com o Rust. */
const OUTSIDE_APP =
  "Esta tela só funciona dentro da janela do Syncly. Feche esta aba e rode `pnpm tauri dev`.";

/** Só em desenvolvimento (`pnpm dev`, endereço com `?mock`): dados de exemplo para revisar o
 * visual no navegador. Fora do build de produção. */
function mockEnabled(): boolean {
  return import.meta.env.DEV && new URLSearchParams(location.search).has("mock");
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    if (import.meta.env.DEV && mockEnabled()) {
      const { mockCall } = await import("./mock");
      return mockCall<T>(command, args);
    }
    throw new ApiError("unavailable", OUTSIDE_APP);
  }
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    if (isApiErrorPayload(e)) throw new ApiError(e.code, e.message);
    throw new ApiError("internal", String(e));
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  /** Tema do sistema (claro/escuro, cor de destaque). */
  systemAppearance: () => call<Appearance>("system_appearance"),
  getPreferences: () => call<Preferences>("get_preferences"),
  /** Valores fora dos limites voltam com o código `invalid`. */
  savePreferences: (preferences: Preferences) =>
    call<Preferences>("save_preferences", { preferences }),
  listAccounts: () => call<AccountSummary[]>("list_accounts"),
  addLocalAccount: (label: string, root: string) =>
    call<AccountSummary>("add_local_account", { label, root }),
  removeAccount: (id: string) => call<void>("remove_account", { id }),
  listJobs: () => call<JobSummary[]>("list_jobs"),
  saveJob: (draft: JobDraft) => call<JobSummary>("save_job", { draft }),
  deleteJob: (id: string) => call<void>("delete_job", { id }),
  previewJob: (id: string) => call<PlanPreview>("preview_job", { id }),
  /** Sem `confirm`, pode falhar com o código `needs_confirmation` (RN-36, RN-42). */
  runJob: (id: string, confirm: boolean) => call<RunReport>("run_job", { id, confirm }),
  cancelRun: (id: string) => call<boolean>("cancel_run", { id }),
  /** RN-100 a RN-103: sem rede (computador + último sync). */
  jobStatus: (id: string, folder: string) => call<JobStatus>("job_status", { id, folder }),
  /** RN-103: "Atualizar do destino" — consulta o destino agora. */
  refreshJobStatus: (id: string, folder: string) =>
    call<JobStatus>("refresh_job_status", { id, folder }),
  listVersions: (jobId: string) => call<VersionItem[]>("list_versions", { jobId }),
  /** Baixa, confere e descompacta numa pasta nova dentro de `destDir` (RN-57, RN-58).
   * Progresso pelo evento `run-progress`; cancela com `cancelRun(jobId)`. */
  restoreVersion: (jobId: string, versionId: string, destDir: string) =>
    call<ExtractResult>("restore_version", { jobId, versionId, destDir }),
  compress: (request: CompressRequest) => call<CompressResult>("compress_paths", { request }),
  extract: (archive: string, destDir: string) =>
    call<ExtractResult>("extract_archive", { archive, destDir }),
  cancelArchive: () => call<boolean>("cancel_archive"),
  /** Abre o navegador e espera a autorização (até 5 minutos). */
  connectGoogle: () => call<AccountSummary>("connect_google"),
  listRemote: (accountId: string, path: string) =>
    call<RemoteItem[]>("list_remote", { accountId, path }),
  uploadFile: (accountId: string, localPath: string, remoteDir: string) =>
    call<RemoteItem>("upload_file", { accountId, localPath, remoteDir }),
  /** Retorna o caminho onde o arquivo foi salvo. */
  downloadFile: (accountId: string, remotePath: string, localDir: string) =>
    call<string>("download_file", { accountId, remotePath, localDir }),
  /** RN-108. Progresso por `archive-progress`; cancela com `cancelArchive()`. */
  downloadAndExtract: (accountId: string, remotePath: string, localDir: string) =>
    call<ExtractResult>("download_and_extract", { accountId, remotePath, localDir }),
  /** RN-109. Progresso por `archive-progress`; cancela com `cancelArchive()`. */
  uploadCompressed: (request: UploadCompressedRequest) =>
    call<RemoteItem>("upload_compressed", { request }),
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

/** Progresso de compactação/descompactação. */
export function onArchiveProgress(handler: (p: ArchiveProgress) => void): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => {});
  return listen<ArchiveProgress>("archive-progress", (event) => handler(event.payload));
}

export async function pickArchive(title: string): Promise<string | null> {
  const picked = await open({
    directory: false,
    multiple: false,
    title,
    filters: [
      { name: "Arquivos compactados", extensions: ["zip", "tar", "gz", "tgz", "zst", "tzst"] },
    ],
  });
  return typeof picked === "string" ? picked : null;
}

/** Mudanças no tema do sistema. */
export function onAppearanceChanged(handler: (a: Appearance) => void): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => {});
  return listen<Appearance>("appearance-changed", (event) => handler(event.payload));
}

/** Progresso das execuções (um evento por item). Retorna a função para parar de ouvir. */
export function onRunProgress(handler: (p: RunProgress) => void): Promise<UnlistenFn> {
  if (!isTauri()) return Promise.resolve(() => {});
  return listen<RunProgress>("run-progress", (event) => handler(event.payload));
}

/** Confirmação nativa (o `confirm()` do navegador não é garantido em todas as webviews). */
export function confirmDanger(message: string, okLabel: string): Promise<boolean> {
  return ask(message, { title: "Syncly", kind: "warning", okLabel, cancelLabel: "Cancelar" });
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
