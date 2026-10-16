# AGENTS.md — Syncly

Instruções para qualquer agente de IA (Claude Code, Codex, Cursor, Copilot, Gemini…).
Este arquivo é carregado automaticamente pela maioria das ferramentas; o material
detalhado fica em [`.agents/`](.agents/README.md) e deve ser lido **sob demanda**.

## O projeto

App desktop para sincronizar e fazer backup de pastas com o Google Drive (outros
provedores depois). Tauri 2 + Rust (núcleo) + Vue 3/TypeScript (UI) + CLI `syncly`.
Fases concluídas: **0, 1a a 1c, 2a a 2d e 3a** — sincronização (Drive e pasta local), status
por arquivo, transferências avulsas, compactação, envio compactado com versões (backup),
restauração, recebimento e agendamento (systemd + notificações) funcionando. Próximas:
**2e** (criptografia) e **3b** (bandeja). Roadmap em `docs/ARQUITETURA.md` §13.

Fonte da verdade — leia antes de mudar comportamento:

- `docs/REGRAS_DE_NEGOCIO.md` — regras `RN-xx`
- `docs/SEGURANCA.md` — controles `SEG-xx`
- `docs/ARQUITETURA.md` — camadas, fluxos e decisões

## Estado atual

Não presuma que algo existe só porque está nos docs — os docs descrevem o projeto completo.

| Já funciona (fases 0, 1a a 1c, 2a a 2d e 3a) | Ainda **não** existe |
|---|---|
| Domínio e validações; **executor** (`sync/executor.rs`) com progresso, cancelamento e histórico (limitado, RN-93) | Paralelizar a listagem remota (hoje uma pasta por vez) |
| Varredura local com filtros, `.syncignore`, bloqueios e exclusões | Testar o recebimento entre dois computadores (RN-107) |
| Planejador, guardas, política de retry, retenção (funções puras) | Ícone na bandeja |
| Estado em SQLite, config TOML, trava por tarefa, logs em arquivo | |
| `LocalProvider` e `GDriveProvider` (Drive: OAuth PKCE, keyring, limitador, retry, envio retomável) | Changes API, download/bidirecional |
| **Prévia** de sincronização (CLI e UI); enviar/listar/baixar arquivos avulsos e criar pasta (`syncly remote`, tela Arquivos) | |
| Compactar/descompactar zip e tar (`core/src/archive/`, `syncly archive`, tela Compactar) | |
| **Envio compactado** (`Delivery::Archive`, `service/src/delivery.rs`): versões com data, retenção, pula se nada mudou | Criptografia, manifesto |
| **Restaurar versão** (`service/src/restore.rs`, `syncly versions`/`restore`, tela Tarefas → Versões) | |
| **Receber** (`Delivery::Receive`, `service/src/receive.rs`); avulsos "baixar e descompactar" e "enviar pasta compactada" (`transfers.rs`, `remote get --extract`/`put --compress`) | |
| **Agendamento** (`core/src/timers.rs` puro, `service/src/schedule.rs`, `os.rs`; `syncly schedule`; modo sem janela `syncly-desktop run <id> --scheduled`) e notificações D-Bus (RN-94); **adiar** sem rede/keyring com nova tentativa pelo systemd (RN-84) | |
| **Conta a reconectar** (`service/src/auth_watch.rs`; RN-06) pausa os agendamentos dela (RN-85); tela **Preferências** (notificações, transferências, reserva de espaço) | |
| **Status** (`sync/status.rs` puro, `service/src/status.rs`; `syncly status`; tela Tarefas → Status; foto do destino no `state.db`) | |
| **CI** (`.github/workflows/ci.yml`): o checklist abaixo a cada push; `scripts/check-boundary.mjs` confere SEG-30/SEG-32 | |
| **Testes do frontend** (`pnpm test`: Vitest + happy-dom; `*.test.ts` ao lado do código, apoio em `ui/src/testing.ts`) | |
| **Visual** Orchis seguindo o tema do sistema (claro/escuro e destaque via portal, `src-tauri/src/appearance.rs`); guia em `.agents/design.md`; modo de demonstração `?mock` | |

`docs/REGRAS_DE_NEGOCIO.md` marca como *(planejado)* as seções e regras que ainda não existem.

## Mapa

| Caminho | Conteúdo |
|---|---|
| `crates/core` | Domínio, regras puras e motor. **Não** depende de Tauri, UI nem Google. |
| `crates/gdrive` | Provedor Google Drive. |
| `crates/service` | Casos de uso + DTOs. Único lugar que conhece todos os provedores. |
| `crates/cli` | Binário `syncly`. |
| `src-tauri` | Comandos expostos à webview. |
| `ui/src/api` | Único ponto do frontend que chama o Rust. |

Dependências só nesta direção: `cli` / `src-tauri` → `service` → `gdrive` → `core`.

## Comandos

Gerenciador de pacotes JS: **pnpm** (não use `npm` nem `yarn` — o lockfile é `pnpm-lock.yaml`).

