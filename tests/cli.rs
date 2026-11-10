//! End-to-end tests of the command tree against a local fake Asaas server.
//! No API key or network access is needed.

use std::sync::{Arc, Mutex};
use std::thread;

use asaas_cli::cmd;
use asaas_cli::cmdutil::{EXIT_AUTH, EXIT_ERROR, EXIT_USAGE, Factory, GlobalFlags};
use asaas_cli::config::{Env, SOURCE_FILE};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone)]
struct Request {
    method: String,
    path: String,
    query: String,
    token: String,
    body: String,
}

/// Records requests and replies with canned Asaas responses.
struct FakeAsaas {
    reqs: Arc<Mutex<Vec<Request>>>,
    server: Arc<tiny_http::Server>,
    url: String,
}

impl Drop for FakeAsaas {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

fn header(req: &tiny_http::Request, name: &str) -> String {
    req.headers()
        .iter()
        .find(|h| h.field.to_string().eq_ignore_ascii_case(name))
        .map(|h| h.value.to_string())
        .unwrap_or_default()
}

/// Decodes a multipart/form-data body into {"field": "value"} and
/// {"file-field": "filename:content"}.
fn parse_multipart(content_type: &str, body: &[u8]) -> Map<String, Value> {
    let boundary = content_type.split("boundary=").nth(1).unwrap().trim();
    let body = String::from_utf8_lossy(body);
    let mut out = Map::new();
    for part in body.split(&format!("--{boundary}")) {
        let Some((headers, content)) = part.split_once("\r\n\r\n") else {
            continue;
        };
        let content = content.strip_suffix("\r\n").unwrap_or(content);
        let attr = |key: &str| {
            headers
                .split(&format!("{key}=\""))
                .nth(1)
                .and_then(|s| s.split('"').next())
                .map(str::to_string)
        };
        let Some(name) = attr("name") else { continue };
        let value = match attr("filename") {
            Some(filename) => format!("{filename}:{content}"),
            None => content.to_string(),
        };
        out.insert(name, json!(value));
    }
    out
}

impl FakeAsaas {
    fn new() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let url = format!("http://{}/v3", server.server_addr().to_ip().unwrap());
        let reqs = Arc::new(Mutex::new(Vec::new()));
        let (s, r) = (server.clone(), reqs.clone());
        thread::spawn(move || {
            for mut req in s.incoming_requests() {
                let mut raw = Vec::new();
                let _ = req.as_reader().read_to_end(&mut raw);
                let (path, query) = match req.url().split_once('?') {
                    Some((p, q)) => (p.to_string(), q.to_string()),
                    None => (req.url().to_string(), String::new()),
                };
                let method = req.method().as_str().to_string();
                let token = header(&req, "access_token");
                let content_type = header(&req, "Content-Type");
                let body = String::from_utf8_lossy(&raw).into_owned();
                r.lock().unwrap().push(Request {
                    method: method.clone(),
                    path: path.clone(),
                    query,
                    token: token.clone(),
                    body: body.clone(),
                });

                let (status, out) = if token == "bad" {
                    (
                        401,
                        r#"{"errors":[{"code":"invalid_access_token","description":"invalid"}]}"#
                            .to_string(),
                    )
                } else if method == "GET" && path == "/v3/customers" {
                    (200, r#"{"object":"list","hasMore":false,"totalCount":1,"limit":10,"offset":0,"data":[{"id":"cus_1","name":"Ana","invoiceUrl":"https://x.test/?a=1&b=2"}]}"#.to_string())
                } else if method == "GET" && path == "/v3/invoices/inv_1" {
                    (200, r#"{"id":"inv_1","status":"SCHEDULED","taxes":{"retainIss":true,"iss":2,"pis":0.65,"cofins":3,"csll":1,"inss":0,"ir":1.5,"pisCofinsRetentionType":"WITHHELD","nbsCode":null}}"#.to_string())
                } else if content_type.starts_with("multipart/form-data") {
                    (
                        200,
                        Value::Object(parse_multipart(&content_type, &raw)).to_string(),
                    )
                } else if method == "DELETE" {
                    (200, r#"{"deleted":true,"id":"cus_1"}"#.to_string())
                } else if method == "POST" && path.ends_with("/empty") {
                    (204, String::new())
                } else {
                    (200, body)
                };
                let _ = req.respond(tiny_http::Response::from_string(out).with_status_code(status));
            }
        });
        FakeAsaas { reqs, server, url }
    }

    fn last(&self) -> Request {
        self.reqs
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("no request received")
    }

    fn count(&self) -> usize {
        self.reqs.lock().unwrap().len()
    }
}

struct Outcome {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(env: &Env, stdin: &str, args: &[&str]) -> Outcome {
    let mut input = stdin.as_bytes();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let env = env.clone();
    let code = {
        let mut f = Factory {
            input: &mut input,
            out: &mut out,
            version: "test".into(),
            flags: GlobalFlags::default(),
            load_env: Box::new(move || env.clone()),
        };
        cmd::run(&mut f, &mut err, args)
    };
    Outcome {
        stdout: String::from_utf8(out).unwrap(),
        stderr: String::from_utf8(err).unwrap(),
        code,
    }
}

fn decode(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("output is not JSON: {e}\n{s}"))
}

fn error_type(r: &Outcome) -> String {
    assert!(
        r.stdout.is_empty(),
        "stdout must be empty on error, got {}",
        r.stdout
    );
    decode(&r.stderr)["error"]["type"]
        .as_str()
        .unwrap()
        .to_string()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn test_env(fa: &FakeAsaas, dir: &tempfile::TempDir) -> Env {
    Env {
        api_key: "k".into(),
        base_url: fa.url.clone(),
        config_dir: dir.path().display().to_string(),
        ..Env::default()
    }
}

#[test]
fn customer_list_uses_env_key() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        api_key: "env-key".into(),
        ..test_env(&fa, &dir)
    };

    let r = run(
        &env,
        "",
        &[
            "customer",
            "list",
            "--name",
            "Ana",
            "--limit",
            "5",
            "--sandbox",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let req = fa.last();
    assert_eq!(
        (req.token.as_str(), req.path.as_str(), req.query.as_str()),
        ("env-key", "/v3/customers", "limit=5&name=Ana")
    );
    assert!(
        r.stdout
            .contains(r#""invoiceUrl": "https://x.test/?a=1&b=2""#),
        "output not indented/unescaped JSON:\n{}",
        r.stdout
    );
    // The raw response keeps its key order.
    assert!(r.stdout.find("\"object\"").unwrap() < r.stdout.find("\"data\"").unwrap());
}

#[test]
fn auth_flow_then_customer_list_uses_config_file() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let mut env = Env {
        api_key: String::new(),
        ..test_env(&fa, &dir)
    };

    let r = run(&env, "", &["customer", "list"]);
    assert_eq!(
        (r.code, error_type(&r).as_str()),
        (EXIT_AUTH, "auth_error"),
        "{}",
        r.stderr
    );

    let r = run(
        &env,
        "",
        &[
            "auth",
            "add",
            "--name",
            "sandbox-dev",
            "--key",
            "file-key",
            "--sandbox",
        ],
    );
    assert_eq!(r.code, 0, "auth add: {}", r.stderr);
    let r = run(
        &env,
        "prod-key\n",
        &[
            "auth",
            "add",
            "--name",
            "prod",
            "--key",
            "-",
            "--production",
            "--activate=false",
        ],
    );
    assert_eq!(r.code, 0, "auth add stdin: {}", r.stderr);

    let r = run(&env, "", &["auth", "status"]);
    let st = decode(&r.stdout);
    assert_eq!(
        (
            st["account"].as_str(),
            st["environment"].as_str(),
            st["source"].as_str()
        ),
        (Some("sandbox-dev"), Some("sandbox"), Some(SOURCE_FILE))
    );
    assert!(
        !r.stdout.contains("file-key"),
        "status leaked the full API key"
    );

    let r = run(&env, "", &["auth", "list"]);
    let list = decode(&r.stdout);
    assert_eq!(list["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(list["accounts"][0]["name"], "prod");

    let r = run(&env, "", &["customer", "list"]);
    assert_eq!(r.code, 0, "customer list: {}", r.stderr);
    assert_eq!(fa.last().token, "file-key");

    let r = run(&env, "", &["auth", "switch", "--name", "prod"]);
    assert_eq!(r.code, 0, "switch: {}", r.stderr);
    run(&env, "", &["customer", "list"]);
    assert_eq!(fa.last().token, "prod-key", "after switch");

    let r = run(&env, "", &["auth", "switch", "--name", "ghost"]);
    assert_eq!(r.code, EXIT_AUTH);
    assert_eq!(
        decode(&r.stderr)["error"]["available_accounts"],
        json!(["prod", "sandbox-dev"])
    );

    // ASAAS_API_KEY takes priority over the active account.
    env.api_key = "env-key".into();
    run(&env, "", &["customer", "list"]);
    assert_eq!(fa.last().token, "env-key");

    let r = run(&env, "", &["auth", "remove", "--name", "prod"]);
    assert_eq!(r.code, 0, "remove: {}", r.stderr);
    assert_eq!(decode(&r.stdout)["active_account"], "");
}

#[test]
fn api_error_goes_to_stderr_as_json() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        api_key: "bad".into(),
        ..test_env(&fa, &dir)
    };
    let r = run(&env, "", &["customer", "list"]);
    assert_eq!(
        (r.code, error_type(&r).as_str()),
        (EXIT_ERROR, "api_error"),
        "{}",
        r.stderr
    );
    let e = &decode(&r.stderr)["error"];
    assert_eq!(num(&e["status_code"]), 401.0);
    assert_eq!(e["errors"][0]["code"], "invalid_access_token");
    assert!(e["hint"].as_str().unwrap().contains("rejected"));
}

#[test]
fn customer_create_update_delete() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let r = run(
        &env,
        "",
        &[
            "customer",
            "create",
            "--name",
            "Ana",
            "--cpf-cnpj",
            "01234567890",
            "--notification-disabled",
            "--data",
            r#"{"company":"ACME","name":"ignored"}"#,
        ],
    );
    assert_eq!(r.code, 0, "create: {}", r.stderr);
    let body = decode(&fa.last().body);
    assert_eq!(body["name"], "Ana");
    assert_eq!(body["cpfCnpj"], "01234567890");
    assert_eq!(body["notificationDisabled"], true);
    assert_eq!(body["company"], "ACME");
    assert!(body.get("email").is_none(), "unset flags must not be sent");

    let r = run(&env, "", &["customer", "create", "--name", "Ana"]);
    assert!(
        r.code == EXIT_USAGE && r.stderr.contains("cpf-cnpj"),
        "missing required flag: {} {}",
        r.code,
        r.stderr
    );

    let r = run(
        &env,
        "",
        &["customer", "update", "cus_1", "--email", "a@b.com"],
    );
    let req = fa.last();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        (req.method.as_str(), req.path.as_str(), req.body.as_str()),
        ("PUT", "/v3/customers/cus_1", r#"{"email":"a@b.com"}"#)
    );

    assert_eq!(
        run(&env, "", &["customer", "update", "cus_1"]).code,
        EXIT_USAGE,
        "empty update"
    );

    let r = run(&env, "", &["customer", "delete", "cus_1"]);
    assert_eq!(decode(&r.stdout)["deleted"], true, "{}", r.stderr);

    let r = run(&env, "", &["customer", "get"]);
    assert_eq!(r.code, EXIT_USAGE, "get without id");
    assert!(r.stderr.contains("<customer-id>"));
    assert_eq!(decode(&r.stderr)["error"]["command"], "asaas customer get");

    run(&env, "", &["customer", "restore", "cus 1"]);
    assert_eq!(fa.last().path, "/v3/customers/cus%201/restore");
}

#[test]
fn api_command() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let r = run(
        &env,
        "",
        &[
            "api",
            "/payments",
            "-X",
            "post",
            "-f",
            "cpfCnpj=0123",
            "-F",
            "value=129.90",
            "-F",
            "flag=true",
        ],
    );
    assert_eq!(r.code, 0, "api: {}", r.stderr);
    let req = fa.last();
    let body = decode(&req.body);
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v3/payments")
    );
    assert_eq!(body["cpfCnpj"], "0123");
    assert_eq!(num(&body["value"]), 129.90);
    assert_eq!(body["flag"], true);
    assert!(
        req.body.contains("129.90"),
        "number literal must be sent as typed: {}",
        req.body
    );

