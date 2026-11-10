# asaas-cli

CLI em Rust para a [API v3 do Asaas](https://docs.asaas.com), inspirada no `gh`, feita para ser consumida por agentes de IA e automações: saída sempre em JSON, sem cores, sem prompts.

## Como rodar

### Requisitos

- [Rust](https://rustup.rs/) 1.88 ou mais recente (`cargo --version` para conferir)
- `make` (opcional; já vem no macOS e na maioria das distribuições Linux)
- Uma chave de API do Asaas, gerada no painel em Integrações → Chaves de API. Para testar sem mexer em dinheiro de verdade, crie uma conta em [sandbox.asaas.com](https://sandbox.asaas.com).

### Instalação

```bash
git clone https://github.com/pablorobert/asaas-cli-rs.git asaas-cli
cd asaas-cli
make install        # compila e instala em ~/.local/bin/asaas
```

`~/.local/bin` precisa estar no `PATH`. Se `asaas` não for encontrado depois da instalação, adicione ao `~/.zshrc` (ou `~/.bashrc`):

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Outras formas:

```bash
make install PREFIX=/usr/local   # instala em /usr/local/bin/asaas
make build                       # só compila em ./bin/asaas, sem instalar
cargo run -- --help              # roda direto do código
make uninstall                   # remove o binário instalado
```

Sem `make` (no Windows, por exemplo): `cargo install --path . --locked` instala o binário `asaas` em `~/.cargo/bin`.

### Primeiros passos

```bash
# 1. Cadastre a conta. Use aspas simples: a chave começa com "$aact_" e o shell
#    trataria "$aact" como variável.
asaas auth add --name sandbox-dev --key '$aact_hmlg_...' --sandbox

#    Para não deixar a chave no histórico do shell, passe pelo stdin:
pbpaste | asaas auth add --name sandbox-dev --key - --sandbox

# 2. Confira se a chave funciona
asaas auth status --verify

# 3. Faça a primeira chamada
asaas customer list --limit 5
```

Sem arquivo de configuração, dá para usar só a variável de ambiente:

```bash
ASAAS_API_KEY='$aact_hmlg_...' asaas customer list --limit 5
```

Todo comando tem `--help` com a descrição completa, o tipo de cada flag e quais são `[required]`.

### Testes

```bash
make test           # cargo test --locked
make lint           # cargo fmt --check + cargo clippy
```

Os testes usam um servidor HTTP falso local: não precisam de chave nem de rede.

## Contrato de saída

| Situação | Destino | Formato |
|---|---|---|
| Sucesso | stdout | JSON indentado (corpo cru da resposta do Asaas) |
| Erro | stderr | `{"error": {"type": "...", "message": "...", ...}}` |

Tipos de erro e exit codes: `usage_error` (2), `auth_error`/`config_error` (3), `network_error` (4), `api_error`/`response_error`/`internal_error` (1). `api_error` inclui `status_code` e o array `errors` do Asaas.

## Autenticação

Ordem de resolução:

1. `--account <nome>`: conta específica do arquivo, só nesta execução.
2. `ASAAS_API_KEY`: usada direto, ignorando o arquivo.
3. `active_account` de `~/.config/asaas/config.json`.

Ambiente: `--sandbox`/`--production` sempre vencem. Sem flag, vale o `environment` da conta; com `ASAAS_API_KEY`, valem `ASAAS_ENV`, depois o prefixo da chave (`$aact_hmlg_` = sandbox, `$aact_prod_` = produção) e, por último, produção.

```bash
asaas auth add --name sandbox-dev --key "$KEY" --sandbox
printf '%s' "$KEY" | asaas auth add --name producao-firma --key - --production
asaas auth switch --name sandbox-dev
asaas auth status --verify
asaas auth list
asaas auth remove --name producao-firma
```

O arquivo é criado com permissão `0600` (diretório `0700`). Variáveis de ambiente: `ASAAS_API_KEY`, `ASAAS_ENV`, `ASAAS_CONFIG_DIR`, `ASAAS_BASE_URL`.

## Comandos

```bash
asaas customer list --name Maria --limit 20
asaas customer list --all                      # percorre todas as páginas
asaas customer get cus_000005219613
asaas customer create --name "Ana" --cpf-cnpj 24971563792 --email ana@x.com
asaas customer update cus_000005219613 --mobile-phone 47999376637
asaas customer delete cus_000005219613
asaas customer restore cus_000005219613

# Notas fiscais de serviço (NFS-e)
asaas fiscal-info municipal-options            # o que a prefeitura exige
asaas fiscal-info save --email fiscal@empresa.com --simples-nacional=true --municipal-inscription 123456
asaas fiscal-info services --description desenvolvimento   # municipalServiceId
asaas invoice create --payment pay_080225913252 --service-description "Consultoria" \
  --observations "Mensal" --value 300 --effective-date 2026-10-31 --municipal-service-id 60380 \
  --retain-iss=false --iss 2 --pis 0 --cofins 0 --csll 0 --inss 0 --ir 0
asaas invoice authorize inv_000000000232       # emite agora, sem esperar effectiveDate
asaas invoice list --status AUTHORIZED --effective-date-from 2026-10-01 --all
asaas invoice update inv_000000000232 --iss 3.5
asaas invoice cancel inv_000000000232

# Qualquer endpoint sem comando dedicado:
asaas api /payments --query status=PENDING
asaas api /payments -X POST -f customer=cus_000005219613 -f billingType=PIX -F value=129.90 -f dueDate=2026-12-31
```

Toda a documentação (tipos de flag, `[required]`, formato de saída) está no `--help` de cada comando.

## Usando com agentes de IA

A CLI foi feita para ser operada por agentes como Claude Code, Codex e Cursor: o agente descobre os comandos pelo `--help` e lê as respostas em JSON. Basta instalar o `asaas`, cadastrar uma conta e dar ao agente instruções como estas. Elas podem ir no `CLAUDE.md`, no `AGENTS.md` ou direto no chat:

```text
Você tem acesso à CLI `asaas` para operar minha conta no Asaas (API v3).

- Antes de usar um comando, leia `asaas --help` e `asaas <comando> --help`.
  O help mostra o tipo de cada flag e quais são [required].
- Sucesso sai em JSON no stdout. Erro sai em JSON no stderr, com exit code
  diferente de 0. Use jq para filtrar.
- Em listagens, use --all quando precisar de todos os registros.
- Para endpoints sem comando dedicado, use `asaas api <caminho>`.
- Comandos de leitura (list, get, auth status) podem rodar à vontade.
- Antes de qualquer comando que cria, altera, cancela ou emite algo (create,
  update, delete, authorize, cancel, `asaas api -X POST/PUT/DELETE`), me
  mostre exatamente o que vai rodar e espere minha confirmação. Emitir ou
  cancelar nota fiscal não tem volta.
- Na dúvida sobre a conta ou o ambiente, rode `asaas auth status`. Para
  testar, use --sandbox.
```

Exemplos de pedidos:

```text
Liste as notas fiscais com status ERROR com data de emissão em setembro e
outubro de 2026, agrupe pelo código de erro devolvido pela prefeitura e me
diga quantas são e o valor total de cada grupo. Não altere nada.
```

```text
Quais cobranças do cliente cus_000005219613 foram pagas no cartão pelo
Asaas e quais foram baixadas manualmente como recebidas em dinheiro?
```

## Estrutura

```
src/main.rs             entrypoint (std::process::exit(asaas_cli::execute()))
build.rs                versão (ASAAS_VERSION) e versão do rustc embutidas no binário
src/cmd/                árvore clap: root (mod.rs), version
  auth.rs               add, switch, status, list, remove
  customer.rs           list, get, create, update, delete, restore
  invoice.rs            list, get, create, update, authorize, cancel (NFS-e)
  fiscalinfo.rs         get, save, municipal-options, services, nbs-codes, federal-service-codes, national-portal
  api.rs                requisição crua para qualquer endpoint
src/config/             config.json, variáveis ASAAS_*, resolução de credenciais
src/cmdutil/            Factory (estilo gh), saída JSON, erros tipados, campos declarativos
src/client/             cliente HTTP genérico (headers, erros, paginação)
tests/cli.rs            testes ponta a ponta contra um servidor Asaas falso
```

### Adicionando um recurso novo (ex.: `payment`)

1. Crie `src/cmd/payment.rs` com um grupo `cmdutil::new_group(...)` e uma função `run` que despacha os subcomandos.
2. Declare filtros e campos como `Vec<cmdutil::Field>` (parâmetro da API, flag, tipo, descrição, obrigatório). Parâmetros com ponto (`taxes.iss`) viram objetos aninhados; `one_of(...)` e o tipo `date()` são validados antes da requisição.
3. Use `cmdutil::run_list` para listagens e `cmdutil::run_request` para get/create/update/delete.
4. Registre o grupo em `src/cmd/mod.rs` (`root_command` e `dispatch`).

O teste `every_command_is_documented` falha se algum comando ficar sem descrição curta/longa ou se alguma flag não documentar o tipo.

## Licença

[MIT](LICENSE)
