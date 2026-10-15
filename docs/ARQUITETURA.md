# Arquitetura do Syncly

Syncly é um app desktop (Tauri 2 + Rust + Vue 3) para sincronizar pastas e arquivos
com provedores de nuvem — começando pelo Google Drive — com backups locais/remotos,
compactação, agendamento e histórico de modificações.

Documentos relacionados:

- [REGRAS_DE_NEGOCIO.md](REGRAS_DE_NEGOCIO.md) — o que o app faz, regra por regra (`RN-xx`).
- [SEGURANCA.md](SEGURANCA.md) — modelo de ameaças e controles (`SEG-xx`), incluindo
  por que sua conta Google não corre risco.
- [CONFIGURAR_GOOGLE.md](CONFIGURAR_GOOGLE.md) — passo a passo do projeto no Google Cloud.

---

## 1. Princípios

Quando dois objetivos conflitarem, vale esta ordem de prioridade:

1. **Segurança** da conta e das credenciais.
2. **Integridade** dos dados — nunca perder arquivo.
3. **Velocidade**.
4. Conveniência.

Princípios de projeto:

| Princípio | Na prática |
|---|---|
| Núcleo independente | `syncly-core` não conhece Tauri, Vue nem Google. Provedores entram por uma interface (`StorageProvider`). |
| Regras como funções puras | Planejador, guardas, retenção, filtros e validações são funções sem I/O, cobertas por testes unitários. |
| Nada é apagado de verdade | A interface de provedor **não tem** método de exclusão definitiva — só "mover para a lixeira". |
| Idempotente e retomável | O índice é gravado arquivo a arquivo; uma execução interrompida continua de onde parou na próxima. |
| Menor privilégio | Escopo OAuth `drive.file`; a webview só chama comandos de alto nível; tokens nunca saem do Rust. |
| Duas portas, um motor | GUI (Tauri) e CLI (`syncly`) chamam os mesmos casos de uso em `syncly-service`. |

## 2. Visão geral

```
 ┌───────────────┐                        ┌──────────────────┐
 │  UI (Vue 3)   │                        │  CLI  (syncly)   │◄── timers do systemd --user
 └───────┬───────┘                        └────────┬─────────┘
         │ invoke() — IPC com allowlist            │
 ┌───────▼───────┐                                 │
 │   src-tauri   │  comandos e eventos             │
 └───────┬───────┘                                 │
         └──────────────────┬──────────────────────┘
                    ┌───────▼────────┐
                    │ syncly-service │  casos de uso + DTOs
                    └───┬────────┬───┘
                        │        │
           ┌────────────▼──┐  ┌──▼─────────────┐
           │  syncly-core  │◄─┤ syncly-gdrive  │  implementa StorageProvider
           │ domínio+motor │  └────────────────┘
           └───────────────┘
```

### Regra de dependência

```
cli ──► service ──► core
src-tauri ──► service ──► gdrive ──► core
```

- `core` não depende de nenhum outro crate do projeto.
- Provedores (`gdrive`, futuros `onedrive`, `dropbox`, `s3`) dependem só de `core`.
- `service` é o único lugar que conhece todos os provedores (fábrica de provedores).
- `src-tauri` e `cli` são adaptadores finos: convertem entrada/saída e chamam `service`.

## 3. Crates

| Crate | Tipo | Responsabilidade |
|---|---|---|
| `crates/core` (`syncly-core`) | lib | Domínio (tarefas, contas, agendamento), validações, filtros, varredura local, planejador, guardas, **executor**, política de retry, retenção, **compactar/descompactar**, estado (SQLite), config (TOML), trava por tarefa, `LocalProvider`. |
| `crates/gdrive` (`syncly-gdrive`) | lib | Provedor Google Drive: OAuth (PKCE + loopback), chamadas REST v3, upload resumable, classificação de erros, escape de queries. |
| `crates/service` (`syncly-service`) | lib | Casos de uso (contas, tarefas, prévia, execução, login Google, transferências avulsas, compactação), DTOs da interface, fábrica de provedores, cofre de segredos, logs. |
| `crates/cli` (`syncly-cli`, binário `syncly`) | bin | Interface de linha de comando; é o que os timers do systemd executam. |
| `src-tauri` (`syncly-desktop`) | bin | App desktop: registra comandos (allowlist), plugins e eventos de progresso. Bandeja na fase 3. |
| `ui/` | Vue 3 + TS | Telas. Toda chamada ao Rust passa por `ui/src/api/`. |

## 4. Estrutura de pastas