    let r = run(&env, "", &["api", "/x/empty", "-X", "POST"]);
    let out = decode(&r.stdout);
    assert_eq!(
        (num(&out["status_code"]), &out["success"]),
        (204.0, &json!(true))
    );

    let r = run(
        &env,
        "",
        &[
            "api",
            "/payments",
            "-X",
            "POST",
            "--data",
            r#"{"b":1,"a":2}"#,
        ],
    );
    assert_eq!(r.code, 0);
    assert_eq!(
        fa.last().body,
        r#"{"b":1,"a":2}"#,
        "--data alone is sent verbatim"
    );

    run(
        &env,
        "",
        &[
            "api",
            "/payments",
            "-q",
            "status=PENDING",
            "--query",
            "limit=2",
        ],
    );
    assert_eq!(fa.last().query, "limit=2&status=PENDING");

    let before = fa.count();
    assert_ne!(
        run(&env, "", &["api", "https://evil.example/v3/x"]).code,
        0,
        "absolute URLs must be rejected"
    );
    assert_eq!(
        run(&env, "", &["api", "/customers", "-f", "a=b"]).code,
        EXIT_USAGE,
        "GET with body"
    );
    assert_eq!(
        run(&env, "", &["api", "/customers", "-X", "TRACE"]).code,
        EXIT_USAGE
    );
    assert_eq!(
        run(&env, "", &["api", "/customers", "--all", "-X", "POST"]).code,
        EXIT_USAGE
    );
    assert_eq!(run(&env, "", &["api"]).code, EXIT_USAGE);
    assert_eq!(fa.count(), before, "rejected calls must not hit the API");

