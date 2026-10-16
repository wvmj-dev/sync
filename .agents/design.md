# Design da interface

Leia antes de criar ou mudar qualquer tela. Vale para qualquer IA ou pessoa.

## Direção

- **Nativo do desktop do usuário.** Base: o tema GTK **Orchis** (GNOME). O app **segue o tema
  do sistema**: claro/escuro e cor de destaque vêm do portal do desktop
  (`src-tauri/src/appearance.rs`) e mudam ao vivo. O modo claro do GNOME chega como "sem
  preferência" e vale claro, como nos apps GNOME — o tema GTK (ex.: Orchis-Dark) não decide.
  Sem portal (navegador, outro sistema), vale `prefers-color-scheme` e o azul do Orchis.
- **Minimalista e calmo.** O usuário quer saber se os arquivos estão seguros. Um único
  elemento marcante por tela (na Visão geral, a frase de estado); o resto, quieto.
- **Fonte do sistema** (`system-ui` → Adwaita Sans no GNOME, a do Android no celular). Uma
  família só. Monoespaçada **só para caminhos de arquivo** (`.path`).

## Tokens (`ui/src/styles/main.css`)

Nunca escreva cor, raio ou fonte direto num componente — use as variáveis.

| Token | Claro | Escuro | Uso |
|---|---|---|---|
| `--window` | #F2F2F2 | #212121 | fundo da janela e da barra lateral |
| `--view` | #FFFFFF | #2C2C2C | superfícies (listas em caixa, campos em foco) |
| `--raised` | #FFFFFF | #3C3C3C | o que flutua por cima |
| `--fg` / `--fg-2` / `--fg-3` | preto 87 / 60 / 40% | branco 100 / 70 / 45% | texto principal, secundário, terciário |
| `--line` | preto 12% | branco 12% | bordas e divisórias |
| `--fill` / `--fill-strong` | preto 6 / 10% | branco 8 / 13% | fundo de botões e campos; hover |
| `--accent` | destaque do sistema (padrão #1A73E8) | idem (padrão #3281EA) | ação principal, seleção, foco |
| `--accent-text` | destaque escurecido | destaque clareado | destaque usado como **texto** (contraste) |
| `--success` / `--warning` / `--danger` | Orchis, ajustados para contraste de texto | Orchis-Dark | estados |

Raios em hierarquia: `--radius-l` 16px (blocos), `--radius` 12px (botões e campos, como no
Orchis), `--radius-s` 8px (detalhes), pílula (999px) para rótulos, abas e itens da navegação.
Escala de tipo: 13 / 15 / 17 / 24–28px; pesos 400, 500 (botões e títulos de linha) e 600
(títulos). Números em tabela com `.num` (algarismos de largura fixa).

## Componentes (classes globais)

| Peça | Classe | Quando usar |
|---|---|---|
| Lista em caixa (GNOME) | `.boxed` + `.item` (`.item-icon`, `.item-main`, `.item-title`, `.item-sub`, `.item-end`) | listas de coisas: contas, versões, passos |
| Bloco com espaçamento | `.boxed.pad` | formulários e conteúdo livre (sem divisórias) |
| Seção | `.section` + `.section-head` (`h2`, `.section-desc`) | agrupar blocos numa página |
| Botões | `.btn`, `.btn-primary` (uma por área), `.btn-flat`, `.btn-danger` (neutro, vermelho ao apontar), `.btn-icon`, `.btn-small` | — |
| Rótulo de estado | `.chip` + `.chip-ok` / `.chip-accent` / `.chip-warn` / `.chip-danger` | situação de arquivo, ação da prévia, "Desativada" |
| Aviso | componente `Notice` (`kind`: ok, warn, danger, info; `dismissible`) | resultado, erro, bloqueio |
| Campos | `.field` + `.label`, `.fields` (grade), `.select` (envolve todo `<select>`), `.switch` (interruptor), `.option` (rádio em lista) | formulários |
| Fatos | `<dl class="facts">` | rótulo + valor ("Destino", "Próxima execução") |
| Abas | `.segmented` com `aria-pressed` | alternar painéis de uma tarefa |
| Tabela | `.table-wrap` + `.table` | dados com colunas |
| Ícones | componente `Icon` (`ui/src/components/icons.ts`) | traço 1,75, 24×24; para ícone novo, desenhe no mesmo estilo |

## Regras

- **Nada de genérico de IA:** sem quadros "número grande + legenda", sem rótulos em CAIXA
  ALTA, sem metadados ligados por " · ", sem "→" no fim de botões, sem sombra igual em
  todo bloco, sem gradiente decorativo, sem emoji como ícone.
- **Estrutura é informação:** borda, divisória e rótulo só quando separam algo de verdade.
  Use `dl.facts` em vez de frases com vários dados colados.
- **Texto:** pt-BR, frases curtas em voz ativa, sem jargão interno. O botão diz o que faz
  ("Criar tarefa", não "Enviar"). Erro explica o que houve e como resolver.
- **Acessibilidade:** foco visível (já global), `aria-label` em botão só de ícone, contraste
  dos tokens, `prefers-reduced-motion` respeitado. Movimento só no progresso.
- **Telas estreitas (e, no futuro, o celular):** abaixo de 760px a barra lateral vira barra
  superior; confira que nada estoura a largura.
- **Segurança:** a CSP não permite estilo inline no HTML nem imagens `data:` no CSS —
  tamanho e cor dinâmicos só por `:style` (vira CSSOM) ou variável CSS. `v-html` proibido.

## Revisar o visual

1. `pnpm dev` e abra `http://localhost:1420/?mock&theme=dark` (ou `theme=light`): o modo de
   demonstração (`ui/src/api/mock.ts`) usa dados de exemplo e **não entra no build de
   produção**. Comando novo sem simulação dá erro ali — acrescente em `mock.ts`.
2. Capturas, claro e escuro, desktop e celular: `node scripts/ui-shots.mjs /tmp/shots`.
3. Antes de terminar: `pnpm build` e confira os dois temas.
