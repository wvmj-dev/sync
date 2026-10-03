import type { ActionKind, RunStatus, SyncMode } from "../api/types";

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