```
.
├── Cargo.toml                 # workspace: versões e lints centralizados
├── package.json               # frontend + CLI do Tauri
├── vite.config.ts             # root = ui/
├── AGENTS.md                  # instruções para agentes de IA (qualquer ferramenta)
├── .agents/                   # receitas e papéis de revisão, lidos sob demanda
├── docs/
├── crates/
│   ├── core/src/
│   │   ├── domain/            # tipos e validações (account, job, schedule, rel_path, plan…)
│   │   ├── sync/              # scanner, remote_tree, planner, guard, executor, retry
│   │   ├── archive/           # compactar/descompactar zip e tar (gz, zst), nome e impressão digital das versões
│   │   ├── backup/            # retention: quais versões manter (RN-54)
│   │   ├── providers/local.rs # LocalProvider
│   │   ├── provider.rs        # trait StorageProvider + classificação de erros
│   │   ├── hash.rs            # hash em streaming no algoritmo de cada provedor
│   │   ├── fsutil.rs          # escrita atômica, permissões 0700/0600
│   │   ├── filters.rs         # exclusões padrão e de arquivos sensíveis
│   │   ├── config.rs          # leitura/escrita atômica do config.toml
│   │   ├── state.rs           # SQLite: índice, execuções, versões compactadas
│   │   ├── lock.rs            # trava exclusiva por tarefa
│   │   └── paths.rs           # diretórios XDG
│   ├── gdrive/src/            # auth, oauth, client, provider, endpoints, query, error
│   ├── service/src/           # service (tarefas/execução), delivery (envio compactado), restore, google, transfers, archive, secrets, dto, logging
│   └── cli/src/
├── src-tauri/
│   ├── capabilities/default.json  # allowlist de comandos da janela
│   ├── build.rs                   # declara os comandos do app (permissões)
│   └── src/commands.rs
└── ui/
    ├── index.html
    └── src/
        ├── api/               # ÚNICO lugar que chama invoke(); tipado
        ├── stores/            # Pinia
        ├── views/             # Visão geral, Tarefas, Contas, Arquivos, Compactar
        └── components/        # JobCard, JobForm, PlanPreview
```

## 5. Onde ficam os dados

| O quê | Onde (Linux/XDG) | Formato | Permissão |
|---|---|---|---|
| Configuração (contas, tarefas, preferências) | `~/.config/syncly/config.toml` | TOML versionado (`version = 1`) | `0600` |
| Cliente OAuth do Google | `~/.config/syncly/google-client.json` | JSON baixado do Google Cloud | `0600` |
| Estado (índice, execuções, versões enviadas) | `~/.local/share/syncly/state.db` | SQLite (WAL) | dir `0700` |
| Temporários do envio compactado | `~/.local/share/syncly/tmp/<tarefa>/.syncly-tmp-*` | apagados ao fim de cada execução | dir `0700` |
| Logs | `~/.local/state/syncly/logs/` (`syncly-desktop.AAAA-MM-DD.log`, `syncly-cli.…`) | texto, rotação diária, 14 dias; nível via `SYNCLY_LOG` | dir `0700` |
| Travas | `$XDG_RUNTIME_DIR/syncly/` | arquivo + `flock` | dir `0700` |
| Tokens OAuth e chaves de criptografia | Keyring do sistema (Secret Service: GNOME Keyring/KWallet) | — | cifrado pelo SO |

**Configuração ≠ estado.** O `config.toml` diz *o que o usuário quer* (pode ser lido,
editado e copiado para outra máquina). O `state.db` guarda *o que o app sabe* (índice,
histórico) e pode ser reconstruído (RN-56).

## 6. Fluxo de sincronização

```
 1. trava da tarefa ──► 2. validar ──► 3. escanear local ──► 4. estado remoto
                                                                   │
 8. registrar ◄── 7. executar ◄── 6. prévia/confirmação ◄── 5. planejar + guardas
```

1. **Trava** (`lock.rs`) — uma execução por tarefa; GUI e timer nunca rodam a mesma tarefa ao mesmo tempo (RN-90).
2. **Validar** — fontes existem, conta ativa, espaço livre (RN-11, RN-41, RN-46).
3. **Escanear local** (`scanner.rs`) — percorre as fontes aplicando filtros e `.syncignore`;
   registra o que foi excluído (RN-28) e o que não pôde ser lido (RN-27); só calcula hash
   de arquivos cujo tamanho/mtime mudou desde o índice; hashing em paralelo.
4. **Estado remoto** (`remote_tree.rs`) — hoje: listagem recursiva da pasta destino, uma
   pasta por vez (paralelizar é pendência); fase 4: Changes API incremental.
