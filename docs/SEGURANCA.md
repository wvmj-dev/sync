# Segurança

## 1. Por que sua conta Google não corre risco

| Preocupação | Como o Syncly evita |
|---|---|
| "O app pode ver ou apagar meus outros arquivos do Drive?" | Usa o escopo **`drive.file`**: o Google só deixa o app acessar arquivos **que ele mesmo criou**. Fotos, documentos, Gmail etc. ficam inacessíveis — mesmo que o app tenha um bug. |
| "Minha senha fica exposta?" | O app nunca vê a senha. O login acontece no navegador, na página do Google (OAuth 2.0 + PKCE). |
| "A conta pode ser bloqueada por uso indevido?" | O app usa só a **API oficial do Drive v3**, do jeito documentado, num projeto do Google Cloud **seu**. Não há scraping, cliente não oficial nem automação de navegador. Erros de cota são tratados com backoff, e o app para em vez de insistir. |
| "E se o app apagar algo por engano?" | Não existe exclusão definitiva no código. Tudo vai para a lixeira do Drive (30 dias). Guardas bloqueiam exclusões em massa. O Drive ainda mantém o histórico de versões de cada arquivo. |
| "E se encher meu armazenamento?" | O app checa a cota antes de enviar e mantém uma reserva mínima (1 GiB). Conta cheia faz o Gmail parar de receber e-mails — o app não deixa chegar lá. |
| "E se roubarem meu token?" | Tokens ficam no keyring do sistema (cifrado pelo SO), nunca em arquivo nem na interface web do app. Você pode revogar o acesso a qualquer momento em [myaccount.google.com/connections](https://myaccount.google.com/connections). |
| "O Google vai 'ler' meus backups?" | Backups podem ser criptografados localmente (`age`) antes do envio. O Google armazena só bytes cifrados. |

## 2. O que protegemos e de quem

**Ativos:** acesso à conta Google; arquivos do usuário (locais e remotos); tokens OAuth;
chaves de criptografia; integridade dos backups.

| Ameaça | Exemplo |
|---|---|
| Roubo de credenciais | Malware lendo um arquivo de token em texto puro. |
| Bug destrutivo | Erro de lógica que interpreta "pasta vazia" como "apagar tudo". |
| Injeção na webview | Um nome de arquivo malicioso renderizado como HTML que chama comandos do app. |
| Arquivo malicioso | Backup adulterado com caminhos `../../.bashrc` (zip slip). |
| Injeção em query/unit | Nome de arquivo com `'` quebrando a query do Drive; expressão de agendamento com quebra de linha injetando diretivas no systemd. |
| Abuso de cota | Loop de retentativas martelando a API. |
| Cadeia de suprimentos | Dependência comprometida. |

## 3. Controles

### Credenciais e OAuth

- **SEG-01** — OAuth 2.0 para *app instalado*: PKCE (S256) + parâmetro `state` + redirecionamento para `http://127.0.0.1:<porta aleatória>`. O servidor local só aceita a resposta com o `state` correto — outras requisições são ignoradas e não abortam o login —, responde com uma página estática (nada da requisição é refletido) e fecha; expira em 5 minutos.
- **SEG-02** — O login abre no **navegador do sistema**. O Google bloqueia OAuth em webviews embutidas, e o navegador mostra a URL real (anti-phishing).
- **SEG-03** — Escopo único: `https://www.googleapis.com/auth/drive.file`. Ampliar escopo (ex.: para o modo bidirecional enxergar arquivos criados pelo site do Drive) será uma decisão explícita, opcional e documentada — nunca silenciosa.
- **SEG-04** — Refresh tokens ficam no keyring do sistema (serviço `syncly`, entrada `account:<id>`). Access tokens só em memória.
- **SEG-05** — `google-client.json` fica em `~/.config/syncly/` com permissão `0600` e **nunca** é versionado (`.gitignore`). O app recusa clientes do tipo `web` — só `installed` (app para computador).
- **SEG-06** — "Desconectar" revoga o token em `https://oauth2.googleapis.com/revoke` antes de apagá-lo localmente.
- **SEG-07** — Falha de renovação (`invalid_grant`) não gera novas tentativas: a conta é marcada para reautenticação (RN-06). Insistir com token inválido gera alertas de segurança.
- **SEG-08** — O app confere o escopo que o usuário realmente concedeu (na tela do Google dá para desmarcar a permissão). Sem `drive.file`, a conexão falha com mensagem clara.

### Integridade e proteção dos dados

- **SEG-10** — A interface `StorageProvider` não possui exclusão definitiva; só `trash` (RN-44).
- **SEG-11** — O app só opera dentro da sua pasta raiz e só altera itens presentes no seu índice (RN-03, RN-31).
- **SEG-12** — Guardas contra exclusão em massa e contra fonte vazia/desmontada (RN-40 a RN-43).
- **SEG-13** — Verificação de checksum em todo upload, download e backup (RN-37, RN-52, RN-58).
- **SEG-14** — Escrita atômica: arquivo temporário + `fsync` + `rename`, em config, downloads, backups e compactações.

### Entrada não confiável

- **SEG-20** — Caminhos relativos (índice, IDs do `LocalProvider`, entradas de arquivos compactados) são validados: sem componentes absolutos, `..`, `.`, vazios ou com caracteres de controle. Nomes remotos com `/` são ignorados.
- **SEG-21** — Valores interpolados em queries do Drive são escapados (`\` e `'`). Filtros sempre incluem o ID do pai e `trashed = false`.
- **SEG-22** — Units do systemd contêm apenas valores gerados pelo app (UUID da tarefa, caminho do binário). Expressões de agendamento passam por allowlist de caracteres (sem quebras de linha) e por `systemd-analyze calendar`.
- **SEG-23** — Links simbólicos não são seguidos (RN-24), evitando sair da fonte e loops.
- **SEG-24** — Filtros de arquivos sensíveis valem também para a própria fonte e não podem ser desfeitos por `.syncignore` (RN-22). O `config.toml` é tratado como entrada não confiável: cada tarefa é revalidada antes de executar (RN-40).
- **SEG-25** — IDs vindos do Drive só entram em URLs se tiverem apenas letras, números, `-` e `_`; nomes remotos passam por `validate_name` antes de virar caminho local.

### Webview (Tauri)

- **SEG-30** — Allowlist de comandos: cada comando é declarado em `build.rs` e liberado em `capabilities/default.json`. Sem acesso amplo a sistema de arquivos ou shell pela webview.
- **SEG-31** — CSP: `default-src 'self'`, sem `unsafe-eval`, sem scripts ou conexões externas. A UI não carrega nada da internet.
- **SEG-32** — A UI renderiza nomes de arquivos sempre como texto (interpolação do Vue); `v-html` é proibido no projeto.
- **SEG-33** — Tokens, chaves e caminhos internos do keyring nunca cruzam a fronteira IPC. Erros enviados à UI não incluem detalhes internos.
- **SEG-34** — Instância única do app (plugin `single-instance`).

### Rede e uso da API

- **SEG-40** — HTTPS com `rustls`, validação de certificado sempre ligada; sem proxy/TLS customizado.
- **SEG-41** — Limitador de taxa (≈ 8 req/s por processo) + no máximo 8 transferências paralelas; backoff exponencial com jitter respeitando `Retry-After` (RN-96).
- **SEG-42** — Erros de cota de armazenamento ou de limite diário abortam a execução (RN-98); limite de taxa persistente adia a execução (RN-99).

### Código e dependências

- **SEG-50** — `unsafe_code = "forbid"` nos crates do projeto.
- **SEG-51** — `Cargo.lock` e `pnpm-lock.yaml` versionados; `cargo audit`/`cargo deny` e `pnpm audit` antes de cada release.
- **SEG-52** — Logs nunca registram tokens, cabeçalhos de autorização, chaves ou conteúdo de arquivos. Ficam em diretório `0700` (`~/.local/state/syncly/logs/`), com rotação diária e 14 dias de retenção.
- **SEG-53** — Diretórios de dados e config criados com `0700`; arquivos sensíveis com `0600`.

### Criptografia

- **SEG-60** — `age` (X25519 + ChaCha20-Poly1305), via crate `age`. Nenhuma criptografia "caseira".
- **SEG-61** — Chave privada só no keyring + arquivo de recuperação exportado pelo usuário (RN-72). Backups automáticos usam apenas a chave pública (RN-73).

## 4. Fora do escopo (limites conhecidos)

- Um malware rodando **como seu usuário**, com a sessão desbloqueada, pode pedir os segredos ao keyring. Nenhum app desktop resolve isso; mantenha o sistema atualizado.
- Arquivos sincronizados (não os backups) ficam legíveis pelo Google — é o que permite visualizá-los no Drive. Use backup criptografado para dados sensíveis.
- Arquivos colocados na pasta `Syncly/` pelo site do Drive não são visíveis ao app enquanto o escopo for `drive.file` (SEG-03).
