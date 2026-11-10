//! `asaas fiscal-info`: the account's fiscal configuration for issuing
//! service invoices (/v3/fiscalInfo).

use std::collections::BTreeMap;
use std::path::Path;

use clap::{ArgMatches, Command};
use serde_json::{Map, Value, json};

use crate::client::File;
use crate::cmdutil::{
    self, Factory, Field, Result, add_fields, bool_arg, changed, check_required, collect_body,
    get_bool, get_string, mark_required, pagination_fields, run_list, run_request, string_arg,
};
use crate::usage_err;

const BASE_PATH: &str = "/fiscalInfo";

/// Flags that accept "-" to read the value from stdin.
const SECRET_FLAGS: &[&str] = &["password", "access-token", "certificate-password"];

pub fn command() -> Command {
    cmdutil::new_group(
        "fiscal-info",
        "asaas fiscal-info <subcommand>",
        "Configure invoice (NFS-e) issuing: fiscal data, city requirements, service and NBS codes",
        r#"Manage the fiscal configuration that service invoices ('asaas invoice')
depend on (/v3/fiscalInfo).

Typical setup:
  1. 'fiscal-info municipal-options'  what the account's city hall requires
     (authenticationType: CERTIFICATE | TOKEN | USER_AND_PASSWORD, special
     tax regimes, service list item, whether cancellation is automatic).
  2. 'fiscal-info save'               send the company's fiscal data and the
     city hall credentials matching authenticationType.
  3. 'fiscal-info services'           find the municipalServiceId for invoices
     (or 'federal-service-codes' / 'nbs-codes' for national codes).
  4. 'fiscal-info national-portal'    switch issuing to the Portal Nacional
     da NFS-e, when applicable."#,
    )
    .subcommands([
        cmd_get(),
        cmd_save(),
        cmd_municipal_options(),
        cmd_services(),
        cmd_nbs_codes(),
        cmd_federal_service_codes(),
        cmd_national_portal(),
    ])
}

pub fn run(f: &mut Factory, cmd: &mut Command, m: &ArgMatches) -> Result<()> {
    match m.subcommand() {
        Some(("get", _)) => run_request(f, "GET", &format!("{BASE_PATH}/"), None, None),
        Some(("save", sub)) => run_save(f, sub),
        Some(("municipal-options", _)) => run_request(
            f,
            "GET",
            &format!("{BASE_PATH}/municipalOptions"),
            None,
            None,
        ),
        Some(("services", sub)) => list(f, sub, "/services", services_filters()),
        Some(("nbs-codes", sub)) => list(f, sub, "/nbsCodes", nbs_filters()),
        Some(("federal-service-codes", sub)) => {
            list(f, sub, "/federalServiceCodes", federal_filters())
        }
        Some(("national-portal", sub)) => {
            let mut body = Map::new();
            body.insert("enabled".into(), json!(get_bool(sub, "enabled")));
            run_request(
                f,
                "POST",
                &format!("{BASE_PATH}/nationalPortal"),
                None,
                Some(body),
            )
        }
        _ => cmdutil::print_group_help(f, cmd),
    }
}

fn list(f: &mut Factory, m: &ArgMatches, suffix: &str, filters: Vec<Field>) -> Result<()> {
    run_list(
        f,
        m,
        &format!("{BASE_PATH}{suffix}"),
        &with_pagination(filters),
        get_bool(m, "all"),
    )
}

fn with_pagination(mut filters: Vec<Field>) -> Vec<Field> {
    filters.extend(pagination_fields());
    filters
}

