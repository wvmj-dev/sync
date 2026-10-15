# Syncly

Sincronização e backup de pastas e arquivos com o Google Drive — seguro por padrão,
rápido e preparado para outros provedores. App desktop em **Tauri 2 + Rust + Vue 3**,
com uma CLI (`syncly`) que também servirá aos agendamentos.

## Status

Já dá para **sincronizar pastas** com o Google Drive ou com uma pasta local (HD externo) —
com prévia, execução, progresso e cancelamento —, fazer **backup compactado com versões**
(local ou no Drive), **enviar e baixar arquivos avulsos** e **compactar/descompactar** em ZIP
e TAR. Tudo verificado por checksum.

| Funcionalidade | Situação |
|---|---|
| Contas: Google Drive (login no navegador, credencial no keyring) e pastas locais | ✅ |
| Tarefas: várias pastas/arquivos, destino, modo envio/espelho, exclusões | ✅ |
| **Envio compactado** (backup): uma versão com data por execução, retenção, pula se nada mudou | ✅ |
| Prévia da sincronização (enviar, atualizar, conflitos, lixeira) | ✅ |
| **Executar a sincronização**, com progresso, cancelamento e histórico | ✅ |
| Proteções: arquivos sensíveis, disco desmontado, exclusão em massa, 1ª execução, espaço na conta | ✅ |
| Enviar, listar e baixar arquivos avulsos | ✅ |
| **Compactar e descompactar** (zip, tar, tar.gz, tar.zst), protegido contra arquivos maliciosos | ✅ |
| Logs em arquivo com rotação | ✅ |
| Tela de status / última modificação de cada arquivo | fase 1c |
| Restaurar uma versão (baixar e descompactar) | fase 2c (hoje: tela Arquivos + Compactar) |
| Criptografia das versões | fase 2d |
| Agendamento, notificações, ícone na bandeja | fase 3 |
| Sincronização bidirecional | fase 4 |
| Outros provedores; macOS e Windows | fase 5 |

## Segurança

- Acessa **só os arquivos que ele mesmo cria** no Drive (escopo `drive.file`).
- **Nunca apaga nada de vez**: remoções vão para a lixeira, com proteção contra exclusão em massa.
- **Arquivos sensíveis** (`.ssh`, `.env`, chaves…) ficam de fora por padrão.
- Sua senha nunca passa pelo app; tokens ficam no **keyring do sistema**.
- Arquivos compactados de terceiros são tratados como **não confiáveis** ao descompactar.

Detalhes em [docs/SEGURANCA.md](docs/SEGURANCA.md).

## Começando

Requisitos (Fedora): Rust ≥ 1.90, Node 20.19+ ou 22.12+, pnpm (`corepack enable`) e
`webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel librsvg2-devel`.

```bash
pnpm install
pnpm tauri dev        # abre a janela do app (não use `pnpm dev`: é só o frontend, sem o Rust)
```

Para usar o Google Drive, configure antes o seu projeto no Google Cloud:
[docs/CONFIGURAR_GOOGLE.md](docs/CONFIGURAR_GOOGLE.md) (~10 minutos).

## Como usar

| Tela | Para quê |
|---|---|
| **Visão geral** | Primeiros passos, tarefas e última execução, onde ficam config, dados e logs. |
| **Contas** | **Conectar Google Drive** (abre o navegador; clique em *Continuar* no aviso "O Google não verificou este app" — é o seu próprio app) e adicionar **pastas locais** (HD externo, NAS). |
| **Tarefas** | Criar uma tarefa (fontes + destino + como enviar), **Pré-visualizar** e **Executar**. Na primeira execução o app pede confirmação. |
| **Arquivos** | Navegar na pasta `Syncly/` de uma conta, **enviar** e **baixar** arquivos avulsos. |
| **Compactar** | Compactar pastas/arquivos em ZIP, TAR.GZ, TAR.ZST ou TAR, e descompactar numa pasta nova. |

Como uma tarefa envia:

- **Arquivo por arquivo**, no modo:
  - **Envio** (recomendado) — manda o que é novo ou mudou; nunca apaga nada no destino.
  - **Espelho** — o que você apagar no computador vai para a **lixeira** do destino
    (recuperável por 30 dias no Drive), com proteção contra exclusões em massa.
- **Compactado** (backup) — a cada execução, tudo vira um único arquivo
  `Tarefa_AAAA-MM-DD_HH-MM-SS.zip` (ou `.tar.gz`, `.tar.zst`, `.tar`). Guarda as últimas 3
  versões + 1 por dia (7 dias), por semana (4) e por mês (6) — configurável; as mais antigas
  vão para a lixeira. Se nada mudou, nada é enviado. Ideal para pastas com muitos arquivos
  pequenos (ex.: saves de jogos). Com uma conta do tipo pasta local, é um backup
  local (HD externo).

Dicas:

- Feche programas que escrevem nos arquivos da tarefa (ex.: um jogo com o mundo aberto).
  Não corrompe nada, mas o que mudar durante o envio é reenviado na próxima execução.
- Da segunda execução em diante, só vai o que mudou.
- Compactar não é criptografar: as versões ficam legíveis no Drive, como os demais arquivos.
- Enquanto o app estiver em "Testando" no Google Cloud, o login vale por 7 dias; depois,
  **Contas → Reconectar**.
- Nada some sem querer: o app nunca sobrescreve nem apaga definitivamente — envio que já
  existe é recusado, download e compactação usam `nome (2)`.

### CLI

```bash
cargo run -p syncly-cli -- info                       # onde ficam config, dados e logs
cargo run -p syncly-cli -- accounts
cargo run -p syncly-cli -- jobs
cargo run -p syncly-cli -- preview Documentos         # nome ou ID da tarefa
cargo run -p syncly-cli -- run Documentos --confirm   # executa (1ª vez exige --confirm)
cargo run -p syncly-cli -- jobs                       # mostra o tipo: envio, espelho ou compactado

cargo run -p syncly-cli -- google connect             # abre o navegador para autorizar
cargo run -p syncly-cli -- remote put ~/foto.jpg Testes          # envia para Syncly/Testes/
cargo run -p syncly-cli -- remote ls Testes
cargo run -p syncly-cli -- remote get Testes/foto.jpg ~/Downloads

cargo run -p syncly-cli -- archive create ~/Fotos --format zip --out ~/Downloads
cargo run -p syncly-cli -- archive create ~/projeto ~/notas.md --format tar.gz --name backup
cargo run -p syncly-cli -- archive extract ~/Downloads/Fotos.zip ~/Desktop
```

Com mais de uma conta, escolha com `--account <nome ou e-mail>`. Para instalar o binário
`syncly` no PATH: `cargo install --path crates/cli`. Códigos de saída do `run`: 0 sucesso,
1 falha, 2 exige confirmação, 3 sucesso parcial.

### Onde ficam os dados

| O quê | Onde |
|---|---|
| Configuração (contas, tarefas) | `~/.config/syncly/config.toml` |
| Cliente OAuth do Google | `~/.config/syncly/google-client.json` |
| Estado (índice, histórico) | `~/.local/share/syncly/state.db` |
| Logs (14 dias; `SYNCLY_LOG=debug` para mais detalhes) | `~/.local/state/syncly/logs/` |
| Tokens e chaves | keyring do sistema (GNOME Keyring/KWallet) |

Nada disso fica no repositório — credenciais e dados pessoais nunca vão para o git.

## Desenvolvimento

Antes de commitar:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets && cargo test --workspace
pnpm build
```

Os testes nunca usam a conta Google real, o keyring real nem o seu `HOME`: o Drive é
simulado (`wiremock`) e o cofre roda em memória.

Pacotes de distribuição (`.rpm`, `.deb`, AppImage em `target/release/bundle/`):

```bash
pnpm tauri build
```

### Estrutura

```
crates/core      domínio, regras, motor de sincronização e compactação (sem Tauri, sem Google)
crates/gdrive    provedor Google Drive (OAuth, API REST, envio retomável)
crates/service   casos de uso compartilhados por GUI e CLI, cofre de segredos, logs
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