    let r = run(&env, "", &["api", "/customers", "--all"]);
    let page = decode(&r.stdout);
    assert_eq!(
        (page["object"].as_str(), num(&page["limit"])),
        (Some("list"), 1.0),
        "{}",
        r.stderr
    );
}

#[test]
fn usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        config_dir: dir.path().display().to_string(),
        ..Env::default()
    };
    let cases: &[&[&str]] = &[
        &["bogus"],
        &["customer", "bogus"],
        &["customer", "list", "--limit", "0"],
        &["customer", "list", "--limit", "abc"],
        &["customer", "list", "--nope"],
        &["auth", "status", "--sandbox", "--production"],
        &["auth", "add", "--name", "x", "--key", "plain"],
        &[
            "auth",
            "add",
            "--name",
            "x",
            "--key",
            "$aact_prod_abc",
            "--sandbox",
        ],
        &[
            "auth",
            "add",
            "--name",
            "bad name",
            "--key",
            "$aact_prod_abc",
        ],
        &["version", "extra"],
    ];
    for args in cases {
        let r = run(&env, "", args);
        assert_eq!(
            (r.code, error_type(&r).as_str()),
            (EXIT_USAGE, "usage_error"),
            "{args:?}: {}",
            r.stderr
        );
        assert!(
            decode(&r.stderr)["error"]["hint"].is_string(),
            "{args:?}: usage errors carry a hint"
        );
    }
}