5. **Planejar** (`planner.rs`) — função pura: `(local, índice, remoto, modo) → Plano`.
   **Guardas** (`guard.rs`) — bloqueiam planos perigosos (RN-40 a RN-44).
6. **Confirmação** — na 1ª execução e quando a guarda pede, a execução volta com o código
   `needs_confirmation`; a UI pergunta ao usuário e a CLI exige `--confirm` (RN-36, RN-45).
7. **Executar** (`executor.rs`) — reserva de espaço na conta (RN-46); pastas primeiro
   (sequencial), depois transferências em paralelo (limite configurável), cada arquivo
   verificado por checksum e gravado no índice; progresso por evento e cancelamento (RN-95).
8. **Registrar** — status e estatísticas no histórico (RN-91, RN-92). Notificações: fase 3.

## 7. Fluxo do envio compactado

Quando a tarefa envia **compactado** (`Delivery::Archive`), `run_job` desvia para
`service/src/delivery.rs`. A trava, a validação e a confirmação são as mesmas da seção 6.

```
1. trava + validar ─► 2. varrer (sem hash) ─► 3. impressão digital ─► igual à última versão
                                                                      e ela está intacta? ─► sucesso, nada enviado
                                                                                │ não
4. guardas (fonte ausente/vazia, 1ª vez) ─► 5. espaço local ─► 6. compactar em tmp/ (zip ou tar.*)
                                                                                │
9. retenção: versões antigas registradas ─► lixeira ◄── 8. registrar versão ◄── 7. cota + enviar (checksum)
```

- **Impressão digital** (`archive::versions::fingerprint`, RN-59): MD5 de caminho, tamanho e
  data de cada entrada — barato, sem ler o conteúdo. Igual à última versão (e no mesmo
  formato) → não recompacta nem reenvia.
- **Guarda** (`guard::evaluate_archive`): fonte ausente (RN-40), fonte vazia com versões
  anteriores (RN-41), nada para compactar, primeira execução (RN-36).
- **Compactação** em `~/.local/share/syncly/tmp/<tarefa>/`, numa thread à parte, com
  progresso e cancelamento; o temporário é apagado ao sair (com sucesso ou erro).
- **Versões** ficam na tabela `archives` do `state.db` (tarefa, conta, pasta, ID remoto,
  nome, tamanho, hash, impressão digital). A retenção (`backup::retention`) só considera
  esses registros; antes de mandar uma versão para a lixeira, o app confere que ela ainda
  tem o nome e o tamanho registrados (RN-55).
- Uma versão é **um único arquivo**: uma sequência de requisições em vez de milhares — mais
  rápido e muito mais gentil com as cotas da API.

**Restaurar** (`service/src/restore.rs`, RN-57, RN-58): com a trava da tarefa, confere que a
versão está intacta, baixa para `tmp/`, compara o checksum com o registrado no envio e só
então descompacta numa pasta nova — pela mesma extração defensiva da tela Compactar
(SEG-26). Vai para o histórico como execução `restore`.

Planejado: receber (fase 2d — rotina de recebimento, baixar e descompactar, enviar pasta
compactada; RN-107 a RN-109) e criptografia `age` com manifesto (fase 2e).

## 8. Agendamento *(planejado — fase 3)*

- Cada agendamento vira um par `syncly-<id>.service/.timer` (um por tarefa, RN-81) em
  `~/.config/systemd/user/`, executando `syncly run <id-da-tarefa>`.
- O app faz **reconciliação**: a cada alteração da config, compara os timers desejados com
  os instalados (prefixo `syncly-`) e cria/atualiza/remove — idempotente.
- `Persistent=true` (roda após o boot se o horário passou), `RandomizedDelaySec=2min`,
  `Nice=10` e `IOSchedulingClass=idle` (não atrapalha o uso do computador).
- O agendador fica atrás de uma interface (`Scheduler`) para no futuro suportar
  launchd (macOS) e Agendador de Tarefas (Windows).

## 9. Fronteira Tauri ↔ Vue

- A UI só conhece **DTOs** (`syncly-service::dto` ↔ `ui/src/api/types.ts`). Nada de
  tokens, chaves ou objetos internos.
- **Allowlist de comandos**: cada comando é declarado em `src-tauri/build.rs` e liberado
  explicitamente em `capabilities/default.json`. Um comando novo não declarado não pode
  ser chamado pela webview.
- Plugins expostos à webview: apenas `dialog:allow-open` (escolher pastas/arquivos),
  `dialog:allow-ask` (confirmações nativas) e eventos.
  O navegador do login OAuth é aberto pelo Rust, não pela UI.
