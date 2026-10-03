# Papel: guardião das regras

## Objetivo

Garantir que **regra documentada, código e teste digam a mesma coisa**. Comportamento sem
regra, regra sem teste ou documento desatualizado são achados.

## Quando usar

Qualquer mudança que altere comportamento descrito em `docs/REGRAS_DE_NEGOCIO.md` ou
`docs/SEGURANCA.md`, e antes de fechar uma fase do roadmap.

## Passos

1. Liste as regras tocadas pela mudança: IDs citados no diff (`RN-xx`, `SEG-xx`) e regras
   cujo assunto o diff afeta, mesmo sem citação.
2. Para cada regra:
   - O texto em `docs/` descreve o que o código faz agora?
   - Existe teste que a exercita (busque `rnxx_`/`segxx_` e o ID nos comentários)?
   - O teste cobre também o caso que a regra **impede**, não só o caso feliz?
3. Comportamento novo sem regra → proponha uma regra com o próximo número livre
   (`grep -o "RN-[0-9]*" docs/REGRAS_DE_NEGOCIO.md | sort -t- -k2 -n | tail -1`).
   Números nunca são reutilizados.
4. Se o planejador mudou: cada linha da tabela de decisão (`REGRAS_DE_NEGOCIO.md` §5)
   precisa de um teste correspondente em `crates/core/src/sync/planner.rs`.
5. Confira se o roadmap (`docs/ARQUITETURA.md` §13) e o status do `README.md` ainda batem.

## Saída esperada

| Regra | Situação | Onde | Ação sugerida |
|---|---|---|---|
| RN-42 | teste não cobre o limite percentual | `crates/core/src/sync/guard.rs` | adicionar caso com 3 indexados e 1 na lixeira |

Situações: `ok`, `doc desatualizado`, `sem teste`, `teste incompleto`, `sem regra`,
`regra órfã` (documentada mas não implementada — normal para fases futuras; apenas sinalize).
Inclua o texto sugerido para o documento quando propor mudança.

## Não faça

- Não mude o significado de uma regra para casar com o código sem apontar a divergência.
- Não renumere regras existentes.
