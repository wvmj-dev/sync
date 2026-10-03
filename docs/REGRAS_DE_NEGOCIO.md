# Regras de negócio

Cada regra tem um identificador (`RN-xx`). O código que a implementa e o teste que a cobre
citam esse identificador. Ao mudar uma regra, atualize este documento no mesmo commit.

## 1. Glossário

| Termo | Significado |
|---|---|
| **Conta** | Um destino de armazenamento autenticado: uma conta Google Drive ou uma pasta local (HD externo, NAS montado). |
| **Pasta raiz do app** | Pasta `Syncly/` criada pelo app na conta. Tudo que o app grava fica dentro dela. |
| **Tarefa** | Conjunto de **fontes** (pastas e/ou arquivos) + o que fazer com elas: **sincronizar** e/ou **fazer backup**, cada um com seu agendamento. |
| **Fonte** | Caminho absoluto local de uma pasta ou arquivo incluído na tarefa. |
| **Índice** | Último estado sincronizado de cada arquivo (tamanho, mtime, MD5 local, ID e MD5 remotos). |
| **Plano** | Lista de ações calculada antes de executar (enviar, atualizar, mover para lixeira…). |
| **Execução** | Uma rodada de sincronização, backup, restauração ou compactação, com status e estatísticas. |
| **Backup** | Arquivo compactado (`.tar.zst`, opcionalmente `.age`) representando as fontes num momento. |

## 2. Contas

- **RN-01** — Contas Google são conectadas exclusivamente via OAuth no navegador do sistema. O app nunca pede, vê ou guarda a senha.
- **RN-02** — Ao conectar uma conta Google, o app procura sua pasta raiz pela marca privada `appProperties.syncly = "root"`; se não existir, cria `Syncly/` com essa marca.
- **RN-03** — O app só opera dentro da pasta raiz. Caminhos de destino são sempre relativos a ela.
- **RN-04** — Várias contas podem coexistir; cada uma com seus próprios tokens no keyring.
- **RN-05** — Desconectar uma conta revoga o token no Google, apaga-o do keyring e **desativa** (não apaga) as tarefas que a usavam.
- **RN-06** — Se o Google recusar a renovação do token (`invalid_grant`), a conta fica com status **requer reautenticação**; todas as execuções dela são abortadas sem novas tentativas até o usuário reconectar.
- **RN-07** — Conta local: uma pasta raiz absoluta, existente. O app grava nela um marcador `.syncly-root` e trata a pasta como trata a raiz do Drive (mesmas regras de lixeira e de caminhos). **Sem o marcador, nada é gravado** — protege contra HD desmontado, quando só resta o ponto de montagem vazio.

## 3. Tarefas

- **RN-10** — Nome obrigatório, 1 a 64 caracteres, sem caracteres de controle, único na configuração (sem diferenciar maiúsculas).
- **RN-11** — Fontes: de 1 a 32 caminhos **absolutos**, sem `..`, únicos na tarefa.
- **RN-12** — Fontes não podem ser pastas de sistema (`/`, `/proc`, `/sys`, `/dev`, `/run`) nem estar dentro das pastas de dados/config do próprio app. Se uma fonte **contém** as pastas do app (ex.: a pasta pessoal inteira), elas são puladas automaticamente.
- **RN-13** — Uma fonte não pode estar dentro de outra fonte da mesma tarefa.
- **RN-14** — O **nome final** (basename) de cada fonte deve ser único na tarefa, pois vira a pasta/arquivo de primeiro nível no destino. Ex.: `~/Documentos` e `~/notas.md` → `Syncly/<destino>/Documentos/…` e `Syncly/<destino>/notas.md`.
- **RN-15** — Toda tarefa precisa de pelo menos uma ação: sincronização e/ou backup.
- **RN-16** — Destino de sincronização: conta existente + caminho relativo à pasta raiz, com 1 a 32 componentes, sem `.`/`..`, sem caracteres de controle, até 255 bytes por componente. A pasta de primeiro nível `Backups` é reservada (RN-51).
- **RN-17** — Duas tarefas não podem ter destinos sobrepostos na mesma conta (iguais ou um dentro do outro) — cada pasta remota tem um único "dono".
- **RN-18** — Tarefa desativada não roda (nem manual nem agendada) e seus timers são removidos.

## 4. Fontes e filtros

