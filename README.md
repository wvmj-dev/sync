# Syncly

Sincronização e backup de pastas e arquivos com o Google Drive — seguro por padrão,
rápido e preparado para outros provedores. App desktop em **Tauri 2 + Rust + Vue 3**,
com uma CLI (`syncly`) que também serve aos agendamentos.

## Status

**Fase 0 — esqueleto funcional.** Já dá para cadastrar contas e tarefas e ver a **prévia**
do que uma sincronização faria; executar e falar com o Google Drive chega na fase 1.

| Funcionalidade | Situação |
|---|---|
| Contas locais (HD externo, NAS montado) | ✅ |
| Tarefas: várias pastas/arquivos, destino, modo envio/espelho, exclusões | ✅ |
| Prévia da sincronização (enviar, atualizar, conflitos, lixeira) — UI e CLI | ✅ |
| Proteções: arquivos sensíveis, disco desmontado, exclusão em massa, 1ª execução | ✅ |
| Logs em arquivo com rotação | ✅ |
| Executar a sincronização | fase 1 |
| Google Drive (login e envio) | fase 1 |
| Backup local/remoto, compactação, criptografia, restauração | fase 2 |
| Agendamento, notificações, ícone na bandeja | fase 3 |
| Sincronização bidirecional | fase 4 |
| Outros provedores; macOS e Windows | fase 5 |

## Segurança

- Acessa **só os arquivos que ele mesmo cria** no Drive (escopo `drive.file`).
- **Nunca apaga nada de vez**: remoções vão para a lixeira, com proteção contra exclusão em massa.
- **Arquivos sensíveis** (`.ssh`, `.env`, chaves…) ficam de fora por padrão.
- Sua senha nunca passa pelo app; tokens ficam no **keyring do sistema**.

Detalhes em [docs/SEGURANCA.md](docs/SEGURANCA.md).

## Começando

Requisitos (Fedora): Rust ≥ 1.89, Node 20.19+ ou 22.12+, pnpm (`corepack enable`) e
`webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel librsvg2-devel`.

```bash
pnpm install
pnpm tauri dev        # abre o app com hot reload
```

Para usar o Google Drive (a partir da fase 1), configure antes o seu projeto no Google
Cloud: [docs/CONFIGURAR_GOOGLE.md](docs/CONFIGURAR_GOOGLE.md) (~10 minutos).

### CLI

```bash
cargo run -p syncly-cli -- info                 # onde ficam config, dados e logs
cargo run -p syncly-cli -- accounts
cargo run -p syncly-cli -- jobs
cargo run -p syncly-cli -- preview Documentos   # nome ou ID da tarefa
```

Para instalar o binário `syncly` no PATH: `cargo install --path crates/cli`.

### Onde ficam os dados

| O quê | Onde |
|---|---|
| Configuração (contas, tarefas) | `~/.config/syncly/config.toml` |
| Cliente OAuth do Google | `~/.config/syncly/google-client.json` |
| Estado (índice, histórico) | `~/.local/share/syncly/state.db` |
| Logs (14 dias; `SYNCLY_LOG=debug` para mais detalhes) | `~/.local/state/syncly/logs/` |
| Tokens e chaves | keyring do sistema (GNOME Keyring/KWallet) |

## Desenvolvimento

Antes de commitar:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets && cargo test --workspace
pnpm build
```

Pacotes de distribuição (`.rpm`, `.deb`, AppImage em `target/release/bundle/`):

```bash
pnpm tauri build
```

### Estrutura

```
crates/core      domínio, regras e motor (sem Tauri, sem Google)
crates/gdrive    provedor Google Drive
crates/service   casos de uso compartilhados por GUI e CLI, logs
crates/cli       binário `syncly`
src-tauri        app desktop (comandos com allowlist)
ui/              frontend Vue 3 + TypeScript
docs/            arquitetura, regras, segurança, configuração do Google
.agents/         material para agentes de IA (ver AGENTS.md)
```

## Documentação

| Documento | Conteúdo |
|---|---|
| [ARQUITETURA.md](docs/ARQUITETURA.md) | Camadas, fluxos, desempenho, decisões e roadmap |
| [REGRAS_DE_NEGOCIO.md](docs/REGRAS_DE_NEGOCIO.md) | Todas as regras (`RN-xx`), incluindo a tabela de decisão da sincronização |
| [SEGURANCA.md](docs/SEGURANCA.md) | Por que sua conta Google não corre risco; controles `SEG-xx` |
| [CONFIGURAR_GOOGLE.md](docs/CONFIGURAR_GOOGLE.md) | Passo a passo do projeto no Google Cloud |

Trabalhando com IA? As instruções para agentes (Claude Code, Codex, Cursor, Copilot,
Gemini…) estão em [AGENTS.md](AGENTS.md) e na pasta [.agents/](.agents/README.md).
