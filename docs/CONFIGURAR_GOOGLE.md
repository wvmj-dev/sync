# Configurar o Google Cloud (uma vez só)

O Syncly usa um projeto do Google Cloud **seu**: as cotas da API são só suas, ninguém
mais usa o mesmo cliente OAuth e você pode revogar tudo quando quiser. Leva ~10 minutos.

> Os nomes dos menus mudam de tempos em tempos. Se não encontrar algo, procure pelo
> termo em inglês entre parênteses.

## 1. Criar o projeto

1. Acesse <https://console.cloud.google.com/> com a conta Google que vai usar.
2. Seletor de projetos → **Novo projeto** → nome sugerido: `syncly-pessoal` → **Criar**.
3. Confirme que o projeto novo está selecionado no topo da página.

## 2. Ativar a Google Drive API

1. Menu → **APIs e serviços** → **Biblioteca** (*Library*).
2. Pesquise **Google Drive API** → **Ativar** (*Enable*).

## 3. Tela de consentimento (Google Auth Platform)

1. Menu → **Google Auth Platform** (antigo *Tela de consentimento OAuth*) → **Começar**.
2. **Informações do app** (*Branding*): nome `Syncly`, e-mail de suporte = seu e-mail,
   contato do desenvolvedor = seu e-mail. **Não envie logotipo** — isso exige verificação da
   marca pelo Google.
3. **Público-alvo** (*Audience*): **Externo** (*External*). Em **Usuários de teste**, adicione
   o seu próprio e-mail.
4. **Acesso a dados** (*Data Access*) → **Adicionar ou remover escopos**. A janela lista dezenas
   de escopos; não procure na lista. Role até **"Adicionar escopos manualmente"**, cole:

   ```
   https://www.googleapis.com/auth/drive.file
   ```

   e clique em **"Adicionar à tabela"** → confira que ficou marcado → **"Atualizar"**.
   **Não marque nenhum outro** — nem `userinfo.email`/`openid` (o app obtém o e-mail pela
   própria Drive API), nem BigQuery/`cloud-platform` (ativados automaticamente em projetos
   novos; não são usados).
5. Clique em **"Salvar"**. O escopo deve aparecer em **"Seus escopos não confidenciais"**, com a
   descrição *"Ver, editar, criar e excluir apenas os arquivos do Google Drive que você usa com
   este app"*. Se der erro ao adicionar, a Google Drive API ainda não está ativada (passo 2).

## 4. Criar o cliente OAuth

1. **Google Auth Platform** → **Clientes** (*Clients*) → **Criar cliente**.
2. Tipo de aplicativo: **App para computador** (*Desktop app*). Nome: `Syncly desktop`.
3. **Criar** → **Baixar JSON**.

## 5. Publicar o app (só antes de usar agendamento)

**Durante o desenvolvimento e os testes, deixe em "Testando".** Tudo funciona igual para os
e-mails cadastrados como usuários de teste; a única diferença é que o login **expira a cada
7 dias** e precisa ser refeito. Isso só atrapalha quando houver sincronização agendada (fase 3).

Para publicar, o Google exige a página **Branding** completa — o botão **Publicar app** fica
desativado com a mensagem *"conclua a configuração na página de branding"* até lá:

1. **Página inicial do app** e **Link da política de privacidade**: URLs públicas e acessíveis.
   Opção gratuita: GitHub Pages (`seu-usuario.github.io/syncly`), com uma página simples e uma
   política dizendo que o app roda localmente, não coleta dados e acessa só os arquivos que
   ele cria no Drive (`drive.file`).
2. **Domínios autorizados**: o domínio dessas URLs (ex.: `seu-usuario.github.io`).
3. **Público-alvo** → **Publicar app** → confirmar.

Como `drive.file` é um escopo **não sensível** e não há logotipo, publicar não exige o
processo de verificação de segurança do Google.

## 6. Entregar o JSON ao Syncly

```bash
mkdir -p ~/.config/syncly && chmod 700 ~/.config/syncly
mv ~/Downloads/client_secret_*.json ~/.config/syncly/google-client.json
chmod 600 ~/.config/syncly/google-client.json
```

Depois, no app: **Contas → Conectar Google Drive** (fase 1).

## Boas práticas

- **Nunca** versione esse JSON nem o envie para ninguém. Ele está no `.gitignore`.
- **Mantenha o mesmo projeto** do Google Cloud. Com `drive.file`, o acesso aos arquivos
  criados pelo app é vinculado ao projeto: um projeto novo não enxerga os arquivos antigos
  (seria preciso reenviar). Trocar só o *cliente* dentro do mesmo projeto não tem problema.
- Se o JSON vazar: exclua o cliente no console, crie outro e substitua o arquivo.
- Para revogar o acesso do app à sua conta: <https://myaccount.google.com/connections>.
