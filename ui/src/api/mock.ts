// Dados de exemplo para revisar o visual no navegador (`pnpm dev` + `?mock` no endereço).
// Só é carregado em desenvolvimento — fica fora do build de produção (ver `call` em index.ts).
// `?mock&theme=dark` ou `theme=light` simula o tema do sistema.
import type {
  AccountSummary,
  Appearance,
  AppInfo,
  EncryptionStatus,
  JobStatus,
  JobSummary,
  PlanPreview,
  Preferences,
  RemoteItem,
  VersionItem,
} from "./types";

const hoursAgo = (h: number) => new Date(Date.now() - h * 3_600_000).toISOString();
const inHours = (h: number) => new Date(Date.now() + h * 3_600_000).toISOString();

const info: AppInfo = {
  version: "0.1.0",
  configFile: "~/.config/syncly/config.toml",
  dataDir: "~/.local/share/syncly",
  logDir: "~/.local/state/syncly/logs",
  googleClientConfigured: true,
  googleClientProblem: null,
  configProblem: null,
};

const accounts: AccountSummary[] = [
  { id: "a1", label: "conta@exemplo.com", provider: "google_drive", detail: "conta@exemplo.com", jobCount: 2, needsReconnectSince: null },
  { id: "a2", label: "HD externo", provider: "local", detail: "/run/media/usuario/HD", jobCount: 1, needsReconnectSince: null },
];

const daily = { kind: "daily", at: "02:30" } as const;

const jobs: JobSummary[] = [
  {
    id: "j1",
    name: "Documentos",
    enabled: true,
    sources: ["/home/usuario/Documentos", "/home/usuario/notas.md"],
    exclude: ["*.log"],
    includeSensitive: false,
    sync: {
      accountId: "a1",
      accountLabel: "conta@exemplo.com",
      path: "Backup/Documentos",
      delivery: { kind: "files" },
      mode: "upload",
      schedule: "Diário às 02:30",
      scheduleSpec: daily,
      scheduleState: "active",
      nextRun: inHours(9),
    },
    lastRun: {
      kind: "sync",
      trigger: "scheduled",
      status: "success",
      startedAt: hoursAgo(15),
      finishedAt: hoursAgo(15),
      error: null,
      items: 42,
      bytes: 18_400_000,
    },
  },
  {
    id: "j2",
    name: "Fotos",
    enabled: true,
    sources: ["/home/usuario/Imagens/Fotos"],
    exclude: [],
    includeSensitive: false,
    sync: {
      accountId: "a2",
      accountLabel: "HD externo",
      path: "Backups/Fotos",
      delivery: {
        kind: "archive",
        format: "tar_zst",
        retention: { keep_last: 3, daily: 7, weekly: 4, monthly: 6 },
      },
      mode: "upload",
      schedule: "Todo domingo às 03:00",
      scheduleSpec: { kind: "weekly", weekday: "sun", at: "03:00" },
      scheduleState: "active",
      nextRun: inHours(60),
    },
    lastRun: {
      kind: "backup",
      trigger: "manual",
      status: "partial_success",
      startedAt: hoursAgo(30),
      finishedAt: hoursAgo(30),
      error: null,
      items: 1,
      bytes: 2_100_000_000,
    },
  },
  {
    id: "j3",
    name: "Receber projetos",
    enabled: false,
    sources: [],
    exclude: [],
    includeSensitive: false,
    sync: {
      accountId: "a1",
      accountLabel: "conta@exemplo.com",
      path: "Backups/Projetos",
      delivery: { kind: "receive", target: "/home/usuario/Recebidos" },
      mode: "upload",
      schedule: "Manual",
      scheduleSpec: { kind: "manual" },
      scheduleState: "manual",
      nextRun: null,
    },
    lastRun: null,
  },
];

