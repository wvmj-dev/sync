import type {
  ActionKind,
  ArchiveFormat,
  RunStatus,
  ScheduleState,
  Situation,
  SyncMode,
  SyncSummary,
  Weekday,
} from "../api/types";

export function formatBytes(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

const dateFormat = new Intl.DateTimeFormat("pt-BR", { dateStyle: "short", timeStyle: "short" });

export function formatDate(iso: string | null): string {
  return iso ? dateFormat.format(new Date(iso)) : "—";
}

const relative = new Intl.RelativeTimeFormat("pt-BR", { numeric: "auto" });

/** "há 15 horas", "em 2 dias", "agora". */
export function formatRelative(iso: string | null): string {
  if (!iso) return "—";
  const seconds = (new Date(iso).getTime() - Date.now()) / 1000;
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of steps) {
    if (Math.abs(seconds) >= size) return relative.format(Math.round(seconds / size), unit);
  }
  return "agora";
}

/** Tom visual de cada resultado de execução. */
export function runTone(status: RunStatus): "ok" | "warn" | "danger" | "neutral" {
  switch (status) {
    case "success":
      return "ok";
    case "partial_success":
    case "awaiting_confirmation":
    case "postponed":
    case "cancelled":
      return "warn";
    case "failed":
    case "aborted":
      return "danger";
    default:
      return "neutral";
  }
}

export const modeLabel: Record<SyncMode, string> = {
  upload: "Envio",
  mirror: "Espelho",
};

export const archiveFormats: { value: ArchiveFormat; label: string; hint: string }[] = [
  { value: "zip", label: "ZIP", hint: "abre em qualquer sistema" },
  { value: "tar_gz", label: "TAR.GZ", hint: "padrão no Linux" },
  { value: "tar_zst", label: "TAR.ZST", hint: "mais rápido e menor" },
  { value: "tar", label: "TAR", hint: "sem compressão" },
];

export function formatLabel(format: ArchiveFormat): string {
  return archiveFormats.find((f) => f.value === format)?.label ?? format;
}

/** "Envio", "Espelho", "Compactado (ZIP)" ou "Recebe". */
export function deliveryLabel(sync: SyncSummary): string {
  switch (sync.delivery.kind) {
    case "archive":
      return sync.delivery.encrypt
        ? `Compactado e criptografado (${formatLabel(sync.delivery.format)})`
        : `Compactado (${formatLabel(sync.delivery.format)})`;
    case "receive":
      return "Recebe";
    default:
      return modeLabel[sync.mode];
  }
}

export const situationLabel: Record<Situation, string> = {
  synced: "Sincronizado",
  pending: "Pendente",
  locally_changed: "Alterado aqui",
  remotely_changed: "Alterado no destino",
  conflict: "Conflito",
  ignored: "Ignorado",
};

export const scheduleStateLabel: Record<ScheduleState, string> = {
  manual: "",
  disabled: "pausado: tarefa desativada",
  paused: "pausado: reconecte a conta",
  waiting_first_run: "começa depois da 1ª execução manual bem-sucedida",
  active: "ativo",
  not_installed: "não instalado — veja os logs",
};

export const weekdays: { value: Weekday; label: string }[] = [
  { value: "mon", label: "segunda" },
  { value: "tue", label: "terça" },
  { value: "wed", label: "quarta" },
  { value: "thu", label: "quinta" },
  { value: "fri", label: "sexta" },
  { value: "sat", label: "sábado" },
  { value: "sun", label: "domingo" },
];

/** RN-80: intervalos que dividem o dia igualmente. */
export const everyHours = [1, 2, 3, 4, 6, 8, 12];

export const runStatusLabel: Record<RunStatus, string> = {
  running: "Em execução",
  success: "Sucesso",
  partial_success: "Sucesso parcial",
  failed: "Falhou",
  aborted: "Abortada",
  awaiting_confirmation: "Aguardando confirmação",
  cancelled: "Cancelada",
  postponed: "Adiada",
  discarded: "Descartada",
};

export const actionLabel: Record<ActionKind, string> = {
  create_folder: "Criar pasta",
  upload: "Enviar",
  update: "Atualizar",
  conflict: "Conflito",
  adopt: "Já idêntico",
  trash: "Lixeira",
  forget: "Esquecer",
  skip: "Ignorado",
};

/** Classe do rótulo de cada situação (RN-101). */
export const situationChip: Record<Situation, string> = {
  synced: "chip-ok",
  pending: "chip-accent",
  locally_changed: "chip-accent",
  remotely_changed: "chip-warn",
  conflict: "chip-danger",
  ignored: "",
};

/** Classe do rótulo de cada ação da prévia. */
export const actionChip: Record<ActionKind, string> = {
  create_folder: "chip-accent",
  upload: "chip-accent",
  update: "chip-accent",
  adopt: "chip-ok",
  conflict: "chip-warn",
  skip: "chip-warn",
  trash: "chip-danger",
  forget: "",
};

/** O que a tarefa faz, em poucas palavras. */
export function kindLabel(sync: SyncSummary): string {
  switch (sync.delivery.kind) {
    case "archive":
      return sync.delivery.encrypt
        ? `Envia compactado e criptografado (${formatLabel(sync.delivery.format)})`
        : `Envia compactado (${formatLabel(sync.delivery.format)})`;
    case "receive":
      return "Recebe e descompacta";
    default:
      return sync.mode === "mirror" ? "Espelha arquivo por arquivo" : "Envia arquivo por arquivo";
  }
}

/** "1 item", "3 itens". */
export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}
