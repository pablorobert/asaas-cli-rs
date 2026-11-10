//! `asaas invoice`: service invoices (NFS-e) through /v3/invoices.

use clap::{ArgMatches, Command};
use serde_json::{Map, Value, json};

use crate::client::{Client, ResponseError};
use crate::cmdutil::{
    self, Factory, Field, Result, add_fields, bool_arg, changed, check_required, collect_body,
    deep_merge, get_bool, get_int, get_string, get_strings, has_path, merge_json_object,
    pagination_fields, require_one_of, resource_path, run_list, run_request, string_arg,
};
use crate::usage_err;

const BASE_PATH: &str = "/invoices";

/// Statuses accepted by the GET /v3/invoices "status" filter.
const STATUSES: &[&str] = &[
    "SCHEDULED",
    "AUTHORIZED",
    "PROCESSING_CANCELLATION",
    "CANCELED",
    "CANCELLATION_DENIED",
    "ERROR",
];

/// The keys Asaas requires whenever a "taxes" object is sent.
const TAX_REQUIRED: &[&str] = &["retainIss", "iss", "pis", "cofins", "csll", "inss", "ir"];

const DATA_USAGE: &str = r#"Extra request body fields as a JSON object: inline, @file.json or - for stdin. Deep-merged under the typed flags (flags win). Use it for taxes.realEstateInfo and taxes.referencedInvoiceAccessKeyList, e.g. '{"taxes":{"referencedInvoiceAccessKeyList":["<50 digits>"]}}' [type: JSON object]"#;

const ID_ARG_DOC: &str =
    "Arguments:\n  <invoice-id>  Asaas invoice ID, e.g. inv_000000000232 [type: string] [required]";

pub fn command() -> Command {
    cmdutil::new_group(
        "invoice",
        "asaas invoice <subcommand>",
        "Manage service invoices / NFS-e (/v3/invoices): list, get, create, update, authorize, cancel",
        r#"Manage service invoices (Nota Fiscal de Serviço eletrônica, NFS-e).

Lifecycle:
  1. 'invoice create' schedules an invoice for its effectiveDate (status SCHEDULED).
  2. On that date Asaas sends it to the city hall / Portal Nacional; use
     'invoice authorize' to send a scheduled invoice right away.
  3. The invoice becomes AUTHORIZED (or ERROR). Authorization is asynchronous:
     poll 'invoice get' or use the INVOICE_AUTHORIZED / INVOICE_ERROR webhooks.
  4. 'invoice cancel' requests cancellation (PROCESSING_CANCELLATION, then
     CANCELED or CANCELLATION_DENIED).

Prerequisite: the account's fiscal information must be configured
(see 'asaas fiscal-info'). Use 'asaas fiscal-info services' to find the
municipalServiceId, or pass --municipal-service-code when the city has no
service list (always the case on the Portal Nacional).

Invoice IDs look like "inv_000000000232". An invoice object contains, among
others: id, status, customer, payment, installment, type, statusDescription,
serviceDescription, pdfUrl, xmlUrl, rpsSerie, rpsNumber, number,
validationCode, value, deductions, effectiveDate, observations,
estimatedTaxesDescription, externalReference, taxes, municipalServiceId,
municipalServiceCode, municipalServiceName."#,
    )
    .subcommands([cmd_list(), cmd_get(), cmd_create(), cmd_update(), cmd_authorize(), cmd_cancel()])
}

pub fn run(f: &mut Factory, cmd: &mut Command, m: &ArgMatches) -> Result<()> {
    match m.subcommand() {
        Some(("list", sub)) => run_list(f, sub, BASE_PATH, &list_filters(), get_bool(sub, "all")),
        Some(("get", sub)) => {
            let id = id_arg(sub)?;
            run_request(f, "GET", &resource_path(BASE_PATH, &[&id]), None, None)
        }
        Some(("create", sub)) => run_create(f, sub),
        Some(("update", sub)) => run_update(f, sub),
        Some(("authorize", sub)) => {
            let id = id_arg(sub)?;
            run_request(
                f,
                "POST",
                &resource_path(BASE_PATH, &[&id, "authorize"]),
                None,
                None,
            )
        }
        Some(("cancel", sub)) => {
            let id = id_arg(sub)?;
            let mut body = Map::new();
            if changed(sub, "only-on-asaas") {
                body.insert(
                    "cancelOnlyOnAsaas".into(),
                    json!(get_bool(sub, "only-on-asaas")),
                );
            }
            run_request(
                f,
                "POST",
                &resource_path(BASE_PATH, &[&id, "cancel"]),
                None,
                Some(body),
            )
        }
        _ => cmdutil::print_group_help(f, cmd),
    }
}