fn cmd_get() -> Command {
    cmdutil::new_command(
        "get",
        "asaas fiscal-info get",
        "Show the account's fiscal information (GET /v3/fiscalInfo)",
        r#"Show the fiscal information used to issue invoices (GET /v3/fiscalInfo).

Output (stdout): {"object", "email", "municipalInscription", "simplesNacional",
"culturalProjectsPromoter", "cnae", "specialTaxRegime", "serviceListItem",
"nbsCode", "rpsSerie", "rpsNumber", "loteNumber", "username", "passwordSent",
"accessTokenSent", "certificateSent", "nationalPortalTaxCalculationRegime",
"serviceProvisionCityDefaultType"}. Secrets are never returned, only the
*Sent booleans. Returns api_error 404 when nothing was configured yet."#,
    )
}

fn cmd_municipal_options() -> Command {
    cmdutil::new_command(
        "municipal-options",
        "asaas fiscal-info municipal-options",
        "Show what the account's city hall requires to issue invoices (GET /v3/fiscalInfo/municipalOptions)",
        r#"Show the city hall's requirements for the account's CNPJ city
(GET /v3/fiscalInfo/municipalOptions). Read this before 'fiscal-info save'.

Output (stdout):
  {"authenticationType": "CERTIFICATE" | "TOKEN" | "USER_AND_PASSWORD",
   "supportsCancellation": bool, "usesSpecialTaxRegimes": bool,
   "usesServiceListItem": bool,
   "specialTaxRegimesList": [{"label", "value"}],
   "nationalPortalTaxCalculationRegimeList": [{"label", "value"}],
   "...Help": string hints about each field's format}"#,
    )
}

fn cmd_national_portal() -> Command {
    let cmd = cmdutil::new_command(
        "national-portal",
        "asaas fiscal-info national-portal --enabled=<true|false>",
        "Enable or disable issuing through the Portal Nacional da NFS-e (POST /v3/fiscalInfo/nationalPortal)",
        r#"Enable or disable issuing invoices through the Portal Nacional da NFS-e
instead of the city hall's own system (POST /v3/fiscalInfo/nationalPortal).

On the Portal Nacional there is no municipal service list: create invoices
with --municipal-service-code instead of --municipal-service-id.

Output (stdout): the Asaas response."#,
    );
    cmdutil::with_examples(
        cmd,
        r#"  asaas fiscal-info national-portal --enabled=true
  asaas fiscal-info national-portal --enabled=false"#,
    )
    .arg(mark_required(bool_arg(
        "enabled",
        "true to issue through the Portal Nacional, false to use the city hall system [type: boolean]",
    )))
}

/// A paginated lookup command over a /fiscalInfo list endpoint.
fn list_cmd(
    name: &'static str,
    usage: &'static str,
    short: &'static str,
    long: &'static str,
    examples: &'static str,
    filters: Vec<Field>,
) -> Command {
    let cmd = cmdutil::with_examples(cmdutil::new_command(name, usage, short, long), examples);
    add_fields(cmd, &with_pagination(filters)).arg(bool_arg(
        "all",
        "Fetch every page and return all matching items in one list [type: boolean]",
    ))
}

fn services_filters() -> Vec<Field> {
    vec![Field::new(
        "description",
        "description",
        "Filter by service name (partial match)",
    )]
}

fn nbs_filters() -> Vec<Field> {
    vec![Field::new(
        "codeDescription",
        "search",
        "Filter by NBS code or description (partial match)",
    )]
}

fn federal_filters() -> Vec<Field> {
    vec![
        Field::new("code", "code", "Filter by tax code (e.g. 01.07.01)"),
        Field::new(
            "description",
            "description",
            "Filter by description (partial match)",
        ),
    ]
}

fn cmd_services() -> Command {
    list_cmd(
        "services",
        "asaas fiscal-info services [--description <text>]",
        "Search the city hall's service list for a municipalServiceId (GET /v3/fiscalInfo/services)",
        r#"Search the municipal services registered for the account's city
(GET /v3/fiscalInfo/services). Use the returned "id" as
'asaas invoice create --municipal-service-id'.

Not available on the Portal Nacional: there, use --municipal-service-code
(see 'fiscal-info federal-service-codes').

Output (stdout): the Asaas list envelope; each item has "id",
"description" and "issTax"."#,
        r#"  asaas fiscal-info services --description "desenvolvimento"
  asaas fiscal-info services --all"#,
        services_filters(),
    )
}

fn cmd_nbs_codes() -> Command {
    list_cmd(
        "nbs-codes",
        "asaas fiscal-info nbs-codes [--search <text>]",
        "Search NBS codes (Nomenclatura Brasileira de Serviços) (GET /v3/fiscalInfo/nbsCodes)",
        r#"Search NBS codes (GET /v3/fiscalInfo/nbsCodes), used in
'fiscal-info save --nbs-code' and 'invoice create --nbs-code'.

Output (stdout): the Asaas list envelope of NBS codes and descriptions."#,
        r#"  asaas fiscal-info nbs-codes --search "software"
  asaas fiscal-info nbs-codes --search 1.1502"#,
        nbs_filters(),
    )
}

fn cmd_federal_service_codes() -> Command {
    list_cmd(
        "federal-service-codes",
        "asaas fiscal-info federal-service-codes [--code <code>] [--description <text>]",
        "Search national service tax codes (código de tributação nacional) (GET /v3/fiscalInfo/federalServiceCodes)",
        r#"Search the national service tax codes (código de tributação nacional,
federalServiceTaxCode) used by the Portal Nacional da NFS-e
(GET /v3/fiscalInfo/federalServiceCodes).

Output (stdout): the Asaas list envelope of codes and descriptions."#,
        r#"  asaas fiscal-info federal-service-codes --description "programação"
  asaas fiscal-info federal-service-codes --code 01.07"#,
        federal_filters(),
    )
}

