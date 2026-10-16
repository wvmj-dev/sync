// Segue o tema do sistema: claro/escuro e cor de destaque (lidos do portal do desktop pelo
// Rust). Sem essa informação, o CSS usa `prefers-color-scheme` e o azul do Orchis.
import { api, onAppearanceChanged } from "./api";
import type { Appearance } from "./api/types";

export function applyAppearance(appearance: Appearance) {
  const root = document.documentElement;
  if (appearance.scheme) root.dataset.theme = appearance.scheme;
  else delete root.dataset.theme;
  // Só aceita #rrggbb: o valor vira uma variável CSS.
  if (appearance.accent && /^#[0-9a-f]{6}$/i.test(appearance.accent)) {
    root.style.setProperty("--system-accent", appearance.accent);
  } else {
    root.style.removeProperty("--system-accent");
  }
}

async function refresh() {
  try {
    applyAppearance(await api.systemAppearance());
  } catch {
    // Fora do app: o padrão do navegador decide.
  }
}

export async function followSystemTheme() {
  await refresh();
  // Mudanças chegam pelo evento do portal; reler ao voltar o foco cobre o caso de o
  // aviso não chegar (ex.: portal reiniciado).
  window.addEventListener("focus", () => void refresh());
  await onAppearanceChanged(applyAppearance);
}
