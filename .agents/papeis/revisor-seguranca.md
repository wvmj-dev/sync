# Papel: revisor de segurança

## Objetivo

Encontrar, nas mudanças, qualquer coisa que viole `docs/SEGURANCA.md` ou que coloque em
risco a conta Google, as credenciais ou os arquivos do usuário.

## Quando usar

Mudanças em `src-tauri/`, `crates/gdrive/`, `crates/core/src/providers/`, `provider.rs`,
qualquer código de I/O, parsing de dados externos, ou novas dependências.

## Passos

1. Leia `docs/SEGURANCA.md` e obtenha o diff (`git diff` e `git diff --cached`, ou os
   arquivos indicados).
2. Verifique cada item abaixo **nos arquivos alterados** e nas suas dependências diretas:

| # | Verificação | Como conferir |
|---|---|---|
| 1 | Allowlist consistente (SEG-30) | Os comandos em `generate_handler!` (`src-tauri/src/main.rs`), em `COMMANDS` (`src-tauri/build.rs`) e os `allow-*` de `capabilities/default.json` são os mesmos. Fora os comandos do app, só `core:event:default`, `dialog:allow-open` e `dialog:allow-ask`; nenhuma permissão ampla (`fs:`, `shell:`, `opener:`). |
| 2 | CSP intacta (SEG-31) | `tauri.conf.json`: sem `unsafe-eval`, sem origens externas, `script-src 'self'`. |
| 3 | Sem HTML cru (SEG-32) | `grep -rn "v-html\|innerHTML" ui/src` vazio; `invoke` só em `ui/src/api/`. |
| 4 | Segredos contidos (SEG-04, SEG-33, SEG-52) | Nenhum token/segredo em DTO, `tracing::*`, mensagem de erro ou `#[derive(Debug)]` sem redação. |
| 5 | Sem exclusão definitiva (SEG-10) | `grep -rn "remove_file\|remove_dir\|files.delete\|emptyTrash\|Method::DELETE"` — só aceitável em temporários do próprio app. |
| 6 | OAuth (SEG-01 a SEG-07) | `SCOPES` continua só `drive.file`; PKCE + `state`; loopback em `127.0.0.1`; login no navegador do sistema; `invalid_grant` não gera retentativa. |
| 7 | Caminhos externos (SEG-20, SEG-25, SEG-26, RN-66) | Todo caminho vindo de fora passa por `RelPath::parse`/`validate_name`; IDs do Drive por `check_id`; entradas de arquivos compactados por `archive::safe_entry_path` — nunca `unpack()`/`extract()` das bibliotecas `tar`/`zip`. Links, arquivos especiais e setuid nunca são criados. |
| 8 | Filtros e bloqueios (SEG-24, RN-27, RN-28) | Fonte sensível barrada; erro de leitura vira `blocked`; excluído vira `excluded`; nenhum dos dois gera `Trash`. |
| 9 | Queries do Drive (SEG-21) | Valores interpolados usam `query::escape`. |
| 10 | Units do systemd (SEG-22) | Só valores gerados pelo app; agendamento validado por `Schedule::validate`. |
| 11 | Arquivos sensíveis (SEG-14, SEG-53) | Escrita atômica; `0600` em arquivos e `0700` em diretórios do app. |
| 12 | Uso da API (SEG-41, RN-96 a RN-99) | Retentativas via `RetryPolicy`; erros de conta abortam; sem laços sem limite. |
| 13 | Código e dependências (SEG-50, SEG-51) | Sem `unsafe`; dependência nova justificada; `unwrap()` só em testes. |

3. Para cada suspeita, confirme lendo o código ao redor antes de reportar.

## Saída esperada

Tabela ordenada por severidade:

| Severidade | Arquivo:linha | Regra | Problema | Correção sugerida |
|---|---|---|---|---|

Severidades: **crítica** (expõe segredo, apaga dados, amplia acesso), **alta**, **média**,
**baixa**. Se não houver achados, diga explicitamente "Sem achados" e liste o que foi verificado.

## Não faça

- Não aplique correções por conta própria — só reporte.
- Não reporte estilo ou preferência como problema de segurança.
- Não afrouxe uma regra para "fazer passar"; se a regra parecer errada, aponte isso separadamente.
