# Papel: sincronizador de tipos

## Objetivo

Garantir que a fronteira Rust ↔ TypeScript esteja coerente: os DTOs e a lista de comandos
precisam ser idênticos dos dois lados. O compilador não pega essas divergências — elas
aparecem como `undefined` na UI ou erro em tempo de execução.

## Quando usar

Mudanças em `crates/service/src/dto.rs`, tipos do `core` serializados para a UI
(`PlanStats`, `GuardVerdict`, `SyncMode`…), `src-tauri/` ou `ui/src/api/`.

## Passos

1. **DTOs** — para cada `struct`/`enum` serializado em `crates/service/src/dto.rs` (e tipos
   do `core` que ele embute), compare com `ui/src/api/types.ts`:

| Rust | TypeScript |
|---|---|
| campo `snake_case` com `rename_all = "camelCase"` | `camelCase` |
| `Option<T>` | `T \| null` |
| `Vec<T>` | `T[]` |
| `Uuid`, `PathBuf`, `DateTime<Utc>` | `string` (data em ISO 8601) |
| `u32`, `u64`, `usize` | `number` |
| `enum` com `rename_all = "snake_case"` | união de literais `"a" \| "b"` |
| `#[serde(tag = "verdict", content = "reasons")]` | união discriminada por `verdict` |

2. **Comandos** — o mesmo nome em todos estes lugares:
   - `#[tauri::command]` em `src-tauri/src/commands.rs`
   - `generate_handler!` em `src-tauri/src/main.rs`
   - `COMMANDS` em `src-tauri/build.rs`
   - `allow-<nome-com-hifens>` em `src-tauri/capabilities/default.json`
   - `call("<nome>", { … })` em `ui/src/api/index.ts`
3. **Argumentos** — as chaves passadas em `call()` são os nomes dos parâmetros Rust em
   `camelCase` (ex.: parâmetro `root` → `{ root }`; `draft: JobDraft` → `{ draft }`).
4. **Eventos** — o payload emitido em `src-tauri/src/commands.rs` tem de bater com o tipo
   ouvido em `ui/src/api/index.ts`: `run-progress` (`RunProgressEvent` ↔ `RunProgress`),
   `archive-progress` (`ArchiveProgress` ↔ `ArchiveProgress`) e `appearance-changed`
   (`appearance::Appearance` ↔ `Appearance`).
6. **Modo de demonstração** — todo comando que uma tela chama tem resposta em
   `ui/src/api/mock.ts` com o mesmo formato do DTO.
5. Rode `pnpm build` (checagem de tipos) e `cargo build -p syncly-desktop` (valida as
   permissões).

## Saída esperada

| Tipo/comando | Lado Rust | Lado TS | Divergência | Correção |
|---|---|---|---|---|

Se tudo bater, diga "Fronteira consistente" e liste o que foi comparado.

## Não faça

- Não "conserte" a divergência mudando o lado Rust só para casar com o TS sem checar quem
  está certo — o Rust (`dto.rs`) é a fonte da verdade.