const preview: PlanPreview = {
  jobId: "j1",
  mode: "upload",
  stats: {
    foldersToCreate: 1,
    uploads: 3,
    updates: 2,
    conflicts: 1,
    adopted: 0,
    trashFiles: 0,
    trashFolders: 0,
    forgotten: 0,
    skipped: 1,
    bytesToSend: 5_300_000,
  },
  verdict: { verdict: "proceed" },
  actions: [
    { kind: "create_folder", path: "Documentos/Contratos", detail: null },
    { kind: "upload", path: "Documentos/Contratos/aluguel.pdf", detail: "1.2 MiB" },
    { kind: "update", path: "Documentos/planilha.ods", detail: "84.0 KiB" },
    { kind: "conflict", path: "notas.md", detail: 'versão remota será renomeada para "notas (conflito remoto 2026-10-15 09-12).md"' },
    { kind: "skip", path: "Documentos/.~lock.planilha.ods#", detail: "arquivo temporário" },
  ],
  totalActions: 5,
  warnings: [],
  hashedFiles: 4,
  durationMs: 182,
  firstRun: false,
  archive: null,
  receive: null,
};

const status: JobStatus = {
  jobId: "j1",
  folder: "Documentos",
  perFile: true,
  items: [
    { path: "Documentos/Contratos", name: "Contratos", kind: "folder", situation: "pending", detail: null, size: 1_250_000, files: 3, localModified: hoursAgo(2), remoteModified: hoursAgo(40), remoteModifiedBy: "Você", syncedAt: hoursAgo(15) },
    { path: "Documentos/planilha.ods", name: "planilha.ods", kind: "file", situation: "locally_changed", detail: "alterado no computador", size: 86_000, files: 1, localModified: hoursAgo(1), remoteModified: hoursAgo(15), remoteModifiedBy: "Você", syncedAt: hoursAgo(15) },
    { path: "Documentos/relatorio.pdf", name: "relatorio.pdf", kind: "file", situation: "synced", detail: null, size: 2_400_000, files: 1, localModified: hoursAgo(50), remoteModified: hoursAgo(15), remoteModifiedBy: "Você", syncedAt: hoursAgo(15) },
    { path: "Documentos/orcamento.xlsx", name: "orcamento.xlsx", kind: "file", situation: "remotely_changed", detail: "alterado no destino", size: 41_000, files: 1, localModified: hoursAgo(70), remoteModified: hoursAgo(3), remoteModifiedBy: "Ana", syncedAt: hoursAgo(15) },
    { path: "Documentos/debug.log", name: "debug.log", kind: "file", situation: "ignored", detail: "excluído pelos filtros", size: 0, files: 0, localModified: null, remoteModified: null, remoteModifiedBy: null, syncedAt: null },
  ],
  totalItems: 5,
  counts: { synced: 38, pending: 3, locallyChanged: 1, remotelyChanged: 1, conflict: 0, ignored: 1 },
  localFiles: 43,
  localBytes: 182_000_000,
  syncedFiles: 40,
  syncedBytes: 178_000_000,
  remoteCheckedAt: null,
  durationMs: 64,
};

const versions: VersionItem[] = [
  { id: "v1", name: "Fotos_2026-10-11_03-00-04.tar.zst", size: 2_100_000_000, createdAt: hoursAgo(30), accountId: "a2", accountLabel: "HD externo", path: "Backups/Fotos", keyFingerprint: null },
  { id: "v2", name: "Fotos_2026-10-04_03-00-02.tar.zst", size: 2_050_000_000, createdAt: hoursAgo(198), accountId: "a2", accountLabel: "HD externo", path: "Backups/Fotos", keyFingerprint: null },
];

const remote: RemoteItem[] = [
  { name: "Backup", path: "Backup", kind: "folder", size: null, modifiedAt: hoursAgo(15), modifiedBy: "Você", downloadable: false },
  { name: "Backups", path: "Backups", kind: "folder", size: null, modifiedAt: hoursAgo(30), modifiedBy: "Você", downloadable: false },
  { name: "apresentacao.zip", path: "apresentacao.zip", kind: "file", size: 12_600_000, modifiedAt: hoursAgo(5), modifiedBy: "Você", downloadable: true },
  { name: "contrato.pdf", path: "contrato.pdf", kind: "file", size: 310_000, modifiedAt: hoursAgo(80), modifiedBy: "Ana", downloadable: true },
];

function appearance(): Appearance {
  const theme = new URLSearchParams(location.search).get("theme");
  return {
    scheme: theme === "dark" || theme === "light" ? theme : null,
    accent: new URLSearchParams(location.search).get("accent"),
  };
}

