# Receitas

Passo a passo das mudanças mais comuns. Siga a ordem: ela evita esquecer uma das pontas.

## 1. Novo comando chamado pela UI

1. **Caso de uso** em `crates/service/src/service.rs` (método de `Service`), com teste.
2. **DTOs** de entrada/saída em `crates/service/src/dto.rs` (`#[serde(rename_all = "camelCase")]`).
   Nada de segredo ou tipo interno do `core` que vaze detalhes (SEG-33).
3. **Comando** em `src-tauri/src/commands.rs`: `async`, retorna `ApiResult<T>`, só converte
   e delega.
4. Registre em `generate_handler!` (`src-tauri/src/main.rs`).
5. Adicione o nome em `COMMANDS` (`src-tauri/build.rs`).
6. Libere `allow-<nome-com-hifens>` em `src-tauri/capabilities/default.json`.
7. **Tipos** em `ui/src/api/types.ts` e função em `ui/src/api/index.ts`.
8. `cargo build -p syncly-desktop` — o build valida as permissões — e `pnpm build`.

## 2. Nova regra de negócio

1. **Documente primeiro** em `docs/REGRAS_DE_NEGOCIO.md` (ou `SEGURANCA.md`) com o próximo
   número livre:
   `grep -o "RN-[0-9]*" docs/REGRAS_DE_NEGOCIO.md | sort -t- -k2 -n | tail -1`
2. Implemente como **função pura** no `crates/core` (sem I/O), com comentário `// RN-xx`.
3. Teste unitário `fn rnxx_<descricao>()` cobrindo o caso feliz e o caso que a regra impede.
4. Ligue a função ao fluxo (service/executor). I/O fica nas bordas.
5. Se a regra muda o que a UI mostra, siga a receita 1 para os DTOs.

## 3. Novo provedor (OneDrive, Dropbox, S3…)

1. Crie `crates/<nome>` dependendo só de `syncly-core`; adicione ao `[workspace]`.
2. Implemente `StorageProvider` (`crates/core/src/provider.rs`). **Não** existe método de
   exclusão definitiva — e não deve ser criado (SEG-10).
3. Se o checksum do serviço não for MD5, adicione a variante em `HashKind`
   (`crates/core/src/hash.rs`) e o cálculo correspondente.
4. Classifique os erros do serviço em `ProviderError` (veja `crates/gdrive/src/error.rs`),
   com testes para cada escopo (transitório, item, conta).
5. Adicione a variante em `AccountKind` (`crates/core/src/domain/account.rs`) e registre na
   fábrica `provider_for` (`crates/service/src/service.rs`).
6. Documente escopos/permissões pedidos em `docs/SEGURANCA.md` e a decisão em
   `docs/ARQUITETURA.md` §12.

## 4. Nova tela

1. Componente em `ui/src/views/<Nome>View.vue` (`<script setup lang="ts">`).
2. Rota em `ui/src/router.ts` e link em `ui/src/App.vue`.
3. Dados via `useAppStore()` ou `api.*` — nunca `invoke` direto.
4. Texto sempre por interpolação (`{{ }}`); `v-html` é proibido (SEG-32).
5. Cores e espaçamentos pelos tokens de `ui/src/styles/main.css` (tema claro/escuro).

## 5. Testar manualmente sem tocar nos seus dados

```bash
export HOME=$(mktemp -d)
mkdir -p "$HOME/Docs" "$HOME/hd" && echo oi > "$HOME/Docs/a.txt"
touch "$HOME/hd/.syncly-root"            # marcador da conta local (RN-07)
cargo run -p syncly-cli -- info
```

Depois crie `$HOME/.config/syncly/config.toml` com uma `[[account]]` local apontando para
`$HOME/hd` e uma `[[job]]` (formato em `crates/core/src/config.rs`, teste `sample()`), e rode
`cargo run -p syncly-cli -- preview <tarefa>`. Nunca use o `HOME` real nem a conta Google real.

## 6. Fechar uma fase do roadmap

1. Marque a fase como ✅ em `docs/ARQUITETURA.md` §13.
2. Atualize a seção **Estado atual** do `AGENTS.md` e o status no `README.md`.
3. Rode o papel `.agents/papeis/guardiao-regras.md` para conferir regras ↔ testes.
4. Rode o papel `.agents/papeis/revisor-seguranca.md` sobre tudo o que mudou na fase.

## 7. Nova dependência

1. Rust: declare em `[workspace.dependencies]` do `Cargo.toml` raiz e use `.workspace = true`.
2. Prefira crates mantidas, sem `unsafe` desnecessário e com `rustls` em vez de OpenSSL.
3. Rode `cargo audit` (ou `cargo deny check`) e `pnpm audit` (SEG-51).
4. Registre o motivo no commit; se for decisão estrutural, em `docs/ARQUITETURA.md` §12.