/// Fields of the nested "taxes" object. Rates are percentages (2.5 means
/// 2.5%). required controls whether the mandatory ones must be present in
/// the final body (create) or not (update).
fn tax_fields(required: bool) -> Vec<Field> {
    vec![
        Field::new("taxes.retainIss", "retain-iss", "Whether the customer (tomador) withholds the ISS; pass --retain-iss=false explicitly when not").bool().body_required(required),
        Field::new("taxes.iss", "iss", "ISS rate in percent (e.g. 2.5 for 2.5%)").float().body_required(required),
        Field::new("taxes.pis", "pis", "PIS withholding rate in percent; 0 when not withheld").float().body_required(required),
        Field::new("taxes.cofins", "cofins", "COFINS withholding rate in percent; 0 when not withheld").float().body_required(required),
        Field::new("taxes.csll", "csll", "CSLL withholding rate in percent; 0 when not withheld").float().body_required(required),
        Field::new("taxes.inss", "inss", "INSS withholding rate in percent; 0 when not withheld").float().body_required(required),
        Field::new("taxes.ir", "ir", "IR withholding rate in percent; 0 when not withheld").float().body_required(required),
        Field::new("taxes.nbsCode", "nbs-code", "NBS code (Nomenclatura Brasileira de Serviços); see 'asaas fiscal-info nbs-codes'"),
        Field::new("taxes.taxSituationCode", "tax-situation-code", "Tax situation code (código de situação tributária, tax reform)"),
        Field::new("taxes.taxClassificationCode", "tax-classification-code", "Tax classification code (código de classificação tributária, tax reform)"),
        Field::new("taxes.operationIndicatorCode", "operation-indicator-code", "Operation indicator code (código do indicador de operação, tax reform), e.g. 020101"),
        Field::new("taxes.pisCofinsTaxStatus", "pis-cofins-tax-status", "PIS/COFINS tax status (situação tributária do PIS/COFINS)"),
        Field::new("taxes.operationPis", "operation-pis", "Operation PIS rate in percent (contributor's own assessment)").float(),
        Field::new("taxes.operationCofins", "operation-cofins", "Operation COFINS rate in percent (contributor's own assessment)").float(),
        Field::new("taxes.operationTypeCode", "operation-type-code", "Tax-reform operation type, 1-5; only for real-estate services (federal tax codes 25.05, 15.09, 17.12, 10.05)").int(),
    ]
}

fn create_fields() -> Vec<Field> {
    let mut f = vec![
        Field::new(
            "payment",
            "payment",
            "Payment ID (pay_...) the invoice refers to; one of --payment, --installment or --customer is required",
        ),
        Field::new(
            "installment",
            "installment",
            "Installment ID the invoice refers to",
        ),
        Field::new(
            "customer",
            "customer",
            "Customer ID (cus_...) for a standalone invoice not tied to a payment",
        ),
        Field::new(
            "serviceDescription",
            "service-description",
            "Description of the services rendered, printed on the invoice",
        )
        .body_required(true),
        Field::new(
            "observations",
            "observations",
            "Additional notes printed on the invoice",
        )
        .body_required(true),
        Field::new(
            "value",
            "value",
            "Total invoice amount in BRL (e.g. 300.00)",
        )
        .float()
        .body_required(true),
        Field::new(
            "deductions",
            "deductions",
            "Deductions in BRL; lower the ISS base but not the total; defaults to 0 when omitted",
        )
        .float(),
        Field::new(
            "effectiveDate",
            "effective-date",
            "Issue date; the invoice is sent to the city hall on this date",
        )
        .date()
        .body_required(true),
        Field::new(
            "municipalServiceId",
            "municipal-service-id",
            "Municipal service ID from 'asaas fiscal-info services'; one of --municipal-service-id or --municipal-service-code is required",
        ),
        Field::new(
            "municipalServiceCode",
            "municipal-service-code",
            "Municipal service code (required on the Portal Nacional, where no service list exists)",
        ),
        Field::new(
            "municipalServiceName",
            "municipal-service-name",
            "Municipal service name; when omitted Asaas uses the service code as the name",
        ),
        Field::new(
            "externalReference",
            "external-reference",
            "Your own identifier for this invoice",
        ),
        Field::new(
            "updatePayment",
            "update-payment",
            "Reduce the linked payment's value by the withheld taxes",
        )
        .bool(),
    ];
    f.extend(tax_fields(true));
    f
}