- **RN-20** — Exclusões usam sintaxe do `.gitignore`. Padrões **sem** `/` valem em qualquer nível; padrões **com** `/` são relativos à raiz de cada fonte.
- **RN-21** — Exclusões padrão (sempre ativas): lixo de sistema e temporários — `.DS_Store`, `Thumbs.db`, `desktop.ini`, `*.tmp`, `*.swp`, `*~`, `~$*`, `.~lock.*#`, `.Trash-*/`, `.syncly-*`.
- **RN-22** — **Arquivos sensíveis são excluídos por padrão**: `.ssh/`, `.gnupg/`, `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`, `*.kdbx`, `.netrc`, `.pgpass`. Só são incluídos se a tarefa marcar explicitamente `include_sensitive = true`. Vale também para a **própria fonte**: escolher `~/.ssh` ou um `.env` como fonte exige essa marcação, e um `.syncignore` não consegue reincluir arquivos sensíveis.
- **RN-23** — Cada pasta pode ter um arquivo `.syncignore` com exclusões adicionais (mesma sintaxe). As exclusões da tarefa têm precedência; entre `.syncignore`, o mais profundo vence (inclusive `!padrão` para reincluir).
- **RN-24** — Links simbólicos **não são seguidos** nem enviados; são ignorados com aviso. Arquivos especiais (sockets, FIFOs, dispositivos) também.
- **RN-25** — Um arquivo cujo tamanho ou mtime mudou durante o envio é marcado como **instável** e reenviado na próxima execução.
- **RN-26** — O hash (no algoritmo que o provedor informa — MD5 no Drive) só é recalculado quando tamanho ou mtime diferem do índice.
- **RN-27** — Arquivos e pastas que não puderam ser lidos (permissão, erro de E/S, `.syncignore` inválido) ficam **bloqueados** naquela execução: nada abaixo deles é enviado, atualizado ou posto na lixeira. Um erro de leitura nunca vira "arquivo apagado".
- **RN-28** — Itens excluídos pelos filtros **nunca vão para a lixeira** do destino, nem no modo Espelho: o app apenas deixa de acompanhá-los (o que já foi enviado permanece lá). Adicionar `node_modules/` às exclusões não apaga nada.

## 5. Sincronização

### Modos

| Modo | Descrição | Fase |
|---|---|---|
| **Envio** (`upload`, padrão) | Local → nuvem. Novos e alterados são enviados. Exclusões locais **não** são propagadas. O mais seguro: funciona como "cópia de segurança viva". | 1 |
| **Espelho** (`mirror`) | Local → nuvem. Exclusões locais vão para a **lixeira** do destino, sob as guardas da seção 6. | 1 |
| **Download** / **Bidirecional** | Nuvem → local / ambos. Exigem decisão sobre escopo (SEG-03). | 4 |

### Tabela de decisão (modos Envio e Espelho), por arquivo

| Local | Índice | Remoto | Situação | Envio | Espelho |
|---|---|---|---|---|---|
| existe | — | — | novo | enviar | enviar |
| existe | existe | — | apagado no destino por fora | reenviar | reenviar |
| existe | * | MD5 igual ao local | já sincronizado | só atualizar índice ("adotar") | idem |
| existe | remoto = índice, local ≠ índice | MD5 diferente | alterado localmente | atualizar | atualizar |
| existe | ausente, ou remoto ≠ índice | MD5 diferente | **conflito** | manter ambos (RN-35) | manter ambos |
| — | existe | MD5 = índice | apagado localmente | esquecer do índice | mover para lixeira |
| — | existe | MD5 ≠ índice | apagado local + alterado remoto | esquecer do índice | esquecer do índice + aviso |
| — | — | existe | desconhecido | **nunca tocar** | **nunca tocar** |
| — | existe | — | apagado nos dois lados | limpar índice | limpar índice |

