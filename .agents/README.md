# .agents/

Material para agentes de IA, independente de ferramenta. A ideia é ter **uma única fonte**
de instruções, sem duplicar conteúdo em `CLAUDE.md`, `.cursor/`, `.github/` etc.

```
AGENTS.md              ← raiz do repo: carregado automaticamente; curto, só o essencial
.agents/
├── README.md          ← este arquivo
├── receitas.md        ← passo a passo das mudanças mais comuns
└── papeis/            ← "personas" com checklist, para revisões focadas
    ├── revisor-seguranca.md
    ├── guardiao-regras.md
    └── sincroniza-tipos.md
```

O `AGENTS.md` fica na raiz porque é lá que as ferramentas procuram. Tudo que é longo ou
específico de uma tarefa fica aqui e é lido só quando necessário, para não ocupar o
contexto do agente à toa.

## Como cada ferramenta usa

| Ferramenta | Como lê |
|---|---|
| Claude Code (≥ 2.1.277) | Lê `AGENTS.md` sozinho quando **não** existe `CLAUDE.md`. Em versões antigas, crie um `CLAUDE.md` contendo só a linha `@AGENTS.md`. |
| OpenAI Codex | `AGENTS.md` nativo. |
| Cursor | `AGENTS.md` nativo. |
| GitHub Copilot (agente) | `AGENTS.md` nativo. |
| Gemini CLI | Configure o arquivo de contexto (`context.fileName`) para incluir `AGENTS.md` — confira o nome da opção na doc da sua versão. |
| Outras | Comece a conversa com: "leia o `AGENTS.md` antes de tudo". |

> Não crie `CLAUDE.md`, `GEMINI.md` ou similares com conteúdo próprio: no Claude Code,
> a existência de um `CLAUDE.md` faz o `AGENTS.md` deixar de ser lido. Se uma ferramenta
> exigir arquivo próprio, ele deve apenas apontar para o `AGENTS.md`.

## Como usar os papéis

Peça explicitamente, em qualquer ferramenta:

> Atue conforme `.agents/papeis/revisor-seguranca.md` e revise as mudanças não commitadas.

Cada papel define objetivo, passos, formato de saída e o que **não** fazer. Papéis só
revisam e apontam problemas — quem decide aplicar as correções é você.

## Como criar um papel novo

Copie um existente e mantenha as seções: **Objetivo**, **Quando usar**, **Passos**,
**Saída esperada** e **Não faça**. Cite regras pelo ID (`RN-xx`, `SEG-xx`) em vez de
repetir o texto delas — assim o papel não fica desatualizado quando a regra mudar.
Depois, liste o papel na tabela do `AGENTS.md`.
