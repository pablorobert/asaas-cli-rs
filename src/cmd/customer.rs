//! `asaas customer`: the /v3/customers resource.

use clap::{ArgMatches, Command};

use crate::cmdutil::{
    self, Factory, Field, Result, add_fields, bool_arg, collect_body, get_bool, get_string,
    get_strings, merge_json_object, pagination_fields, resource_path, run_list, run_request,
    string_arg,
};
use crate::usage_err;

const BASE_PATH: &str = "/customers";

const DATA_USAGE: &str = r#"Extra request body fields as a JSON object: inline, @file.json or - for stdin. Typed flags override keys present here (e.g. '{"company":"ACME"}') [type: JSON object]"#;

const ID_ARG_DOC: &str = "Arguments:\n  <customer-id>  Asaas customer ID, e.g. cus_000005219613 [type: string] [required]";

pub fn command() -> Command {
    cmdutil::new_group(
        "customer",
        "asaas customer <subcommand>",
        "Manage customers (/v3/customers): list, get, create, update, delete, restore",
        r#"Manage the customers (payers) of the authenticated Asaas account.
Every subcommand calls /v3/customers and prints the raw Asaas JSON response.

Customer IDs look like "cus_000005219613". A customer object contains, among
others: id, name, cpfCnpj, personType (FISICA|JURIDICA), email, phone,
mobilePhone, address, addressNumber, complement, province, postalCode,
city, state, externalReference, notificationDisabled, additionalEmails,
groupName, company, foreignCustomer, deleted, dateCreated."#,
    )
    .subcommands([
        cmd_list(),
        cmd_get(),
        cmd_create(),
        cmd_update(),
        cmd_delete(),
        cmd_restore(),
    ])
}

pub fn run(f: &mut Factory, cmd: &mut Command, m: &ArgMatches) -> Result<()> {
    match m.subcommand() {
        Some(("list", sub)) => run_list(f, sub, BASE_PATH, &list_filters(), get_bool(sub, "all")),
        Some(("get", sub)) => {
            let id = id_arg(sub)?;
            run_request(f, "GET", &resource_path(BASE_PATH, &[&id]), None, None)
        }
        Some(("create", sub)) => {
            let set = collect_body(sub, &body_fields(true))?;
            let body = merge_json_object(f.input, "data", &get_string(sub, "data"), set)?;
            run_request(f, "POST", BASE_PATH, None, Some(body))
        }
        Some(("update", sub)) => {
            let id = id_arg(sub)?;
            let set = collect_body(sub, &body_fields(false))?;
            let body = merge_json_object(f.input, "data", &get_string(sub, "data"), set)?;
            if body.is_empty() {
                return Err(usage_err!(
                    "nothing to update: set at least one field flag or --data"
                ));
            }
            run_request(
                f,
                "PUT",
                &resource_path(BASE_PATH, &[&id]),
                None,
                Some(body),
            )
        }
        Some(("delete", sub)) => {
            let id = id_arg(sub)?;
            run_request(f, "DELETE", &resource_path(BASE_PATH, &[&id]), None, None)
        }
        Some(("restore", sub)) => {
            let id = id_arg(sub)?;
            run_request(
                f,
                "POST",
                &resource_path(BASE_PATH, &[&id, "restore"]),
                None,
                None,
            )
        }
        _ => cmdutil::print_group_help(f, cmd),
    }
}

/// The writable customer attributes shared by create and update.
fn body_fields(require_identity: bool) -> Vec<Field> {
    vec![
        Field::new("name", "name", "Customer full name or company name").required(require_identity),
        Field::new(
            "cpfCnpj",
            "cpf-cnpj",
            "CPF (11 digits) or CNPJ (14 digits), digits only",
        )
        .required(require_identity),
        Field::new("email", "email", "Primary e-mail address"),
        Field::new(
            "phone",
            "phone",
            "Landline phone with area code, digits only (e.g. 4738010919)",
        ),
        Field::new(
            "mobilePhone",
            "mobile-phone",
            "Mobile phone with area code, digits only (e.g. 47999376637)",
        ),
        Field::new("address", "address", "Street name"),
        Field::new("addressNumber", "address-number", "Street number"),
        Field::new(
            "complement",
            "complement",
            "Address complement (apartment, suite...)",
        ),
        Field::new("province", "province", "Neighborhood (bairro)"),
        Field::new(
            "postalCode",
            "postal-code",
            "Brazilian postal code (CEP), 8 digits",
        ),
        Field::new(
            "externalReference",
            "external-reference",
            "Your own identifier for this customer",
        ),
        Field::new(
            "notificationDisabled",
            "notification-disabled",
            "Disable Asaas e-mail/SMS notifications to this customer",
        )
        .bool(),
        Field::new(
            "additionalEmails",
            "additional-emails",
            "Extra e-mails that receive notifications, comma-separated (e.g. a@x.com,b@y.com)",
        ),
        Field::new(
            "municipalInscription",
            "municipal-inscription",
            "Municipal registration number (inscrição municipal)",
        ),
        Field::new(
            "stateInscription",
            "state-inscription",
            "State registration number (inscrição estadual)",
        ),
        Field::new("observations", "observations", "Free-text notes"),
        Field::new("groupName", "group-name", "Customer group name"),
        Field::new(
            "company",
            "company",
            "Company name (for individuals linked to a company)",
        ),
        Field::new(
            "foreignCustomer",
            "foreign-customer",
            "Customer is not Brazilian (no CPF/CNPJ validation)",
        )
        .bool(),
    ]
}

