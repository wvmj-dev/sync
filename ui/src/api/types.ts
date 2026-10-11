// Espelho de `crates/service/src/dto.rs`. Ao mudar um lado, mude o outro no mesmo commit.

export type SyncMode = "upload" | "mirror";

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
  backupEnabled: boolean;
  lastRun: RunSummary | null;
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
}

export type ApiErrorCode =
  | "needs_confirmation"
  | "invalid"
  | "busy"
  | "provider"
  | "internal"
  | "not_found"
  | "unavailable";
