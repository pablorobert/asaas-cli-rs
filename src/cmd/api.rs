//! `asaas api`: raw authenticated requests to any Asaas v3 endpoint, in the
//! spirit of `gh api`.

use clap::{ArgMatches, Command};
use serde_json::{Map, Value};

use crate::client::{Body, Query};
use crate::cmdutil::{
    self, Factory, Result, bool_arg, get_bool, get_string, get_strings, merge_json_object,
    parse_key_value, read_json_arg, string_arg, string_list_arg, typed_value,
};
use crate::usage_err;

const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH"];

pub fn command() -> Command {
    let cmd = cmdutil::new_command(
        "api",
        "asaas api <path> [flags]",
        "Send an authenticated request to any Asaas API v3 endpoint",
        r#"Send an authenticated request to any Asaas API v3 endpoint and print the
JSON response. Use it for resources that have no dedicated command yet
(payments, subscriptions, pix, transfers, webhooks, ...).

Arguments:
  <path>  Endpoint path relative to /v3, e.g. /payments or payments/pay_123.
          A leading "/v3" is accepted and stripped. Absolute URLs are rejected
          so the API key is only ever sent to the Asaas host. [type: string] [required]

Request body (POST/PUT/PATCH):
  --data       a JSON document (inline, @file.json, or - for stdin)
  -F/--field   key=value with JSON typing: true, false, null and valid JSON
               numbers become literals ("0123" stays a string)
  -f/--raw-field key=value, always sent as a string (use for CPF/CNPJ, CEP...)
  Fields are merged on top of --data, which must then be a JSON object.

Output (stdout): the raw Asaas response body, or
{"status_code": N, "success": true} when the API returns no body."#,
    );
    cmdutil::with_examples(
        cmd,
        r#"  asaas api /payments --query status=PENDING --query limit=20
  asaas api /payments -X POST -f customer=cus_000005219613 -f billingType=PIX \
    -F value=129.90 -f dueDate=2026-12-31
  asaas api /subscriptions -X POST --data @subscription.json
  asaas api /finance/balance
  asaas api /payments --all --query customer=cus_000005219613"#,
    )
    .arg(cmdutil::positional_args())
    .arg(string_arg("method", "HTTP method: GET, POST, PUT, DELETE or PATCH [type: string]").short('X').default_value("GET"))
    .arg(string_list_arg("query", "Query parameter as key=value; repeatable [type: string, key=value]").short('q'))
    .arg(string_list_arg("field", "Body field as key=value with JSON typing (true/false/null/numbers); repeatable [type: string, key=value]").short('F'))
    .arg(string_list_arg("raw-field", "Body field as key=value, always a JSON string; repeatable [type: string, key=value]").short('f'))
    .arg(string_arg("data", "Request body as JSON: inline, @file.json, or - for stdin [type: JSON]").short('d'))
    .arg(bool_arg("all", "GET only: follow pagination (hasMore) and return every item of a list endpoint in one list [type: boolean]"))
}

pub fn run(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let args = get_strings(m, "args");
    if args.len() != 1 {
        return Err(usage_err!(
            "expected exactly one <path> argument (e.g. /payments), got {}",
            args.len()
        ));
    }
    let path = &args[0];

    let raw_method = get_string(m, "method");
    let method = raw_method.to_uppercase();
    if !ALLOWED_METHODS.contains(&method.as_str()) {
        return Err(usage_err!(
            "--method: unsupported HTTP method {raw_method:?}"
        ));
    }

    let mut query = Query::new();
    for kv in get_strings(m, "query") {
        let (k, v) = parse_key_value("query", &kv)?;
        query.add(k, v);
    }

    let mut fields = Map::new();
    for kv in get_strings(m, "field") {
        let (k, v) = parse_key_value("field", &kv)?;
        fields.insert(k, typed_value(&v));
    }
    for kv in get_strings(m, "raw-field") {
        let (k, v) = parse_key_value("raw-field", &kv)?;
        fields.insert(k, Value::String(v));
    }

    let data = get_string(m, "data");
    let all = get_bool(m, "all");
    let has_body = !data.is_empty() || !fields.is_empty();
    if has_body && (method == "GET" || method == "DELETE") {
        return Err(usage_err!(
            "{method} requests take no body; use --query for parameters"
        ));
    }
    if all && method != "GET" {
        return Err(usage_err!("--all can only be used with GET"));
    }

    let body = if !fields.is_empty() {
        Some(Body::Json(Value::Object(merge_json_object(
            f.input, "data", &data, fields,
        )?)))
    } else if !data.is_empty() {
        Some(Body::Raw(read_json_arg(f.input, "data", &data)?))
    } else {
        None
    };

    let client = f.client()?;
    if all {
        let page = client.list_all(path, &query)?;
        return f.print(&page);
    }
    let resp = client.request(&method, path, Some(&query), body)?;
    f.print_response(&resp)
}