let preferences: Preferences = { notify: "failures", minFreeSpaceMib: 1024, maxParallelTransfers: 4 };

const MOCK_KEY = "3f9a1c0b7d2e4a65";
let encryption: EncryptionStatus = { active: null, known: [] };

/** `?mock&encrypted`: chave ativa e versões de "Fotos" criptografadas (RN-70, RN-74). */
if (new URLSearchParams(location.search).has("encrypted")) {
  encryption = { active: MOCK_KEY, known: [MOCK_KEY, "9b04e7d1c2a35f80"] };
  const fotos = jobs.find((j) => j.id === "j2");
  if (fotos?.sync?.delivery.kind === "archive") fotos.sync.delivery.encrypt = true;
  for (const v of versions) {
    v.name += ".age";
    v.keyFingerprint = v.id === "v1" ? MOCK_KEY : "9b04e7d1c2a35f80";
  }
}

const responses: Record<string, () => unknown> = {
  get_preferences: () => preferences,
  encryption_status: () => encryption,
  system_appearance: appearance,
  app_info: () => info,
  list_accounts: () => accounts,
  list_jobs: () => jobs,
  preview_job: () => preview,
  job_status: () => status,
  refresh_job_status: () => ({ ...status, remoteCheckedAt: new Date().toISOString() }),
  list_versions: () => versions,
  list_remote: () => remote,
};

/** `?mock&reconnect`: a conta do Google precisa ser reconectada (RN-06, RN-85). */
if (new URLSearchParams(location.search).has("reconnect")) {
  const google = accounts.find((a) => a.id === "a1");
  if (google) google.needsReconnectSince = hoursAgo(20);
  for (const job of jobs) {
    if (job.sync?.accountId === "a1" && job.sync.scheduleState === "active") {
      job.sync.scheduleState = "paused";
      job.sync.nextRun = null;
    }
  }
}

/** `?mock&postponed`: a última execução agendada de "Documentos" ficou sem rede (RN-84). */
if (new URLSearchParams(location.search).has("postponed")) {
  const docs = jobs.find((j) => j.id === "j1");
  if (docs) {
    docs.lastRun = {
      kind: "sync",
      trigger: "scheduled",
      status: "postponed",
      startedAt: hoursAgo(0.2),
      finishedAt: hoursAgo(0.2),
      error: "sem conexão com a internet; nova tentativa em 15 min",
      items: null,
      bytes: null,
    };
  }
}

export async function mockCall<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (command === "create_remote_folder") {
    const { parent, name } = args as { parent: string; name: string };
    if (remote.some((r) => r.name === name)) throw new Error(`já existe '${name}' em Syncly/${parent} — nada foi sobrescrito`);
    const item: RemoteItem = { name, path: parent ? `${parent}/${name}` : name, kind: "folder", size: null, modifiedAt: new Date().toISOString(), modifiedBy: "Você", downloadable: false };
    if (!parent) remote.unshift(item);
    return item as T;
  }
  if (command === "create_encryption_key") {
    const folder = (args?.folder as string) ?? "/tmp";
    return {
      fingerprint: MOCK_KEY,
      publicKey: "age1demonstracao0000000000000000000000000000000000000000000",
      recoveryFile: `${folder}/syncly-chave-${MOCK_KEY}.txt`,
    } as T;
  }
  if (command === "activate_encryption_key") {
    if (!args?.saved) throw new Error("confirme que guardou a chave de recuperação");
    encryption = { active: MOCK_KEY, known: [MOCK_KEY, ...encryption.known.filter((k) => k !== MOCK_KEY)] };
    return encryption as T;
  }
  if (command === "import_encryption_key") {
    const imported = "9b04e7d1c2a35f80";
    encryption = {
      active: encryption.active ?? imported,
      known: [...new Set([...encryption.known, imported])],
    };
    return encryption as T;
  }
  if (command === "save_preferences") {
    preferences = args?.preferences as Preferences;
    return preferences as T;
  }
  const response = responses[command];
  if (!response) throw new Error(`modo de demonstração: '${command}' não simulado`);
  return response() as T;
}
