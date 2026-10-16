// Espelho de `crates/service/src/dto.rs`. Ao mudar um lado, mude o outro no mesmo commit.

export type SyncMode = "upload" | "mirror";

/** RN-94: o que as execuções agendadas avisam. */
export type NotifyPolicy = "always" | "failures" | "never";

/** Preferências (RN-46, RN-94, SEG-41). */
export interface Preferences {
  notify: NotifyPolicy;
  minFreeSpaceMib: number;
  maxParallelTransfers: number;
}

/** Tema do sistema, lido do portal do desktop. */
export interface Appearance {
  /** `null` = sem portal do desktop (vale `prefers-color-scheme`). */
  scheme: "light" | "dark" | null;
  /** Cor de destaque em `#rrggbb`. */
  accent: string | null;
}

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

/** RN-80. Horários em "HH:MM". */
export type Schedule =
  | { kind: "manual" }
  | { kind: "every_hours"; hours: number }
  | { kind: "daily"; at: string }
  | { kind: "weekly"; weekday: Weekday; at: string }
  | { kind: "calendar"; expr: string };

/** Situação do timer da tarefa (RN-18, RN-36). */
export type ScheduleState =
  | "manual"
  | "disabled"
  | "paused"
  | "waiting_first_run"
  | "active"
  | "not_installed";

/** RN-54. Vem do core com os mesmos nomes do config.toml (snake_case). */
export interface Retention {
  keep_last: number;
  daily: number;
  weekly: number;
  monthly: number;
}

/** O que a tarefa faz: envia arquivo por arquivo, envia compactado (RN-50) ou recebe (RN-107). */
export type Delivery =
  | { kind: "files" }
  | { kind: "archive"; format: ArchiveFormat; retention: Retention }
  | { kind: "receive"; target: string };

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
  /** RN-06: desde quando a conta precisa ser reconectada (`null` = tudo certo). */
  needsReconnectSince: string | null;
}

export type RunStatus =
  | "running"
  | "success"
  | "partial_success"
  | "failed"
  | "aborted"
  | "awaiting_confirmation"
  | "cancelled"
  | "postponed"
  | "discarded";

export interface RunSummary {
  kind: "sync" | "backup" | "restore" | "compress" | "receive";
  trigger: "manual" | "scheduled";
  status: RunStatus;
  startedAt: string;
  finishedAt: string | null;
  error: string | null;
  /** RN-100: itens transferidos e bytes. */
  items: number | null;
  bytes: number | null;
}

export interface SyncSummary {
  accountId: string;
  accountLabel: string | null;
  path: string;
  delivery: Delivery;
  mode: SyncMode;
  /** Rótulo legível ("Diário às 02:00"). */
  schedule: string;
  scheduleSpec: Schedule;
  scheduleState: ScheduleState;
  /** RN-100: próximo disparo do timer, se ativo. */
  nextRun: string | null;
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

/** "Enviar compactado" avulso (RN-109). */
export interface UploadCompressedRequest {
  accountId: string;
  inputs: string[];
  /** Pasta dentro de Syncly/ ("" = raiz). */
  remoteDir: string;
  format: ArchiveFormat;
  /** Sem extensão; `null` = nome da entrada. */
  name: string | null;
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

/** RN-101. */
export type Situation =
  | "ignored"
  | "synced"
  | "pending"
  | "remotely_changed"
  | "locally_changed"
  | "conflict";

export interface ItemStatus {
  path: string;
  name: string;
  kind: "file" | "folder";
  situation: Situation;
  detail: string | null;
  /** Arquivo: tamanho; pasta: soma dos arquivos dentro (RN-102). */
  size: number;
  /** Arquivos dentro (pasta); 1 para arquivo. */
  files: number;
  localModified: string | null;
  remoteModified: string | null;
  remoteModifiedBy: string | null;
  syncedAt: string | null;
}

export interface SituationCounts {
  synced: number;
  pending: number;
  locallyChanged: number;
  remotelyChanged: number;
  conflict: number;
  ignored: number;
}

/** RN-100 a RN-103. */
export interface JobStatus {
  jobId: string;
  folder: string;
  /** Tarefas compactadas ou que recebem não têm status por arquivo. */
  perFile: boolean;
  items: ItemStatus[];
  totalItems: number;
  counts: SituationCounts;
  localFiles: number;
  localBytes: number;
  syncedFiles: number;
  syncedBytes: number;
  /** `null` = dados do último sync. */
  remoteCheckedAt: string | null;
  durationMs: number;
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
  /** `null` mantém o agendamento atual. */
  schedule: Schedule | null;
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
  /** Presente quando a tarefa recebe; aí `stats` e `actions` ficam vazios. */
  receive: ReceivePreview | null;
}

/** RN-107. */
export interface ReceivePreview {
  /** Versão mais recente no destino; `null` se não houver nenhuma. */
  name: string | null;
  size: number | null;
  modifiedAt: string | null;
  alreadyReceived: boolean;
  /** Pasta do computador onde a pasta nova será criada. */
  target: string;
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
  /** Presente quando a tarefa recebe. */
  receive: ReceiveRun | null;
}

export interface ReceiveRun {
  name: string;
  /** Pasta criada (ou, se nada mudou, a do último recebimento). */
  dest: string;
  files: number;
  bytes: number;
  skippedCount: number;
  unchanged: boolean;
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
  | "unavailable"
  | "needs_reconnect"
  | "postponed";