const CITY_TYPES: &[&str] = &[
    "MY_ACCOUNT_CITY",
    "CUSTOMER_CITY",
    "CUSTOMER_CITY_WHEN_ISS_IS_RETAINED",
];

fn save_fields() -> Vec<Field> {
    vec![
        Field::new("email", "email", "E-mail that receives invoice notifications and alerts from Asaas").body_required(true),
        Field::new("simplesNacional", "simples-nacional", "Whether the company is in the Simples Nacional regime; pass --simples-nacional=false explicitly when not").bool().body_required(true),
        Field::new("municipalInscription", "municipal-inscription", "Company's municipal registration (inscrição municipal)"),
        Field::new("culturalProjectsPromoter", "cultural-projects-promoter", "Whether the company is a cultural incentive promoter (incentivador cultural)").bool(),
        Field::new("cnae", "cnae", "CNAE code"),
        Field::new("specialTaxRegime", "special-tax-regime", "Special tax regime ID; a value from specialTaxRegimesList in 'fiscal-info municipal-options'"),
        Field::new("serviceListItem", "service-list-item", "Service list item per LC 116/2003, when usesServiceListItem is true"),
        Field::new("nbsCode", "nbs-code", "Default NBS code; see 'fiscal-info nbs-codes'"),
        Field::new("rpsSerie", "rps-serie", "RPS series used by the company (some cities require 10001-49999)"),
        Field::new("rpsNumber", "rps-number", "RPS number of the last invoice issued (Asaas continues from the next one)").int(),
        Field::new("loteNumber", "lote-number", "Batch (lote) number of the last invoice issued").int(),
        Field::new("username", "username", "City hall portal username (authenticationType USER_AND_PASSWORD)"),
        Field::new("password", "password", "City hall portal password, or '-' to read it from stdin (USER_AND_PASSWORD)"),
        Field::new("accessToken", "access-token", "City hall portal token, or '-' to read it from stdin (authenticationType TOKEN)"),
        Field::new("certificatePassword", "certificate-password", "Digital certificate password, or '-' to read it from stdin (authenticationType CERTIFICATE)"),
        Field::new("nationalPortalTaxCalculationRegime", "national-portal-tax-calculation-regime", "Simples Nacional tax calculation regime on the Portal Nacional; a value from nationalPortalTaxCalculationRegimeList"),
        Field::new("serviceProvisionCityDefaultType", "service-provision-city-default-type", "Default city where the service is provided; kept as-is when omitted").one_of(CITY_TYPES),
    ]
}