fn update_fields() -> Vec<Field> {
    let mut f = vec![
        Field::new(
            "serviceDescription",
            "service-description",
            "Description of the services rendered",
        ),
        Field::new(
            "observations",
            "observations",
            "Additional notes printed on the invoice",
        ),
        Field::new("value", "value", "Total invoice amount in BRL").float(),
        Field::new(
            "deductions",
            "deductions",
            "Deductions in BRL (lower the ISS base, not the total)",
        )
        .float(),
        Field::new("effectiveDate", "effective-date", "New issue date").date(),
        Field::new(
            "externalReference",
            "external-reference",
            "Your own identifier for this invoice",
        ),
        Field::new(
            "updatePayment",
            "update-payment",
            "Reduce the linked payment's value by the withheld taxes",
        )
        .bool(),
    ];
    f.extend(tax_fields(false));
    f
}

fn list_filters() -> Vec<Field> {
    let mut f = vec![
        Field::new(
            "effectiveDate[ge]",
            "effective-date-from",
            "Only invoices with effectiveDate on or after this date",
        )
        .date(),
        Field::new(
            "effectiveDate[le]",
            "effective-date-to",
            "Only invoices with effectiveDate on or before this date",
        )
        .date(),
        Field::new("status", "status", "Filter by status").one_of(STATUSES),
        Field::new("customer", "customer", "Filter by customer ID (cus_...)"),
        Field::new("payment", "payment", "Filter by payment ID (pay_...)"),
        Field::new("installment", "installment", "Filter by installment ID"),
        Field::new(
            "externalReference",
            "external-reference",
            "Filter by your external reference",
        ),
    ];
    f.extend(pagination_fields());
    f
}

fn id_arg(m: &ArgMatches) -> Result<String> {
    match get_strings(m, "args").as_slice() {
        [id] if !id.is_empty() => Ok(id.clone()),
        args => Err(usage_err!(
            "expected exactly one <invoice-id> argument (e.g. inv_000000000232), got {}",
            args.len()
        )),
    }
}

/// Checks taxes.operationTypeCode bounds when set by flag.
fn validate_operation_type(m: &ArgMatches) -> Result<()> {
    if changed(m, "operation-type-code")
        && let Some(v) = get_int(m, "operation-type-code")
        && !(1..=5).contains(&v)
    {
        return Err(usage_err!(
            "--operation-type-code must be between 1 and 5, got {v}"
        ));
    }
    Ok(())
}

fn cmd_list() -> Command {
    let cmd = cmdutil::new_command(
        "list",
        "asaas invoice list [flags]",
        "List invoices, with optional filters and pagination (GET /v3/invoices)",
        r#"List the service invoices of the authenticated account (GET /v3/invoices).

Filters are combined with AND. Results are paginated by the API with
--offset/--limit (max 100 per page); --all follows "hasMore" and returns
every matching invoice in a single list (ignores --limit).

Output (stdout), the Asaas list envelope:
  {"object": "list", "hasMore": bool, "totalCount": int, "limit": int,
   "offset": int, "data": [<invoice>, ...]}"#,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas invoice list --status AUTHORIZED --limit 20
  asaas invoice list --effective-date-from 2026-10-01 --effective-date-to 2026-10-31 --all
  asaas invoice list --customer cus_000005219613
  asaas invoice list --payment pay_080225913252"#,
    );
    add_fields(cmd, &list_filters()).arg(bool_arg(
        "all",
        "Fetch every page and return all matching invoices in one list [type: boolean]",
    ))
}