#[test]
fn help_and_version_print_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        config_dir: dir.path().display().to_string(),
        ..Env::default()
    };

    for args in [
        &[][..],
        &["--help"],
        &["customer"],
        &["invoice", "create", "--help"],
    ] {
        let r = run(&env, "", args);
        assert_eq!(r.code, 0, "{args:?}: {}", r.stderr);
        assert!(r.stdout.contains("Usage:"), "{args:?}: {}", r.stdout);
        assert!(r.stderr.is_empty());
    }
    let help = run(&env, "", &["invoice", "create", "--help"]).stdout;
    assert!(help.contains("[required, via this flag or --data]"));
    assert!(help.contains("Examples:"));

    let r = run(&env, "", &["version"]);
    let v = decode(&r.stdout);
    assert_eq!(v["version"], "test");
    assert!(v["rust_version"].is_string() && v["os"].is_string() && v["arch"].is_string());
}

#[test]
fn every_command_is_documented() {
    let root = cmd::root_command();
    let mut stack: Vec<&clap::Command> = root.get_subcommands().collect();
    while let Some(c) = stack.pop() {
        stack.extend(c.get_subcommands());
        let name = c.get_name();
        assert!(
            c.get_about().is_some() && c.get_long_about().is_some(),
            "{name}: missing short or long description"
        );
        for a in c.get_arguments().filter(|a| !a.is_positional()) {
            let help = a.get_help().map(|h| h.to_string()).unwrap_or_default();
            assert!(
                help.contains("[type: "),
                "{name} --{}: help does not document its type",
                a.get_id()
            );
        }
    }
    for a in root.get_arguments() {
        let help = a.get_help().map(|h| h.to_string()).unwrap_or_default();
        assert!(
            help.contains("[type: "),
            "asaas --{}: help does not document its type",
            a.get_id()
        );
    }
}