fn cmd_save() -> Command {
    let cmd = cmdutil::new_command(
        "save",
        "asaas fiscal-info save --email <email> --simples-nacional=<bool> [flags]",
        "Create or update the account's fiscal information (POST /v3/fiscalInfo)",
        r#"Create or update the fiscal information used to issue invoices
(POST /v3/fiscalInfo, sent as multipart/form-data).

Run 'asaas fiscal-info municipal-options' first and send only the city hall
credentials matching its authenticationType:
  CERTIFICATE        --certificate-file <file.pfx> --certificate-password <pwd>
  TOKEN              --access-token <token>
  USER_AND_PASSWORD  --username <user> --password <pwd>
Provide --special-tax-regime when usesSpecialTaxRegimes is true and
--service-list-item when usesServiceListItem is true.

--email and --simples-nacional are required on every call. Only flags that
are explicitly set are sent.

Secrets: --password, --access-token and --certificate-password accept '-'
to read the value from stdin (one of them per call), keeping it out of
shell history and the process list.

Output (stdout): the saved fiscal information (secrets are reported only as
passwordSent / accessTokenSent / certificateSent booleans)."#,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas fiscal-info save --email fiscal@empresa.com --simples-nacional=true \
    --municipal-inscription 123456 --cnae 6201501 --rps-serie 1 --rps-number 0

  printf '%s' "$CERT_PWD" | asaas fiscal-info save --email fiscal@empresa.com \
    --simples-nacional=true --certificate-file ./empresa.pfx --certificate-password -"#,
    );
    add_fields(cmd, &save_fields()).arg(string_arg(
        "certificate-file",
        "Path to the digital certificate file (.pfx / .p12) for authenticationType CERTIFICATE [type: string, file path]",
    ))
}

fn run_save(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let fields = save_fields();
    let mut body = collect_body(m, &fields)?;
    check_required(&body, &fields)?;
    read_secrets_from_stdin(f, m, &fields, &mut body)?;

    let form: BTreeMap<String, String> = body
        .into_iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => s,
                other => other.to_string(),
            };
            (k, s)
        })
        .collect();

    let mut files = Vec::new();
    let cert = get_string(m, "certificate-file");
    if !cert.is_empty() {
        let content =
            std::fs::read(&cert).map_err(|e| usage_err!("--certificate-file: open {cert}: {e}"))?;
        let filename = Path::new(&cert)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cert.clone());
        files.push(File {
            field: "certificateFile".into(),
            filename,
            content,
        });
    }

    let client = f.client()?;
    let resp = client.post_multipart(&format!("{BASE_PATH}/"), &form, &files)?;
    f.print_response(&resp)
}

fn read_secrets_from_stdin(
    f: &mut Factory,
    m: &ArgMatches,
    fields: &[Field],
    body: &mut Map<String, Value>,
) -> Result<()> {
    let from_stdin: Vec<&str> = SECRET_FLAGS
        .iter()
        .copied()
        .filter(|name| changed(m, name) && get_string(m, name) == "-")
        .collect();
    match from_stdin.as_slice() {
        [] => Ok(()),
        [flag] => {
            let line = f
                .read_line()
                .map_err(|e| usage_err!("--{flag} -: could not read the value from stdin: {e}"))?;
            if let Some(field) = fields.iter().find(|fl| fl.flag == *flag) {
                body.insert(
                    field.param.into(),
                    Value::String(line.trim_end_matches(['\r', '\n']).to_string()),
                );
            }
            Ok(())
        }
        many => Err(usage_err!(
            "only one secret can be read from stdin per call, got: --{}",
            many.join(", --")
        )),
    }
}