- CSP restritiva (`default-src 'self'`, sem `eval`, sem recursos externos).
- Progresso por eventos Tauri: `run-progress` (execução ou restauração de tarefa, com `jobId`) e
  `archive-progress` (compactar/descompactar). Ouvidos só em `ui/src/api/`.
- Operações longas (execução, compactação) podem ser canceladas: `cancel_run`, `cancel_archive`.
- Erros chegam à UI como `{ code, message }` com mensagem em português, sem detalhes internos.

## 10. Desempenho

| Técnica | Ganho |
|---|---|
| Hash só de arquivos alterados (tamanho + mtime contra o índice) | Re-sincronizar 100 mil arquivos sem mudança custa só um `stat` por arquivo. |
| Hashing paralelo (`rayon`) | Usa todos os núcleos no 1º scan. |
| "Adotar" arquivo remoto idêntico (mesmo MD5) | Reinstalar o app ou perder o índice não causa re-upload. |
| Transferências paralelas (padrão 4, máx. 8) + limitador de taxa | Throughput alto sem estourar cotas. |
| Upload simples (< 5 MiB) e resumable em blocos de 8 MiB | Menos requisições; arquivos grandes retomam de onde pararam. |
| `fields=` em toda chamada ao Drive | Respostas pequenas. |
| Um único `reqwest::Client` (HTTP/2, rustls) | Reuso de conexões TLS. |
| SQLite em WAL, escrita em lote por transação | Índice rápido mesmo com GUI e CLI abertos. |
| Streaming em tudo (upload, hash, tar, zstd, age) | Memória constante, independente do tamanho do arquivo. |
| zstd nível 3 multithread | Compressão rápida; dados já comprimidos passam quase sem custo. |
| Compactar sem hash | A compactação reaproveita a varredura, mas pula o cálculo de MD5. |
| Impressão digital das fontes no envio compactado | Execução sem mudanças custa só a varredura: nada é recompactado nem reenviado (RN-59). |
| ZIP sem recompressão de jpg/mp4/zip… | Arquivos já comprimidos vão "armazenados", sem gastar CPU. |
| Changes API (fase 4) | Detectar mudanças remotas sem listar tudo. |

> Pior caso do Drive: muitos arquivos pequenos (≈1 requisição por arquivo). Para pastas com
> dezenas de milhares de arquivos pequenos (ex.: projetos com dependências), prefira
> o **envio compactado** (seção 7) em vez de sincronização arquivo a arquivo.

## 11. Manutenção

- **Regras com número.** Toda regra de negócio tem um ID (`RN-xx`, `SEG-xx`). O código
  que a implementa cita o ID em comentário; o teste que a cobre também. Mudou a regra →
  atualize o documento no mesmo commit.
- **Versões centralizadas** em `[workspace.dependencies]`; lints em `[workspace.lints]`
  (`unsafe_code = "forbid"` nos crates do projeto).
- **Erros tipados** (`thiserror`) por camada; mensagens para o usuário em português.
- **Migrações**: `config.toml` tem `version`; `state.db` usa `PRAGMA user_version` com
  migrações sequenciais em `state.rs`.
- **Testes**: funções puras com testes unitários; `LocalProvider` + `tempfile` para testes
  de ponta a ponta sem rede; Drive com API simulada (`wiremock`) e cofre em memória
  (`secrets::MemoryStore`) — veja `crates/service/src/test_support.rs`. Nenhum teste usa a
  conta Google real, o keyring real ou o `HOME` real.
- **Checklist antes de commitar**: `cargo fmt`, `cargo clippy --workspace`, `cargo test --workspace`,
  `pnpm build`.
- **Adicionar um provedor**: novo crate `crates/<nome>` implementando `StorageProvider`,
  registrar na fábrica em `syncly-service`, adicionar variante em `AccountKind`. Se o
  provedor usa outro algoritmo de checksum (Dropbox `content_hash`, OneDrive `quickXorHash`),
  acrescente uma variante em `HashKind` — o scanner passa a calcular o mesmo localmente.
  Nada no `core` ou na UI precisa mudar além disso.

## 12. Decisões registradas