fn cmd_get() -> Command {
    let long = format!(
        "Retrieve a single service invoice (GET /v3/invoices/{{id}}).\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the invoice object. Check \"status\" (SCHEDULED, AUTHORIZED,\n\
PROCESSING_CANCELLATION, CANCELED, CANCELLATION_DENIED, ERROR), and\n\
\"statusDescription\" for the city hall's message when it is ERROR. Once\n\
AUTHORIZED, \"pdfUrl\" and \"xmlUrl\" point to the issued document."
    );
    let cmd = cmdutil::new_command(
        "get",
        "asaas invoice get <invoice-id>",
        "Retrieve one invoice by ID (GET /v3/invoices/{id})",
        long,
    );
    cmdutil::with_examples(cmd, "  asaas invoice get inv_000000000232")
        .arg(cmdutil::positional_args())
}

fn cmd_authorize() -> Command {
    let long = format!(
        "Send a scheduled invoice to the city hall / Portal Nacional immediately,\n\
instead of waiting for its effectiveDate (POST /v3/invoices/{{id}}/authorize).\n\n\
This issues a legal fiscal document and runs with no confirmation prompt.\n\
Authorization is asynchronous: the response may still show a processing\n\
status; poll 'asaas invoice get' until AUTHORIZED or ERROR, or listen to the\n\
INVOICE_AUTHORIZED / INVOICE_ERROR webhooks.\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the invoice object."
    );
    let cmd = cmdutil::new_command(
        "authorize",
        "asaas invoice authorize <invoice-id>",
        "Issue a scheduled invoice now (POST /v3/invoices/{id}/authorize)",
        long,
    );
    cmdutil::with_examples(cmd, "  asaas invoice authorize inv_000000000232")
        .arg(cmdutil::positional_args())
}

fn cmd_cancel() -> Command {
    let long = format!(
        "Request cancellation of a service invoice (POST /v3/invoices/{{id}}/cancel).\n\
Runs with no confirmation prompt.\n\n\
For an issued (AUTHORIZED) invoice, Asaas asks the city hall to cancel it.\n\
Whether that is automatic depends on the city: check \"supportsCancellation\"\n\
in 'asaas fiscal-info municipal-options'. The request may stay in\n\
PROCESSING_CANCELLATION before becoming CANCELED or CANCELLATION_DENIED\n\
(webhooks INVOICE_CANCELED / INVOICE_CANCELLATION_DENIED).\n\n\
--only-on-asaas cancels the record in Asaas only, without contacting the\n\
city hall (e.g. after cancelling it manually on the city's portal).\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the invoice object."
    );
    let cmd = cmdutil::new_command(
        "cancel",
        "asaas invoice cancel <invoice-id> [--only-on-asaas]",
        "Cancel an invoice (POST /v3/invoices/{id}/cancel)",
        long,
    );
    cmdutil::with_examples(
        cmd,
        r#"  asaas invoice cancel inv_000000000232
  asaas invoice cancel inv_000000000232 --only-on-asaas"#,
    )
    .arg(cmdutil::positional_args())
    .arg(bool_arg(
        "only-on-asaas",
        "Cancel only in Asaas, without asking the city hall to cancel the issued NFS-e [type: boolean]",
    ))
}

fn cmd_create() -> Command {
    let cmd = cmdutil::new_command(
        "create",
        "asaas invoice create --service-description <text> --value <amount> --effective-date <YYYY-MM-DD> [flags]",
        "Schedule a service invoice / NFS-e (POST /v3/invoices)",
        r#"Create a service invoice and schedule it for --effective-date
(POST /v3/invoices). It is sent to the city hall / Portal Nacional on that
date; run 'asaas invoice authorize' to send it earlier.

Required (each via its flag or the same key in --data):
  - origin: one of --payment, --installment or --customer
  - service: one of --municipal-service-id or --municipal-service-code
  - --service-description, --observations, --value, --effective-date
  - taxes: --retain-iss, --iss, --pis, --cofins, --csll, --inss, --ir
    (rates in percent; use 0 for taxes that are not withheld)
--deductions defaults to 0. Missing fields are reported together as a
usage_error before any request is sent.

Real-estate services (tax reform) also need taxes.realEstateInfo and/or
taxes.referencedInvoiceAccessKeyList; pass them with --data.

Output (stdout): the created invoice object (status SCHEDULED), with its "id"."#,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  # Invoice for a received payment, Simples Nacional company, 2% ISS, no withholdings
  asaas invoice create --payment pay_080225913252 \
    --service-description "Consultoria em TI - outubro/2026" --observations "Mensal" \
    --value 300 --effective-date 2026-10-31 --municipal-service-id 60380 \
    --retain-iss=false --iss 2 --pis 0 --cofins 0 --csll 0 --inss 0 --ir 0

  # Standalone invoice on the Portal Nacional, taxes from a file
  asaas invoice create --customer cus_000005219613 --service-description "Desenvolvimento" \
    --observations "-" --value 1500 --effective-date 2026-11-05 \
    --municipal-service-code 01.07.01 --data @taxes.json"#,
    );
    add_fields(cmd, &create_fields()).arg(string_arg("data", DATA_USAGE))
}