- **RN-30** — O planejador é uma função pura `(local, índice, remoto, modo) → plano` e é a **única** fonte de decisões.
- **RN-31** — **O app nunca altera nem apaga um item remoto que ele não sabe que sincronizou** (sem entrada no índice).
- **RN-32** — Pastas: criadas antes dos arquivos (ordem de profundidade). No modo Espelho, uma pasta só vai para a lixeira se **todo** o seu conteúdo remoto também for para a lixeira no mesmo plano.
- **RN-33** — Tipos divergentes (arquivo local × pasta remota com o mesmo nome, ou o inverso) → item ignorado com aviso.
- **RN-34** — Itens remotos sem MD5 (ex.: Google Docs nativos), com nome inválido localmente (contém `/`, `.` ou `..`) ou **duplicados** na mesma pasta → ignorados com aviso. O app nunca "adivinha".
- **RN-35** — Conflito: a versão remota é **renomeada** para `<nome> (conflito remoto AAAA-MM-DD HH-MM).<ext>` (se o nome já existir: `… 14-30 2).<ext>`, `… 14-30 3).<ext>`…) e a versão local é enviada com o nome original. Nenhuma versão se perde.
- **RN-36** — A primeira execução de toda tarefa é **sempre** uma prévia com confirmação. Agendamentos só ficam ativos após a primeira execução manual bem-sucedida.
- **RN-37** — Cada envio é verificado: MD5 calculado durante o upload deve bater com o `md5Checksum` devolvido pelo Drive. Divergência → nova tentativa; persistindo, item falha.
- **RN-38** — O índice é gravado imediatamente após cada item confirmado. Execução interrompida é retomada sem duplicar nada.
- **RN-39** — Ordem de execução: criar pastas → enviar/atualizar/conflitos (em paralelo) → lixeira de arquivos → lixeira de pastas (mais profundas primeiro).

## 6. Proteções (guardas)

- **RN-40** — Fonte inexistente ou ilegível → execução **abortada**. A tarefa é revalidada antes de cada execução (o `config.toml` pode ter sido editado à mão).
- **RN-41** — Fonte **vazia** enquanto o índice tem arquivos dela → execução **abortada** (protege contra HD externo desmontado: o ponto de montagem existe, mas vazio).
- **RN-42** — Plano que move para a lixeira mais de **50 arquivos** ou mais de **25 %** dos arquivos indexados exige **confirmação explícita**. Valores configuráveis por tarefa (1–100 %).
- **RN-43** — Execuções agendadas **nunca** confirmam sozinhas: se a guarda pedir confirmação, a execução fica *aguardando confirmação* e o usuário é notificado.
- **RN-44** — Não existe exclusão definitiva. Remoto: lixeira do Drive (recuperável por 30 dias). Local (conta local e restaurações): pasta `.syncly-trash/` ou lixeira do sistema.
- **RN-45** — Na CLI, planos que exigem confirmação só rodam com `--confirm`.
- **RN-46** — **Reserva de espaço**: antes de enviar, o app consulta a cota. Se o espaço livre após o envio ficar abaixo de **1 GiB** (configurável), a execução é abortada — conta Google cheia para de receber e-mails no Gmail.

## 7. Backup

- **RN-50** — Backup = arquivo único `<tarefa>_<AAAAMMDD-HHMMSS>.tar.zst` (+ `.age` se criptografado), contendo as fontes (com os mesmos filtros da tarefa) e um `MANIFEST.json` (caminho, tamanho, mtime, sha256 de cada arquivo).
- **RN-51** — Destinos: pasta local (absoluta, **fora** das fontes — senão o backup se incluiria) e/ou remoto em `Syncly/Backups/<tarefa>/`. Pelo menos um.
- **RN-52** — O arquivo é escrito como `.partial`, verificado (sha256) e só então renomeado. Backup incompleto nunca aparece como válido.
- **RN-53** — Antes de começar, o app checa espaço local (estimativa = tamanho total das fontes) e cota remota (RN-46).
- **RN-54** — Retenção (padrão): últimos **3** + o mais recente de cada um dos **7** dias mais recentes que tiverem backup + idem para **4** semanas e **6** meses. Mínimo de 1 "últimos".
- **RN-55** — A retenção só roda **depois** de um backup bem-sucedido e verificado, e nunca remove o backup mais recente. Remoções vão para a lixeira.
- **RN-56** — Recuperação: o índice pode ser reconstruído a partir do destino (MD5 + `appProperties`), sem reenviar nada (adoção, tabela da seção 5).

### Restauração

- **RN-57** — Restauração vai, por padrão, para uma **pasta nova** (`restauracao-<tarefa>-<data>/`). Restaurar por cima exige confirmação, e os arquivos substituídos vão para a lixeira do sistema.
- **RN-58** — Antes de extrair, o sha256 do arquivo é verificado. Entradas com caminho absoluto, `..`, links para fora do destino ou arquivos especiais são rejeitadas.

## 8. Compactação