// ---- invoices and fiscal info ----

const TAX_ARGS: &[&str] = &[
    "--retain-iss=false",
    "--iss",
    "2",
    "--pis",
    "0",
    "--cofins",
    "0",
    "--csll",
    "0",
    "--inss",
    "0",
    "--ir",
    "0",
];

fn args<'a>(parts: &[&[&'a str]]) -> Vec<&'a str> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

#[test]
fn invoice_create() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let a = args(&[
        &[
            "invoice",
            "create",
            "--payment",
            "pay_1",
            "--service-description",
            "Consultoria",
            "--observations",
            "Mensal",
            "--value",
            "300",
            "--effective-date",
            "2026-10-31",
            "--municipal-service-id",
            "60380",
            "--nbs-code",
            "1.1502",
        ],
        TAX_ARGS,
    ]);
    let r = run(&env, "", &a);
    assert_eq!(r.code, 0, "create: {}", r.stderr);
    let req = fa.last();
    let body = decode(&req.body);
    let taxes = &body["taxes"];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v3/invoices")
    );
    assert_eq!(num(&body["value"]), 300.0);
    assert_eq!(num(&body["deductions"]), 0.0);
    assert_eq!(body["effectiveDate"], "2026-10-31");
    assert_eq!(taxes["retainIss"], false);
    assert_eq!(num(&taxes["iss"]), 2.0);
    assert_eq!(taxes["nbsCode"], "1.1502");
    assert!(
        req.body.contains(r#""value":300}"#),
        "integral floats are sent without .0: {}",
        req.body
    );
}