fn run_create(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    validate_operation_type(m)?;
    let fields = create_fields();
    let set = collect_body(m, &fields)?;
    let mut body = merge_json_object(f.input, "data", &get_string(m, "data"), set)?;
    if !has_path(&body, "deductions") {
        body.insert("deductions".into(), json!(0));
    }
    require_one_of(&body, &fields, &["payment", "installment", "customer"])?;
    require_one_of(
        &body,
        &fields,
        &["municipalServiceId", "municipalServiceCode"],
    )?;
    check_required(&body, &fields)?;
    run_request(f, "POST", BASE_PATH, None, Some(body))
}

fn cmd_update() -> Command {
    let long = format!(
        "Update an invoice that has not been issued yet (PUT /v3/invoices/{{id}}).\n\
Only flags that are explicitly set are sent. At least one field flag or\n\
--data is required. Origin (payment/customer) and municipal service cannot\n\
be changed; cancel and create a new invoice instead.\n\n\
Asaas requires the full set of mandatory taxes (retainIss, iss, pis, cofins,\n\
csll, inss, ir) whenever \"taxes\" is sent. If you change only some of them\n\
(e.g. just --iss), the CLI first reads the invoice and fills the missing\n\
ones with their current values.\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the updated invoice object."
    );
    let cmd = cmdutil::new_command(
        "update",
        "asaas invoice update <invoice-id> [flags]",
        "Update a scheduled invoice (PUT /v3/invoices/{id})",
        long,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas invoice update inv_000000000232 --effective-date 2026-11-10
  asaas invoice update inv_000000000232 --iss 3.5
  asaas invoice update inv_000000000232 --value 450 --observations "Valor revisado""#,
    );
    add_fields(cmd, &update_fields())
        .arg(string_arg("data", DATA_USAGE))
        .arg(cmdutil::positional_args())
}

fn run_update(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let id = id_arg(m)?;
    validate_operation_type(m)?;
    let set = collect_body(m, &update_fields())?;
    let mut body = merge_json_object(f.input, "data", &get_string(m, "data"), set)?;
    if body.is_empty() {
        return Err(usage_err!(
            "nothing to update: set at least one field flag or --data"
        ));
    }

    let client = f.client()?;
    let path = resource_path(BASE_PATH, &[&id]);
    complete_taxes(&client, &path, &mut body)?;
    let resp = client.put(&path, Value::Object(body))?;
    f.print_response(&resp)
}

/// Fills mandatory tax keys missing from body["taxes"] with the invoice's
/// current values, since Asaas rejects a partial taxes object.
fn complete_taxes(client: &Client, path: &str, body: &mut Map<String, Value>) -> Result<()> {
    let Some(Value::Object(taxes)) = body.get("taxes") else {
        return Ok(());
    };
    if TAX_REQUIRED.iter().all(|k| taxes.contains_key(*k)) {
        return Ok(());
    }

    let resp = client.get(path, None)?;
    let bad = |msg: &str| {
        crate::client::Error::Response(ResponseError {
            status_code: resp.status_code,
            message: format!("reading current invoice taxes: {msg}"),
            body: String::new(),
        })
    };
    let current = match &resp.body {
        Some(Value::Object(inv)) => match inv.get("taxes") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(t)) => t.clone(),
            Some(_) => return Err(bad("\"taxes\" is not an object").into()),
        },
        Some(_) => return Err(bad("response is not a JSON object").into()),
        None => return Err(bad("empty body").into()),
    };

    let mut merged = Map::new();
    for k in TAX_REQUIRED {
        if let Some(v) = current.get(*k).filter(|v| !v.is_null()) {
            merged.insert((*k).to_string(), v.clone());
        }
    }
    if let Some(Value::Object(taxes)) = body.remove("taxes") {
        deep_merge(&mut merged, taxes);
    }
    body.insert("taxes".into(), Value::Object(merged));
    Ok(())
}