- **RN-60** — Ação avulsa "Compactar": entradas (pastas/arquivos) → arquivo `.tar.zst` (padrão) ou `.zip` (compatibilidade).
- **RN-61** — Nível zstd 1–19 (padrão 3). No `.zip`, arquivos já comprimidos (jpg, png, mp4, zip, gz, 7z…) são armazenados sem recompressão.
- **RN-62** — O arquivo de saída não pode estar dentro das entradas e **nunca sobrescreve** um existente (recebe sufixo ` (2)`).
- **RN-63** — Criptografia só para `.tar.zst` (gera `.tar.zst.age`).

## 9. Criptografia

- **RN-70** — Opcional, por tarefa, aplicada a backups e compactações. A sincronização envia arquivos como estão (para serem visualizáveis no Drive).
- **RN-71** — Formato `age` com chave X25519. A chave pública fica no `config.toml`; a privada, no keyring.
- **RN-72** — Ativar criptografia exige **exportar a chave de recuperação** e confirmar que ela foi guardada. Sem ela, os backups são irrecuperáveis — o app avisa isso explicitamente.
- **RN-73** — Backups automáticos usam só a chave pública; a privada só é lida na restauração.
- **RN-74** — Cada backup registra a impressão digital da chave usada; trocar a chave não invalida backups antigos.

## 10. Agendamento

- **RN-80** — Opções: manual; a cada N horas (N ∈ {1, 2, 3, 4, 6, 8, 12}); diário às HH:MM; semanal (dia + HH:MM); avançado (expressão `OnCalendar` do systemd).
- **RN-81** — Sincronização e backup de uma tarefa têm agendamentos independentes.
- **RN-82** — Expressão avançada: até 64 caracteres, apenas `[A-Za-z0-9 *-:/,.~]`, validada por `systemd-analyze calendar` antes de salvar.
- **RN-83** — Execução perdida (computador desligado) roda no próximo boot/login.
- **RN-84** — Se o keyring estiver bloqueado ou não houver rede, a execução fica **adiada**; notifica após 3 adiamentos seguidos.
- **RN-85** — Conta em *requer reautenticação* suspende os agendamentos dela.

## 11. Execuções, histórico e notificações

- **RN-90** — No máximo uma execução por tarefa ao mesmo tempo (trava exclusiva); um disparo concorrente é descartado e registrado.
- **RN-91** — Status possíveis: `sucesso`, `sucesso_parcial` (alguns itens falharam), `falha`, `abortada` (guarda, espaço, fonte), `aguardando_confirmacao`, `cancelada`, `adiada`.
- **RN-92** — Estatísticas por execução: itens e bytes enviados/atualizados/na lixeira/ignorados/com falha, duração.
- **RN-93** — Histórico mantido por 180 dias ou 1000 execuções por tarefa (o que vier primeiro).
- **RN-94** — Notifica em `falha`, `abortada`, `aguardando_confirmacao` e adiamentos repetidos. Sucesso é silencioso (configurável).
- **RN-95** — Cancelar uma execução interrompe as transferências em andamento; o que já foi confirmado permanece no índice (RN-38).

### Tratamento de erros

- **RN-96** — Erros transitórios (rede, 408, 429, 5xx, cotas de taxa) → nova tentativa com backoff exponencial e jitter (1 s → 64 s, até 6 tentativas), respeitando `Retry-After`.
- **RN-97** — Erros permanentes de item (404, 403 de permissão, 400) → o item falha, os demais seguem.
- **RN-98** — Erros que afetam a conta inteira → a execução é **abortada** imediatamente: autenticação (`401`, RN-06), armazenamento cheio (`storageQuotaExceeded`, `activeItemCreationLimitExceeded`), cota diária do projeto (`dailyLimitExceeded`).
- **RN-99** — Limite de taxa persistente: se **3 itens seguidos** esgotarem as tentativas por `rateLimitExceeded`/`userRateLimitExceeded`/`429`, a execução é abortada como **adiada**. É assim que o limite de **750 GB/dia** de upload do Google se manifesta — o app para em vez de insistir.

## 12. Status e última modificação

- **RN-100** — Para cada tarefa: última execução (data, status, estatísticas), próxima execução agendada, total de arquivos e bytes.
- **RN-101** — Para cada arquivo ou pasta de uma tarefa: data de modificação local; data de modificação e **autor da última modificação** no destino; data do último sync; situação — `sincronizado`, `alterado localmente`, `alterado no destino`, `pendente`, `conflito`, `ignorado`.
- **RN-102** — Para pastas, a "última modificação" é a mais recente entre todos os arquivos dentro dela.
- **RN-103** — A consulta de status usa o índice (instantânea, funciona offline); "Atualizar do destino" busca os dados atuais na nuvem.