#[test]
fn invoice_create_taxes_from_data() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let data = r#"{"taxes":{"retainIss":false,"iss":5,"pis":0,"cofins":0,"csll":0,"inss":0,"ir":0,"realEstateInfo":{"cibCode":"X"}}}"#;
    let r = run(
        &env,
        data,
        &[
            "invoice",
            "create",
            "--customer",
            "cus_1",
            "--service-description",
            "Aluguel",
            "--observations",
            "-",
            "--value",
            "1000",
            "--effective-date",
            "2026-11-05",
            "--municipal-service-code",
            "25.05",
            "--iss",
            "3",
            "--data",
            "-",
        ],
    );
    assert_eq!(r.code, 0, "create: {}", r.stderr);
    let body = decode(&fa.last().body);
    let taxes = &body["taxes"];
    assert_eq!(num(&taxes["iss"]), 3.0, "flag wins over --data");
    assert_eq!(num(&taxes["pis"]), 0.0);
    assert_eq!(
        taxes["realEstateInfo"]["cibCode"], "X",
        "deep merge failed: {taxes}"
    );
    assert_eq!(body["observations"], "-");
}

#[test]
fn invoice_create_validation() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);
    let base: &[&str] = &[
        "invoice",
        "create",
        "--service-description",
        "x",
        "--observations",
        "x",
        "--value",
        "1",
        "--effective-date",
        "2026-10-31",
    ];

    let cases = [
        (
            "no origin",
            args(&[base, &["--municipal-service-id", "1"], TAX_ARGS]),
            "--payment",
        ),
        (
            "no service",
            args(&[base, &["--customer", "cus_1"], TAX_ARGS]),
            "--municipal-service-id",
        ),
        (
            "missing taxes",
            args(&[
                base,
                &[
                    "--customer",
                    "cus_1",
                    "--municipal-service-id",
                    "1",
                    "--iss",
                    "2",
                ],
            ]),
            "--retain-iss (taxes.retainIss), --pis",
        ),
        (
            "bad date",
            vec!["invoice", "create", "--effective-date", "31/10/2026"],
            "YYYY-MM-DD",
        ),
        (
            "bad op type",
            vec!["invoice", "create", "--operation-type-code", "9"],
            "between 1 and 5",
        ),
        (
            "bad data",
            args(&[base, &["--data", "[1]"]]),
            "must be a JSON object",
        ),
    ];
    for (name, a, want) in cases {
        let r = run(&env, "", &a);
        assert!(
            r.code == EXIT_USAGE && r.stderr.contains(want),
            "{name}: exit {}, stderr {}",
            r.code,
            r.stderr
        );
    }
    assert_eq!(fa.count(), 0, "validation errors must not hit the API");
}

#[test]
fn invoice_update_completes_partial_taxes() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let r = run(&env, "", &["invoice", "update", "inv_1", "--iss", "3.5"]);
    assert_eq!(r.code, 0, "update: {}", r.stderr);
    let req = fa.last();
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("PUT", "/v3/invoices/inv_1")
    );
    let body = decode(&req.body);
    let taxes = &body["taxes"];
    assert_eq!(num(&taxes["iss"]), 3.5);
    assert_eq!(taxes["retainIss"], true);
    assert_eq!(num(&taxes["cofins"]), 3.0);
    assert_eq!(num(&taxes["ir"]), 1.5);
    assert!(
        taxes.get("pisCofinsRetentionType").is_none(),
        "pisCofinsRetentionType must not be sent"
    );

    // Non-tax updates do not read the invoice first.
    let before = fa.count();
    run(
        &env,
        "",
        &[
            "invoice",
            "update",
            "inv_1",
            "--effective-date",
            "2026-11-10",
        ],
    );
    assert_eq!(fa.count(), before + 1, "expected a single PUT");
}