| Decisão | Alternativa descartada | Motivo |
|---|---|---|
| Tauri 2 (estável) + Rust | Electron; Python; Tauri 3 (alpha) | Binário pequeno, memória segura, núcleo reutilizado pela CLI; estabilidade. |
| Vue 3 + TypeScript + Pinia | — | Escolha do usuário; tipagem na fronteira com o Rust. |
| REST do Drive direto (`reqwest`) | crate `google-drive3` | Controle total de retry, `fields` e upload resumable; menos dependências. |
| Escopo `drive.file` | `drive` (acesso total) | Menor privilégio, escopo não sensível (sem verificação), bug não alcança outros arquivos. |
| Timers do systemd | Daemon próprio em segundo plano | Roda com o app fechado, sobrevive a reboot, zero código de agendador. |
| TOML (config) + SQLite (estado) | Tudo em JSON / tudo em SQLite | Config legível e portátil; estado transacional e rápido. |
| ZIP, TAR, TAR.GZ e TAR.ZST, à escolha (avulso e nas tarefas) | Um formato só | ZIP abre em qualquer sistema; TAR.GZ é o padrão do Linux; zstd é o mais rápido e compacto. |
| Backup = forma de envio da tarefa (compactado, com versões) | Tarefa com "sincronização" e "backup" separados, pasta `Backups/` reservada | Um conceito a menos e um agendamento por tarefa; local ou remoto vem da conta de destino. |
| Versões registradas no `state.db` | Reconhecer versões pelo padrão do nome | A retenção nunca toca um arquivo que o app não criou, mesmo com nome parecido (RN-55). |
| `age` para criptografia | GPG; AES "caseiro" | Formato moderno e auditado, streaming, chave pública permite backup automático sem segredo em memória. |
| `rustls` | OpenSSL | Sem dependência nativa, implementação memory-safe. |
| `reqwest` 0.13 (HTTP/2, `rustls`) | crate `google-drive3` | Já citado acima; HTTP/2 reaproveita a conexão entre requisições. |
| `keyring` 4 (Secret Service via `zbus`) | Arquivo cifrado próprio | Cofre do sistema (GNOME Keyring/KWallet), Rust puro, sem libdbus. |
| `wiremock` nos testes | Testar contra a conta real | Testes do Drive rodam offline, sem credenciais (regra 9 do `AGENTS.md`). |
| `zip` só com deflate (Rust puro) + `tar`/`flate2`/`zstd` | Bibliotecas com todos os compressores (bzip2, lzma em C) | Menos código nativo; cobre os formatos comuns. Extração sempre manual, nunca `unpack()` das bibliotecas (SEG-26). |
| `fs4` para espaço livre | Ignorar espaço em disco | RN-67: bomba de compressão e disco cheio. |
| Varredura própria (`std::fs`) + `ignore::gitignore` só para padrões | Walker do crate `ignore` | Controle total de exclusões, `.syncignore` e erros — necessário para RN-27/RN-28. |
| pnpm | npm, yarn | Instalação rápida e `node_modules` estrito (sem dependências fantasmas). |
| Sem exclusão definitiva | — | Proteção contra bugs e erros humanos (lixeira do Drive guarda 30 dias). |

## 13. Roadmap

| Fase | Entrega | Status |
|---|---|---|
| 0 | Esqueleto: workspace, domínio, validações, filtros, planejador, guardas, retenção, estado, `LocalProvider`, logs, prévia de sincronização, UI base | ✅ |
| 1a | Login Google (PKCE + keyring), provedor Drive, transferências avulsas (enviar/baixar) | ✅ |
| 1b | Executor, sync manual (modo envio/espelho) com progresso e cancelamento | ✅ |
| 1c | Status e última modificação (RN-100 a RN-103) | |
| 2a | Compactar e descompactar (zip, tar, tar.gz, tar.zst) | ✅ |
| 2b | Envio compactado: versões com data, retenção, backup local/remoto | ✅ |
| 2c | Restaurar versão (baixar, conferir e descompactar numa pasta nova) | ✅ |
| 2d | Receber: rotina de recebimento, baixar e descompactar, enviar pasta compactada (RN-107 a RN-109) | |
| 2e | Criptografia (age) e manifesto das versões | |
| 3 | Agendamento (systemd), notificações, bandeja | |
| 4 | Changes API, download/bidirecional (requer decisão sobre escopo — ver SEG-03) | |
| 5 | Outros provedores (OneDrive, Dropbox, S3), macOS/Windows | |

Limitações conhecidas:

- O `LocalProvider` calcula o hash de cada arquivo do destino a cada listagem — lento para
  destinos locais grandes na sincronização arquivo por arquivo (o envio compactado não
  lista conteúdo: usa só os nomes). Um cache persistente fica para uma fase futura.
- A listagem do destino remoto é feita uma pasta por vez.
- Na tela Arquivos ainda não há botão para criar pasta (a CLI `remote put` cria as que faltam).
