# Regras de negócio

Cada regra tem um identificador (`RN-xx`). O código que a implementa e o teste que a cobre
citam esse identificador. Ao mudar uma regra, atualize este documento no mesmo commit.

Este documento descreve o projeto **completo**. Seções marcadas como *(planejado)* ainda não
estão implementadas — o estado de cada fase está em [ARQUITETURA.md §13](ARQUITETURA.md#13-roadmap).

## 1. Glossário

| Termo | Significado |
|---|---|
| **Conta** | Um destino de armazenamento autenticado: uma conta Google Drive ou uma pasta local (HD externo, NAS montado). |
| **Pasta raiz do app** | Pasta `Syncly/` criada pelo app na conta. Tudo que o app grava fica dentro dela. |
| **Tarefa** | Uma rotina com um agendamento. Envia **fontes** (pastas e/ou arquivos) a um **destino** — arquivo por arquivo (sincronização) ou compactado (versões) — ou **recebe** de lá a versão mais recente. |
| **Fonte** | Caminho absoluto local de uma pasta ou arquivo incluído na tarefa. |
| **Índice** | Último estado sincronizado de cada arquivo (tamanho, mtime, MD5 local, ID e MD5 remotos). |
| **Plano** | Lista de ações calculada antes de executar (enviar, atualizar, mover para lixeira…). |
| **Execução** | Uma rodada de uma tarefa (sincronização ou envio compactado), restauração ou compactação, com status e estatísticas. |
| **Versão** | Arquivo compactado (`.zip`, `.tar.gz`, `.tar.zst` ou `.tar`) com as fontes de uma tarefa num momento — o **backup**. Local ou remoto conforme a conta de destino. |

## 2. Contas

- **RN-01** — Contas Google são conectadas exclusivamente via OAuth no navegador do sistema. O app nunca pede, vê ou guarda a senha.
- **RN-02** — Ao conectar uma conta Google, o app procura sua pasta raiz pela marca privada `appProperties.syncly = "root"`; se não existir, cria `Syncly/` com essa marca.
- **RN-03** — O app só opera dentro da pasta raiz. Caminhos de destino são sempre relativos a ela.
- **RN-04** — Várias contas podem coexistir; cada uma com seus próprios tokens no keyring.
- **RN-05** — Desconectar uma conta revoga o token no Google, apaga-o do keyring e **desativa** (não apaga) as tarefas que a usavam.
- **RN-06** — Se o Google recusar a renovação do token (`invalid_grant`), a conta fica com status **requer reautenticação**; todas as execuções dela são abortadas sem novas tentativas até o usuário reconectar. Conectar de novo com o **mesmo e-mail** atualiza a conta existente — as tarefas continuam valendo. *(Hoje a operação falha com "reconecte a conta"; guardar esse status na conta é planejado.)*
- **RN-07** — Conta local: uma pasta raiz absoluta, existente. O app grava nela um marcador `.syncly-root` e trata a pasta como trata a raiz do Drive (mesmas regras de lixeira e de caminhos). **Sem o marcador, nada é gravado** — protege contra HD desmontado, quando só resta o ponto de montagem vazio.
- **RN-08** — Se a pasta `Syncly/` registrada numa conta Google for para a lixeira ou sumir, o app **não cria outra silenciosamente**: as operações falham pedindo reconexão. Ao reconectar, ele reencontra a pasta (se tiver sido restaurada) ou cria uma nova.

## 3. Tarefas

- **RN-10** — Nome obrigatório, 1 a 64 caracteres, sem caracteres de controle, único na configuração (sem diferenciar maiúsculas).
- **RN-11** — Fontes: de 1 a 32 caminhos **absolutos**, sem `..`, únicos na tarefa (tarefas que recebem não têm fontes, RN-110).
- **RN-12** — Fontes não podem ser pastas de sistema (`/`, `/proc`, `/sys`, `/dev`, `/run`) nem estar dentro das pastas de dados/config do próprio app. Se uma fonte **contém** as pastas do app (ex.: a pasta pessoal inteira), elas são puladas automaticamente.
- **RN-13** — Uma fonte não pode estar dentro de outra fonte da mesma tarefa.
- **RN-14** — O **nome final** (basename) de cada fonte deve ser único na tarefa, pois vira a pasta/arquivo de primeiro nível no destino. Ex.: `~/Documentos` e `~/notas.md` → `Syncly/<destino>/Documentos/…` e `Syncly/<destino>/notas.md`.
- **RN-15** — Toda tarefa tem um destino e faz uma de três coisas: **envia arquivo por arquivo** (seção 5, padrão), **envia compactado** (seção 7) ou **recebe e descompacta** (RN-107).
- **RN-16** — Destino: conta existente + caminho relativo à pasta raiz, com 1 a 32 componentes, sem `.`/`..`, sem caracteres de controle, até 255 bytes por componente.
- **RN-17** — Duas tarefas não podem ter destinos sobrepostos na mesma conta (iguais ou um dentro do outro) — cada pasta remota tem um único "dono". Tarefas que recebem só leem, então ficam de fora: podem apontar para a pasta de uma tarefa que envia.
- **RN-18** — Tarefa desativada não roda (nem manual nem agendada) e seus timers são removidos.
- **RN-19** — Com destino numa conta local, a pasta de destino não pode ficar dentro de uma fonte da tarefa, nem conter uma fonte — senão a cada execução a cópia copiaria a si mesma.

## 4. Fontes e filtros

- **RN-20** — Exclusões usam sintaxe do `.gitignore`. Padrões **sem** `/` valem em qualquer nível; padrões **com** `/` são relativos à raiz de cada fonte.
- **RN-21** — Exclusões padrão (sempre ativas): lixo de sistema e temporários — `.DS_Store`, `Thumbs.db`, `desktop.ini`, `*.tmp`, `*.swp`, `*~`, `~$*`, `.~lock.*#`, `.Trash-*/`, `.syncly-*`.
- **RN-22** — **Arquivos sensíveis são excluídos por padrão**: `.ssh/`, `.gnupg/`, `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`, `*.kdbx`, `.netrc`, `.pgpass`. Só são incluídos se a tarefa marcar explicitamente `include_sensitive = true`. Vale também para a **própria fonte**: escolher `~/.ssh` ou um `.env` como fonte exige essa marcação, e um `.syncignore` não consegue reincluir arquivos sensíveis.
- **RN-23** — Cada pasta pode ter um arquivo `.syncignore` com exclusões adicionais (mesma sintaxe). As exclusões da tarefa têm precedência; entre `.syncignore`, o mais profundo vence (inclusive `!padrão` para reincluir).
- **RN-24** — Links simbólicos **não são seguidos** nem enviados; são ignorados com aviso. Arquivos especiais (sockets, FIFOs, dispositivos) também.
- **RN-25** — Um arquivo cujo tamanho ou mtime mudou durante o envio é marcado como **instável** e reenviado na próxima execução. No índice, ele fica marcado para ter o hash recalculado.
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

- **RN-30** — O planejador é uma função pura `(local, índice, remoto, modo) → plano` e é a **única** fonte de decisões. No envio compactado (seção 7), as decisões também são funções puras: guarda (`evaluate_archive`), nome e impressão digital da versão, e retenção.
- **RN-31** — **O app nunca altera nem apaga um item remoto que ele não sabe que sincronizou** (sem entrada no índice).
- **RN-32** — Pastas: criadas antes dos arquivos (ordem de profundidade). No modo Espelho, uma pasta só vai para a lixeira se **todo** o seu conteúdo remoto também for para a lixeira no mesmo plano.
- **RN-33** — Tipos divergentes (arquivo local × pasta remota com o mesmo nome, ou o inverso) → item ignorado com aviso.
- **RN-34** — Itens remotos sem MD5 (ex.: Google Docs nativos), com nome inválido localmente (contém `/`, `.` ou `..`) ou **duplicados** na mesma pasta → ignorados com aviso. O app nunca "adivinha".
- **RN-35** — Conflito: a versão remota é **renomeada** para `<nome> (conflito remoto AAAA-MM-DD HH-MM).<ext>` (se o nome já existir: `… 14-30 2).<ext>`, `… 14-30 3).<ext>`…) e a versão local é enviada com o nome original. Nenhuma versão se perde.
- **RN-36** — A primeira execução de toda tarefa é **sempre** uma prévia com confirmação. Agendamentos só ficam ativos após a primeira execução manual bem-sucedida.
- **RN-37** — Cada envio é verificado: MD5 calculado durante o upload deve bater com o `md5Checksum` devolvido pelo Drive. Divergência → nova tentativa; persistindo, item falha.
- **RN-38** — O índice é gravado imediatamente após cada item confirmado. Execução interrompida é retomada sem duplicar nada. Se um envio falhar sem resposta, antes de tentar de novo o app procura o arquivo no destino — ele pode ter chegado — para não criar duplicata.
- **RN-39** — Ordem de execução: criar pastas → enviar/atualizar/conflitos (em paralelo) → lixeira de arquivos → lixeira de pastas (mais profundas primeiro).

## 6. Proteções (guardas)

- **RN-40** — Fonte inexistente ou ilegível → execução **abortada**. A tarefa é revalidada antes de cada execução (o `config.toml` pode ter sido editado à mão).
- **RN-41** — Fonte **vazia** enquanto o índice tem arquivos dela (ou, no envio compactado, enquanto já há versões no destino) → execução **abortada** (protege contra HD externo desmontado: o ponto de montagem existe, mas vazio).
- **RN-42** — Plano que move para a lixeira mais de **50 arquivos** ou mais de **25 %** dos arquivos indexados exige **confirmação explícita**. Valores configuráveis por tarefa (1–100 %). Não se aplica ao envio compactado, que só manda para a lixeira versões antigas que ele mesmo criou (RN-55).
- **RN-43** — Execuções agendadas **nunca** confirmam sozinhas: se a guarda pedir confirmação, a execução fica *aguardando confirmação* e o usuário é notificado.
- **RN-44** — Não existe exclusão definitiva. Remoto: lixeira do Drive (recuperável por 30 dias). Local (conta local e restaurações): pasta `.syncly-trash/` ou lixeira do sistema.
- **RN-45** — Na CLI, planos que exigem confirmação só rodam com `--confirm`.
- **RN-46** — **Reserva de espaço**: antes de enviar, o app consulta a cota. Se o espaço livre após o envio ficar abaixo de **1 GiB** (configurável), a execução é abortada — conta Google cheia para de receber e-mails no Gmail.

## 7. Envio compactado (versões)

Em vez de copiar arquivo por arquivo, a tarefa junta as fontes num **único arquivo
compactado** a cada execução e o envia ao destino, guardando versões anteriores. É o
**backup** do app: remoto com uma conta Google, local com uma conta do tipo pasta (HD externo,
NAS). Vale a pena para pastas com muitos arquivos pequenos (ex.: saves de jogos) —
uma transferência só, em vez de milhares.

- **RN-50** — Versão = arquivo único `<tarefa>_<AAAA-MM-DD_HH-MM-SS>.<ext>` (hora local) no formato escolhido na tarefa (RN-60), com as fontes no primeiro nível (RN-14) e os mesmos filtros (RN-64). Nome já usado no destino ganha ` (2)`, ` (3)`… — nunca sobrescreve. O que não pôde ser lido fica de fora e a execução termina como *sucesso parcial* (RN-91). *(Planejado: `MANIFEST.json` com o sha256 de cada arquivo e `.age` se criptografado — seção 9.)*
- **RN-51** — Destino: a pasta da tarefa (RN-16), em qualquer conta. As versões ficam lado a lado nessa pasta.
- **RN-52** — O arquivo é montado num temporário do app (`~/.local/share/syncly/tmp/`), enviado com checksum conferido (RN-37) e só então registrado como versão. O temporário é apagado ao final — com sucesso, erro ou cancelamento — e sobras de uma execução interrompida são limpas na próxima. Versão incompleta nunca aparece como válida.
- **RN-53** — Antes de compactar, o app checa o espaço no computador (estimativa = tamanho total das fontes + a reserva de RN-67); antes de enviar, a cota da conta com o tamanho real (RN-46).
- **RN-54** — Retenção (padrão): últimas **3** + a mais recente de cada um dos **7** dias mais recentes que tiverem versão + idem para **4** semanas e **6** meses, em horário local. Configurável na tarefa; mínimo de 1 "últimas".
- **RN-55** — A retenção só roda **depois** de uma versão nova enviada e verificada, nunca remove a mais recente e só considera versões **registradas pelo app** naquele destino — outros arquivos da pasta nunca entram. Versão renomeada, substituída ou removida fora do app é esquecida, não vai para a lixeira. Remoções vão para a lixeira (RN-44).
- **RN-56** — Recuperação: o índice pode ser reconstruído a partir do destino (MD5 + `appProperties`), sem reenviar nada (adoção, tabela da seção 5). *(planejado)*
- **RN-59** — Se nada mudou nas fontes desde a última versão (mesmos caminhos, tamanhos e datas, mesmo formato) e ela continua intacta no destino, a execução termina com sucesso **sem enviar nada**.

As guardas da seção 6 valem aqui também: primeira execução com confirmação (RN-36), fonte
ausente ou esvaziada (RN-40, RN-41), agendada nunca confirma sozinha (RN-43).

### Restauração

Tela **Tarefas → Versões → Restaurar…** ou `syncly restore <tarefa> --to <pasta>`.

- **RN-57** — Restaurar uma versão a descompacta sempre numa **pasta nova** `<nome da versão>/` dentro da pasta escolhida (`(2)`, `(3)`… se já existir; RN-65) — nunca por cima de arquivos existentes. A pasta escolhida não pode ficar dentro de uma fonte da tarefa (a próxima versão levaria a restauração junto) nem das pastas do app. A restauração conta como execução da tarefa: usa a trava dela (RN-90) — a retenção não remove a versão no meio do caminho — e vai para o histórico. A rotina de recebimento (RN-107) segue a mesma regra.
- **RN-58** — Antes de descompactar, a versão é conferida: precisa estar intacta no destino (nome e tamanho registrados, RN-55) e o arquivo baixado precisa ter o **mesmo checksum registrado no envio** — o que também pega uma versão trocada por outra de mesmo nome e tamanho. Se não conferir, nada é descompactado. Mesmo conferido, o conteúdo é tratado como não confiável (RN-66).

### Rotina de recebimento

Tarefa com **Receber e descompactar** (tela Tarefas) — `syncly run <tarefa>`.

- **RN-107** — Uma tarefa que **recebe** busca, na pasta de destino, a versão mais recente — o arquivo com nome de versão (`<tarefa>_AAAA-MM-DD_HH-MM-SS[ (n)].<ext>`, RN-50) de data mais nova; outros arquivos são ignorados, e nomes repetidos bloqueiam (RN-34). Ela é baixada (checksum conferido pelo provedor, RN-106) e descompactada **sempre numa pasta nova** `<nome da versão>/` na pasta do computador (RN-57, RN-65 a RN-67), nunca por cima. Se a versão mais recente já foi recebida (mesmo item, nome, tamanho e, quando houver, conteúdo), nada é feito. A pasta do computador ausente bloqueia a execução (RN-40: disco desmontado); a primeira execução pede confirmação (RN-36) e a agendada nunca confirma sozinha (RN-43). No Drive, só enxerga o que o Syncly enviou (escopo `drive.file`, SEG-03) — inclusive de outro computador com o mesmo projeto do Google Cloud (ainda não testado com duas máquinas).
- **RN-110** — Uma tarefa que recebe **não tem fontes** nem exclusões. A pasta do computador segue as restrições de uma fonte (RN-12): caminho absoluto, fora das pastas de sistema e das pastas do app.

## 8. Compactação e descompactação

Tela **Compactar** e `syncly archive create|extract`.

### Compactar

- **RN-60** — Ação avulsa "Compactar": até 32 pastas/arquivos → `.zip` (padrão na interface; abre em qualquer sistema), `.tar.gz` (padrão no Linux), `.tar.zst` (mais rápido e menor) ou `.tar` (sem compressão). Tarefas que enviam compactado (RN-50) usam o formato escolhido nelas. Nome padrão: o da única entrada, ou `arquivos`.
- **RN-61** — Níveis padrão: zstd 3, gzip 6, deflate 6. No `.zip`, arquivos já comprimidos (jpg, png, mp4, zip, gz, 7z…) são armazenados sem recompressão. O bit de execução é preservado.
- **RN-62** — O arquivo de saída não pode estar dentro das entradas e **nunca sobrescreve** um existente (`nome (2).zip`, `nome (2).tar.gz`…). É escrito num temporário e só ganha o nome final ao terminar — erro ou cancelamento não deixam arquivo pela metade.
- **RN-63** — Criptografia só para `.tar.zst` (gera `.tar.zst.age`) — fase 2e.
- **RN-64** — Valem os filtros da sincronização: lixo e temporários (RN-21) ficam de fora; **arquivos sensíveis** (RN-22) só entram se marcados; `.syncignore` é respeitado; links e arquivos especiais não entram; o que não pôde ser lido vira aviso. Se nada sobrar, nada é criado.

### Descompactar

- **RN-65** — O conteúdo vai sempre para uma **pasta nova** em `destino/<nome do arquivo>/` (`(2)`, `(3)`… se já existir) — nunca por cima de arquivos existentes. Se o compactado tiver uma única pasta no topo, o conteúdo dela vai direto para a pasta nova (sem `Mundo/Mundo/`). A extração acontece numa pasta temporária, que só ganha o nome final se tudo der certo.
- **RN-66** — Arquivos compactados vindos de fora são **entrada não confiável**. Entradas com caminho absoluto, `..`, nomes inválidos, links (simbólicos ou não) e arquivos especiais são **ignoradas** e listadas no resultado. Entradas repetidas não sobrescrevem a primeira. Só o bit de execução é aplicado — setuid/setgid nunca.
- **RN-67** — Proteção contra "bomba de compressão" e disco cheio: compactar ou descompactar nunca deixa menos de **1 GiB** livre no disco; se fosse deixar, a operação para e nada fica pela metade.

## 9. Criptografia *(planejado — fase 2e)*

- **RN-70** — Opcional, por tarefa, aplicada a versões (seção 7) e compactações. A sincronização envia arquivos como estão (para serem visualizáveis no Drive).
- **RN-71** — Formato `age` com chave X25519. A chave pública fica no `config.toml`; a privada, no keyring.
- **RN-72** — Ativar criptografia exige **exportar a chave de recuperação** e confirmar que ela foi guardada. Sem ela, as versões criptografadas são irrecuperáveis — o app avisa isso explicitamente.
- **RN-73** — Versões automáticas usam só a chave pública; a privada só é lida na restauração.
- **RN-74** — Cada versão registra a impressão digital da chave usada; trocar a chave não invalida versões antigas.

## 10. Agendamento *(planejado — fase 3)*

- **RN-80** — Opções: manual; a cada N horas (N ∈ {1, 2, 3, 4, 6, 8, 12}); diário às HH:MM; semanal (dia + HH:MM); avançado (expressão `OnCalendar` do systemd).
- **RN-81** — Um agendamento por tarefa, qualquer que seja a forma de envio. Para sincronizar e também guardar versões das mesmas pastas, crie duas tarefas com destinos diferentes (RN-17).
- **RN-82** — Expressão avançada: até 64 caracteres, apenas `[A-Za-z0-9 *-:/,.~]`, validada por `systemd-analyze calendar` antes de salvar.
- **RN-83** — Execução perdida (computador desligado) roda no próximo boot/login.
- **RN-84** — Se o keyring estiver bloqueado ou não houver rede, a execução fica **adiada**; notifica após 3 adiamentos seguidos.
- **RN-85** — Conta em *requer reautenticação* suspende os agendamentos dela.

## 11. Execuções, histórico e notificações

- **RN-90** — No máximo uma execução por tarefa ao mesmo tempo (trava exclusiva); um disparo concorrente é descartado e registrado.
- **RN-91** — Status possíveis: `sucesso`, `sucesso_parcial` (alguns itens falharam), `falha`, `abortada` (guarda, espaço, fonte), `aguardando_confirmacao`, `cancelada`, `adiada`.
- **RN-92** — Estatísticas por execução: itens e bytes enviados/atualizados/na lixeira/ignorados/com falha, duração.
- **RN-93** — Histórico mantido por 180 dias ou 1000 execuções por tarefa (o que vier primeiro). *(planejado)*
- **RN-94** — Notifica em `falha`, `abortada`, `aguardando_confirmacao` e adiamentos repetidos. Sucesso é silencioso (configurável). *(planejado — fase 3)*
- **RN-95** — Cancelar uma execução interrompe as transferências em andamento; o que já foi confirmado permanece no índice (RN-38).

### Tratamento de erros

- **RN-96** — Erros transitórios (rede, 408, 429, 5xx, cotas de taxa) → nova tentativa com backoff exponencial e jitter (1 s → 64 s, até 6 tentativas), respeitando `Retry-After`.
- **RN-97** — Erros permanentes de item (404, 403 de permissão, 400) → o item falha, os demais seguem.
- **RN-98** — Erros que afetam a conta inteira → a execução é **abortada** imediatamente: autenticação (`401`, RN-06), armazenamento cheio (`storageQuotaExceeded`, `activeItemCreationLimitExceeded`), cota diária do projeto (`dailyLimitExceeded`).
- **RN-99** — Limite de taxa persistente: se **3 itens seguidos** esgotarem as tentativas por `rateLimitExceeded`/`userRateLimitExceeded`/`429`, a execução é abortada como **adiada**. É assim que o limite de **750 GB/dia** de upload do Google se manifesta — o app para em vez de insistir.

## 12. Status e última modificação *(planejado — fase 1c)*

- **RN-100** — Para cada tarefa: última execução (data, status, estatísticas), próxima execução agendada, total de arquivos e bytes.
- **RN-101** — Para cada arquivo ou pasta de uma tarefa: data de modificação local; data de modificação e **autor da última modificação** no destino; data do último sync; situação — `sincronizado`, `alterado localmente`, `alterado no destino`, `pendente`, `conflito`, `ignorado`.
- **RN-102** — Para pastas, a "última modificação" é a mais recente entre todos os arquivos dentro dela.
- **RN-103** — A consulta de status usa o índice (instantânea, funciona offline); "Atualizar do destino" busca os dados atuais na nuvem.

## 13. Transferências avulsas

Enviar ou baixar um arquivo específico — ou uma pasta compactada — fora de uma tarefa (tela
**Arquivos** e `syncly remote`).

- **RN-104** — Envio cria as pastas que faltarem dentro de `Syncly/`, mas **nunca sobrescreve nem duplica**: se já existir um item com o mesmo nome na pasta, o envio é recusado.
- **RN-105** — Os filtros de arquivos sensíveis e temporários (RN-21, RN-22) valem também para o envio avulso.
- **RN-106** — Download **nunca sobrescreve**: se o nome já existir na pasta local, grava como `nome (2).ext`, `nome (3).ext`… Todo envio e download é verificado por checksum (RN-37) e o download é gravado de forma atômica. Documentos nativos do Google (sem conteúdo binário) não são baixados.
- **RN-108** — **Baixar e descompactar** (`remote get --extract`): um `.zip`/`.tar.*` da conta é baixado (checksum conferido) e descompactado direto numa pasta nova no computador, com as proteções de RN-65 a RN-67. Formato desconhecido é recusado antes de baixar.
- **RN-109** — **Enviar pasta compactada** (`remote put --compress`): pastas/arquivos são compactados (RN-60 a RN-64) num temporário do app e enviados como um único arquivo `<nome>.<ext>`, sem criar tarefa. Vale RN-104 (nunca sobrescreve nem duplica, conferido antes e depois de compactar), RN-105 (sensíveis **sempre** ficam de fora) e a reserva de espaço da conta (RN-46).