#[test]
fn invoice_list_authorize_cancel() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    run(
        &env,
        "",
        &[
            "invoice",
            "list",
            "--status",
            "AUTHORIZED",
            "--effective-date-from",
            "2026-10-01",
            "--effective-date-to",
            "2026-10-31",
        ],
    );
    assert_eq!(
        fa.last().query,
        "effectiveDate%5Bge%5D=2026-10-01&effectiveDate%5Ble%5D=2026-10-31&status=AUTHORIZED"
    );
    assert_eq!(
        run(&env, "", &["invoice", "list", "--status", "authorized"]).code,
        EXIT_USAGE,
        "invalid enum"
    );

    run(&env, "", &["invoice", "authorize", "inv_1"]);
    let req = fa.last();
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v3/invoices/inv_1/authorize")
    );

    run(&env, "", &["invoice", "cancel", "inv_1", "--only-on-asaas"]);
    let req = fa.last();
    assert_eq!(
        (req.path.as_str(), req.body.as_str()),
        ("/v3/invoices/inv_1/cancel", r#"{"cancelOnlyOnAsaas":true}"#)
    );

    run(&env, "", &["invoice", "cancel", "inv_1"]);
    assert_eq!(fa.last().body, "{}");
}

#[test]
fn fiscal_info() {
    let fa = FakeAsaas::new();
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(&fa, &dir);

    let cert = dir.path().join("empresa.pfx");
    std::fs::write(&cert, "PFXDATA").unwrap();
    let cert = cert.display().to_string();
    let r = run(
        &env,
        "s3cret\n",
        &[
            "fiscal-info",
            "save",
            "--email",
            "f@x.com",
            "--simples-nacional=true",
            "--rps-number",
            "10",
            "--certificate-file",
            &cert,
            "--certificate-password",
            "-",
        ],
    );
    assert_eq!(r.code, 0, "save: {}", r.stderr);
    let out = decode(&r.stdout);
    assert_eq!(out["email"], "f@x.com");
    assert_eq!(out["simplesNacional"], "true");
    assert_eq!(out["rpsNumber"], "10");
    assert_eq!(out["certificatePassword"], "s3cret");
    assert_eq!(out["certificateFile"], "empresa.pfx:PFXDATA");
    assert_eq!(fa.last().path, "/v3/fiscalInfo/");

    let r = run(&env, "", &["fiscal-info", "save", "--email", "f@x.com"]);
    assert!(
        r.code == EXIT_USAGE && r.stderr.contains("simples-nacional"),
        "missing simplesNacional: {} {}",
        r.code,
        r.stderr
    );

    let r = run(
        &env,
        "",
        &[
            "fiscal-info",
            "save",
            "--email",
            "f@x.com",
            "--simples-nacional=false",
            "--password",
            "-",
            "--access-token",
            "-",
        ],
    );
    assert!(
        r.code == EXIT_USAGE && r.stderr.contains("only one secret"),
        "{}",
        r.stderr
    );

    run(
        &env,
        "",
        &[
            "fiscal-info",
            "services",
            "--description",
            "software",
            "--limit",
            "5",
        ],
    );
    let req = fa.last();
    assert_eq!(
        (req.path.as_str(), req.query.as_str()),
        ("/v3/fiscalInfo/services", "description=software&limit=5")
    );

    run(&env, "", &["fiscal-info", "nbs-codes", "--search", "1.15"]);
    let req = fa.last();
    assert_eq!(
        (req.path.as_str(), req.query.as_str()),
        ("/v3/fiscalInfo/nbsCodes", "codeDescription=1.15")
    );

    run(
        &env,
        "",
        &["fiscal-info", "national-portal", "--enabled=false"],
    );
    let req = fa.last();
    assert_eq!(
        (req.path.as_str(), req.body.as_str()),
        ("/v3/fiscalInfo/nationalPortal", r#"{"enabled":false}"#)
    );

    run(&env, "", &["fiscal-info", "national-portal", "--enabled"]);
    assert_eq!(fa.last().body, r#"{"enabled":true}"#);

    assert_eq!(
        run(&env, "", &["fiscal-info", "national-portal"]).code,
        EXIT_USAGE,
        "national-portal without --enabled"
    );
}