fn list_filters() -> Vec<Field> {
    let mut f = vec![
        Field::new("name", "name", "Filter by customer name (partial match)"),
        Field::new("email", "email", "Filter by e-mail address"),
        Field::new(
            "cpfCnpj",
            "cpf-cnpj",
            "Filter by CPF (11 digits) or CNPJ (14 digits), digits only",
        ),
        Field::new("groupName", "group-name", "Filter by customer group name"),
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
            "expected exactly one <customer-id> argument (e.g. cus_000005219613), got {}",
            args.len()
        )),
    }
}

fn with_id(cmd: Command) -> Command {
    cmd.arg(cmdutil::positional_args())
}

fn cmd_list() -> Command {
    let cmd = cmdutil::new_command(
        "list",
        "asaas customer list [flags]",
        "List customers, with optional filters and pagination (GET /v3/customers)",
        r#"List the customers of the authenticated account (GET /v3/customers).

Filters are combined with AND. Results are paginated by the API with
--offset/--limit (max 100 per page); --all follows "hasMore" and returns
every matching customer in a single list (ignores --limit).

Output (stdout), the Asaas list envelope:
  {"object": "list", "hasMore": bool, "totalCount": int, "limit": int,
   "offset": int, "data": [<customer>, ...]}"#,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas customer list
  asaas customer list --limit 100 --offset 200
  asaas customer list --cpf-cnpj 24971563792
  asaas customer list --name "Marcelo" --all
  ASAAS_API_KEY="$KEY" asaas customer list --sandbox"#,
    );
    add_fields(cmd, &list_filters()).arg(bool_arg(
        "all",
        "Fetch every page and return all matching customers in one list [type: boolean]",
    ))
}

fn cmd_get() -> Command {
    let long = format!(
        "Retrieve a single customer (GET /v3/customers/{{id}}).\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the customer object. Deleted customers are still returned,\n\
with \"deleted\": true. Unknown IDs return api_error with status_code 404."
    );
    let cmd = cmdutil::new_command(
        "get",
        "asaas customer get <customer-id>",
        "Retrieve one customer by ID (GET /v3/customers/{id})",
        long,
    );
    with_id(cmdutil::with_examples(
        cmd,
        "  asaas customer get cus_000005219613",
    ))
}

fn cmd_create() -> Command {
    let cmd = cmdutil::new_command(
        "create",
        "asaas customer create --name <name> --cpf-cnpj <digits> [flags]",
        "Create a customer (POST /v3/customers)",
        r#"Create a customer (POST /v3/customers). Only flags that are explicitly set
are sent. Fields without a dedicated flag can be passed with --data.

Note: Asaas does not prevent duplicates; run 'asaas customer list --cpf-cnpj'
first if a customer with the same document may already exist.

Output (stdout): the created customer object, including its new "id"."#,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas customer create --name "Marcelo Almeida" --cpf-cnpj 24971563792
  asaas customer create --name "ACME Ltda" --cpf-cnpj 12345678000195 \
    --email financeiro@acme.com --mobile-phone 47999376637 --postal-code 01310000
  asaas customer create --name "Ana" --cpf-cnpj 24971563792 --data @extra.json"#,
    );
    add_fields(cmd, &body_fields(true)).arg(string_arg("data", DATA_USAGE))
}

fn cmd_update() -> Command {
    let long = format!(
        "Update a customer (PUT /v3/customers/{{id}}). Only flags that are explicitly\n\
set are sent, so unspecified fields keep their current values. At least one\n\
field flag or --data is required.\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the updated customer object."
    );
    let cmd = cmdutil::new_command(
        "update",
        "asaas customer update <customer-id> [flags]",
        "Update fields of an existing customer (PUT /v3/customers/{id})",
        long,
    );
    let cmd = cmdutil::with_examples(
        cmd,
        r#"  asaas customer update cus_000005219613 --email novo@email.com
  asaas customer update cus_000005219613 --notification-disabled=true
  asaas customer update cus_000005219613 --data '{"observations":"VIP"}'"#,
    );
    with_id(add_fields(cmd, &body_fields(false)).arg(string_arg("data", DATA_USAGE)))
}

fn cmd_delete() -> Command {
    let long = format!(
        "Delete a customer (DELETE /v3/customers/{{id}}). Asaas performs a soft delete:\n\
the customer is flagged \"deleted\": true and can be brought back with\n\
'asaas customer restore'. Runs immediately, with no confirmation prompt.\n\n{ID_ARG_DOC}\n\n\
Output (stdout): {{\"deleted\": true, \"id\": \"<customer-id>\"}}"
    );
    let cmd = cmdutil::new_command(
        "delete",
        "asaas customer delete <customer-id>",
        "Delete a customer (DELETE /v3/customers/{id}); reversible with restore",
        long,
    );
    with_id(cmdutil::with_examples(
        cmd,
        "  asaas customer delete cus_000005219613",
    ))
}

fn cmd_restore() -> Command {
    let long = format!(
        "Restore a previously deleted customer (POST /v3/customers/{{id}}/restore).\n\n{ID_ARG_DOC}\n\n\
Output (stdout): the restored customer object."
    );
    let cmd = cmdutil::new_command(
        "restore",
        "asaas customer restore <customer-id>",
        "Restore a deleted customer (POST /v3/customers/{id}/restore)",
        long,
    );
    with_id(cmdutil::with_examples(
        cmd,
        "  asaas customer restore cus_000005219613",
    ))
}