```bash
cargo test --workspace
cargo clippy --workspace --all-targets   # deve terminar sem avisos
cargo fmt --all
pnpm test                                # testes do frontend (Vitest)
pnpm build                               # vue-tsc estrito + vite
pnpm tauri dev                           # app com hot reload (abre uma janela)
cargo run -p syncly-cli -- preview <tarefa>
cargo run -p syncly-cli -- run <tarefa> [--confirm]     # executa de verdade
cargo run -p syncly-cli -- google connect               # abre o navegador (conta real!)
cargo run -p syncly-cli -- remote ls|mkdir|put|get …    # arquivos avulsos em Syncly/
cargo run -p syncly-cli -- archive create|extract …     # zip, tar, tar.gz, tar.zst
cargo run -p syncly-cli -- schedule                     # atualiza os timers (systemd real!)
cargo run -p syncly-cli -- status <tarefa> [pasta] [--remote]   # situação e última modificação
```

Testes do Drive usam `wiremock` (Google simulado) e `secrets::MemoryStore` (nunca o keyring
real) — veja `crates/service/src/test_support.rs`.

Teste manual da CLI **sem tocar nos dados reais** do usuário (regra 9):

```bash
export HOME=$(mktemp -d)                 # config, estado e logs vão para lá
cargo run -p syncly-cli -- info
```

Logs: `~/.local/state/syncly/logs/` (nível via `SYNCLY_LOG=debug`).

## Regras inegociáveis

Violar qualquer uma é bug de segurança, mesmo que o código "funcione".

1. **Sem exclusão definitiva de dados do usuário.** Remoção só via `StorageProvider::trash`
   (SEG-10, RN-44). Nada de `files.delete`/esvaziar lixeira no Drive; `remove_file` só em
   temporários do próprio app (`.syncly-tmp-*`) e nas units do systemd que ele gerou
   (cabeçalho `timers::HEADER`, SEG-22).
2. **Escopo OAuth fixo em `drive.file`** (SEG-03). Ampliar exige decisão explícita do usuário.
3. **Segredos não saem do Rust.** Tokens, chaves e o `google-client.json` nunca vão para
   DTOs, logs, mensagens de erro ou git (SEG-04, SEG-33, SEG-52).
4. **Comando Tauri novo** exige as quatro partes: `src-tauri/src/commands.rs`,
   `generate_handler!` em `main.rs`, lista em `src-tauri/build.rs` e permissão em
   `src-tauri/capabilities/default.json` (SEG-30).
5. **Frontend:** proibido `v-html`; `invoke` só dentro de `ui/src/api/` (SEG-32). Visual pelos
   tokens e componentes de `.agents/design.md`.
6. **`unsafe` proibido** (lint `forbid` no workspace).
7. **Decisões de sincronização só no planejador** (`crates/core/src/sync/planner.rs`, RN-30).
   O executor apenas aplica o plano. No envio compactado e no recebimento, as decisões ficam em
   funções puras (`guard::evaluate_archive`/`evaluate_receive`, `archive::versions`,
   `backup::retention`).
8. **Nunca tocar item remoto fora do índice** (RN-31) — no envio compactado, fora das versões
   registradas (RN-55); itens ilegíveis ou excluídos pelos filtros nunca vão para a lixeira
   (RN-27, RN-28).
9. **Testes nunca usam a conta Google real, o `HOME` real nem o systemd real**: use
   `tempfile`, `AppPaths::in_dir`, `LocalProvider` e `test_support::FakeOs`.

## Convenções

- Código que implementa uma regra cita o ID em comentário (`// RN-42`); o teste também
  (`fn rn42_...`).
- Mudou uma regra? Atualize `docs/` **no mesmo commit**. Regra nova ganha número novo;
  números nunca são reutilizados.
- Mudou `crates/service/src/dto.rs`? Atualize `ui/src/api/types.ts` igual.
- Identificadores em inglês; comentários, docs e mensagens ao usuário em português (pt-BR).
- Regra de negócio = função pura + teste unitário. I/O fica nas bordas.
- Erros com `thiserror` por camada; sem `unwrap()` fora de testes.
- Versões de dependências Rust só em `[workspace.dependencies]`.

## Antes de dizer que terminou

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets` sem avisos, `cargo test --workspace`
- [ ] `pnpm test`, `pnpm build` e `node scripts/check-boundary.mjs`, se mexeu em `ui/`, nos DTOs ou em comandos
- [ ] `docs/` atualizados, se alguma regra mudou
- [ ] Seção **Estado atual** deste arquivo e status do `README.md` atualizados, se algo passou a existir
- [ ] Revisão com o papel adequado de `.agents/papeis/`

## Material sob demanda (`.agents/`)

| Arquivo | Quando ler |
|---|---|
| `.agents/receitas.md` | Antes de criar comando, regra, provedor, tela ou dependência nova; para testar manualmente; ao fechar uma fase. |
| `.agents/design.md` | Antes de criar ou mudar qualquer tela: tokens, componentes, regras visuais e como revisar claro/escuro. |
| `.agents/papeis/revisor-seguranca.md` | Ao revisar qualquer mudança em `src-tauri/`, `crates/gdrive/`, providers, I/O ou dependências. |
| `.agents/papeis/guardiao-regras.md` | Quando uma mudança altera comportamento descrito em `docs/`. |
| `.agents/papeis/sincroniza-tipos.md` | Quando mexer em DTOs, comandos Tauri ou `ui/src/api/`. |
