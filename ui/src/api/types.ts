// Espelho de `crates/service/src/dto.rs`. Ao mudar um lado, mude o outro no mesmo commit.

export type SyncMode = "upload" | "mirror";

/** RN-54. Vem do core com os mesmos nomes do config.toml (snake_case). */
export interface Retention {
  keep_last: number;
  daily: number;
  weekly: number;
  monthly: number;
}

/** RN-50: arquivo por arquivo, ou tudo num arquivo compactado com versões. */
export type Delivery =
  | { kind: "files" }
  | { kind: "archive"; format: ArchiveFormat; retention: Retention };

export interface AppInfo {
  version: string;
  configFile: string;
  dataDir: string;
  logDir: string;
  googleClientConfigured: boolean;
  googleClientProblem: string | null;
  configProblem: string | null;
}

export interface AccountSummary {
  id: string;
  label: string;
  provider: "google_drive" | "local";
  detail: string;
  jobCount: number;
}

export type RunStatus =
  | "running"
  | "success"
  | "partial_success"
  | "failed"
  | "aborted"
  | "awaiting_confirmation"
  | "cancelled"
  | "postponed";

export interface RunSummary {
  kind: "sync" | "backup" | "restore" | "compress";
  trigger: "manual" | "scheduled";
  status: RunStatus;
  startedAt: string;
  finishedAt: string | null;
  error: string | null;
}

export interface SyncSummary {
  accountId: string;
  accountLabel: string | null;
  path: string;
  delivery: Delivery;
  mode: SyncMode;
  schedule: string;
}

export interface JobSummary {
  id: string;
  name: string;
  enabled: boolean;
  sources: string[];
  exclude: string[];
  includeSensitive: boolean;
  sync: SyncSummary | null;
  lastRun: RunSummary | null;
}

/** Versão enviada por uma tarefa compactada (RN-50). */
export interface VersionItem {
  id: string;
  name: string;
  size: number;
  createdAt: string;
  accountId: string;
  /** `null` se a conta foi removida. */
  accountLabel: string | null;
  /** Pasta no destino, relativa à raiz do app. */
  path: string;
}

export interface RemoteItem {
  name: string;
  /** Relativo à pasta raiz do app (Syncly/). */
  path: string;
  kind: "file" | "folder";
  size: number | null;
  modifiedAt: string | null;
  modifiedBy: string | null;
  /** Google Docs nativos não têm conteúdo para baixar. */
  downloadable: boolean;
}

export interface SyncDraft {
  accountId: string;
  path: string;
  delivery: Delivery;
  /** Só vale para a entrega arquivo por arquivo. */
  mode: SyncMode;
}

export interface JobDraft {
  id: string | null;
  name: string;
  enabled: boolean;
  sources: string[];
  exclude: string[];
  includeSensitive: boolean;
  sync: SyncDraft | null;
}

export interface PlanStats {
  foldersToCreate: number;
  uploads: number;
  updates: number;
  conflicts: number;
  adopted: number;
  trashFiles: number;
  trashFolders: number;
  forgotten: number;
  skipped: number;
  bytesToSend: number;
}

export type GuardVerdict =
  | { verdict: "proceed" }
  | { verdict: "needs_confirmation"; reasons: string[] }
  | { verdict: "abort"; reasons: string[] };

export type ActionKind =
  | "create_folder"
  | "upload"
  | "update"
  | "conflict"
  | "adopt"
  | "trash"
  | "forget"
  | "skip";

export interface ActionView {
  kind: ActionKind;
  path: string;
  detail: string | null;
}

export interface PlanPreview {
  jobId: string;
  mode: SyncMode;
  stats: PlanStats;
  verdict: GuardVerdict;
  actions: ActionView[];
  totalActions: number;
  warnings: string[];
  hashedFiles: number;
  durationMs: number;
  firstRun: boolean;
  /** Presente quando a tarefa envia compactado; aí `stats` e `actions` ficam vazios. */
  archive: ArchivePreview | null;
}

export interface ArchivePreview {
  /** Nome que a versão teria se fosse criada agora. */
  name: string;
  format: ArchiveFormat;
  files: number;
  folders: number;
  /** Tamanho antes de compactar. */
  bytes: number;
  /** RN-59: nada mudou desde a última versão. */
  unchanged: boolean;
  versions: number;
  /** RN-54: versões que iriam para a lixeira depois do envio. */
  toTrash: string[];
}

export interface RunProgress {
  jobId: string;
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  current: string | null;
}

export interface RunFailure {
  path: string;
  error: string;
}

export interface RunReport {
  status: RunStatus;
  foldersCreated: number;
  uploaded: number;
  updated: number;
  conflicts: number;
  adopted: number;
  trashed: number;
  forgotten: number;
  bytesSent: number;
  /** RN-25: mudaram durante o envio; vão de novo na próxima execução. */
  unstable: number;
  /** Até 50; o total está em `failureCount`. */
  failures: RunFailure[];
  failureCount: number;
  cancelled: boolean;
  aborted: string | null;
  durationMs: number;
  /** Presente quando a tarefa envia compactado. */
  archive: ArchiveRun | null;
}

export interface ArchiveRun {
  /** `null` quando nada mudou (RN-59). */
  name: string | null;
  files: number;
  bytesIn: number;
  bytesOut: number;
  unchanged: boolean;
  /** Versões mantidas no destino depois da retenção. */
  versions: number;
}

export type ArchiveFormat = "zip" | "tar" | "tar_gz" | "tar_zst";

export interface ArchiveProgress {
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  /** 0 = desconhecido (descompactar .tar.*). */
  bytesTotal: number;
  current: string | null;
}

export interface CompressRequest {
  inputs: string[];
  outputDir: string;
  /** Sem extensão; vazio = nome da entrada. */
  name: string | null;
  format: ArchiveFormat;
  includeSensitive: boolean;
}

export interface CompressResult {
  path: string;
  format: ArchiveFormat;
  files: number;
  folders: number;
  bytesIn: number;
  bytesOut: number;
  warnings: string[];
  durationMs: number;
}

export interface SkippedEntry {
  name: string;
  reason: string;
}

export interface ExtractResult {
  dest: string;
  files: number;
  folders: number;
  bytes: number;
  /** RN-66: ignoradas por segurança (até 50; total em `skippedCount`). */
  skipped: SkippedEntry[];
  skippedCount: number;
  durationMs: number;
}

export type ApiErrorCode =
  | "cancelled"
  | "disk_full"
  | "needs_confirmation"
  | "invalid"
  | "busy"
  | "provider"
  | "internal"
  | "not_found"
  | "unavailable";
