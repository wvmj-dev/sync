import type {
  ActionKind,
  ArchiveFormat,
  RunStatus,
  ScheduleState,
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
      return `Compactado (${formatLabel(sync.delivery.format)})`;
    case "receive":
      return "Recebe";
    default:
      return modeLabel[sync.mode];
  }
}

export const scheduleStateLabel: Record<ScheduleState, string> = {
  manual: "",
  disabled: "pausado: tarefa desativada",
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
