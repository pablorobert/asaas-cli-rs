use std::io::Read;

use clap::parser::ValueSource;
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Map, Number, Value};

use crate::client::Query;
use crate::usage_err;

use super::Result;

/// The data type of a declarative flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Int,
    Float,
    Bool,
    /// String in YYYY-MM-DD format.
    Date,
}

impl FieldType {
    fn label(self) -> &'static str {
        match self {
            FieldType::String => "string",
            FieldType::Int => "integer",
            FieldType::Float => "decimal number, dot as separator",
            FieldType::Bool => "boolean",
            FieldType::Date => "date, YYYY-MM-DD",
        }
    }
}

/// Maps one CLI flag to one Asaas API parameter (query or body). New
/// resources are added by declaring `Vec<Field>`, not by writing parsing code.
#[derive(Debug, Clone)]
pub struct Field {
    /// The API name. A dotted name ("taxes.iss") is sent as a nested JSON
    /// object: {"taxes": {"iss": ...}}.
    pub param: &'static str,
    /// Flag name, e.g. "cpf-cnpj".
    pub flag: &'static str,
    pub ty: FieldType,
    pub usage: &'static str,
    /// Makes the parser reject the command when the flag is missing.
    pub required: bool,
    /// The value must be present in the final request body, either from
    /// this flag or from --data (see check_required).
    pub body_required: bool,
    /// The accepted values (String fields only).
    pub enum_values: &'static [&'static str],
}

impl Field {
    /// A string field.
    pub const fn new(param: &'static str, flag: &'static str, usage: &'static str) -> Self {
        Field {
            param,
            flag,
            ty: FieldType::String,
            usage,
            required: false,
            body_required: false,
            enum_values: &[],
        }
    }

    pub const fn int(mut self) -> Self {
        self.ty = FieldType::Int;
        self
    }

    pub const fn float(mut self) -> Self {
        self.ty = FieldType::Float;
        self
    }

    pub const fn bool(mut self) -> Self {
        self.ty = FieldType::Bool;
        self
    }

    pub const fn date(mut self) -> Self {
        self.ty = FieldType::Date;
        self
    }

    pub const fn required(mut self, yes: bool) -> Self {
        self.required = yes;
        self
    }

    pub const fn body_required(mut self, yes: bool) -> Self {
        self.body_required = yes;
        self
    }

    pub const fn one_of(mut self, values: &'static [&'static str]) -> Self {
        self.enum_values = values;
        self
    }

    fn validate(&self, v: &str) -> Result<()> {
        if self.ty == FieldType::Date && !valid_date(v) {
            return Err(usage_err!(
                "--{}: {v:?} is not a date in YYYY-MM-DD format",
                self.flag
            ));
        }
        if !self.enum_values.is_empty() && !self.enum_values.contains(&v) {
            return Err(usage_err!(
                "--{}: {v:?} is not one of: {}",
                self.flag,
                self.enum_values.join(", ")
            ));
        }
        Ok(())
    }
}

/// A string-valued flag. Like pflag, the next argument is always taken as
/// the value, even when it starts with '-'.
pub fn string_arg(name: &'static str, help: impl Into<String>) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help.into())
        .action(ArgAction::Set)
        .allow_hyphen_values(true)
}

/// A repeatable string flag (each occurrence adds one value).
pub fn string_list_arg(name: &'static str, help: impl Into<String>) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help.into())
        .action(ArgAction::Append)
        .allow_hyphen_values(true)
}

/// A boolean flag: `--x` means true; `--x=true` / `--x=false` set it
/// explicitly (also 1/0, t/f, TRUE/FALSE, True/False).
pub fn bool_arg(name: &'static str, help: impl Into<String>) -> Arg {
    Arg::new(name)
        .long(name)
        .help(help.into())
        .action(ArgAction::Set)
        .num_args(0..=1)
        .require_equals(true)
        .default_missing_value("true")
        .value_parser(parse_bool)
}

fn parse_bool(s: &str) -> std::result::Result<bool, String> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Ok(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Ok(false),
        _ => Err(format!("invalid boolean {s:?}: use true or false")),
    }
}

fn parse_int(s: &str) -> std::result::Result<i64, String> {
    s.parse::<i64>()
        .map_err(|e| format!("invalid integer {s:?}: {e}"))
}

fn parse_float(s: &str) -> std::result::Result<f64, String> {
    match s.parse::<f64>() {
        Ok(f) if f.is_finite() => Ok(f),
        _ => Err(format!(
            "invalid decimal number {s:?}: use a dot as separator, e.g. 129.90"
        )),
    }
}

/// Marks a flag as required and appends "[required]" to its help.
pub fn mark_required(arg: Arg) -> Arg {
    let help = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
    if help.ends_with("[required]") {
        return arg.required(true);
    }
    arg.required(true).help(format!("{help} [required]"))
}

/// Registers fields as flags on cmd, documenting each one's type.
pub fn add_fields(mut cmd: Command, fields: &[Field]) -> Command {
    for f in fields {
        let mut usage = f.usage.to_string();
        if !f.enum_values.is_empty() {
            usage += &format!("; one of: {}", f.enum_values.join(", "));
        }
        usage += &format!(" [type: {}]", f.ty.label());
        if f.body_required {
            usage += " [required, via this flag or --data]";
        }
        let mut arg = match f.ty {
            FieldType::Bool => bool_arg(f.flag, usage),
            FieldType::Int => string_arg(f.flag, usage).value_parser(parse_int),
            FieldType::Float => string_arg(f.flag, usage).value_parser(parse_float),
            FieldType::String | FieldType::Date => string_arg(f.flag, usage),
        };
        if f.required {
            arg = mark_required(arg);
        }
        cmd = cmd.arg(arg);
    }
    cmd
}

/// Whether the flag was given on the command line.
pub fn changed(m: &ArgMatches, id: &str) -> bool {
    m.value_source(id) == Some(ValueSource::CommandLine)
}

/// The string value of a flag ("" when absent).
pub fn get_string(m: &ArgMatches, id: &str) -> String {
    m.get_one::<String>(id).cloned().unwrap_or_default()
}

/// The boolean value of a flag (false when absent and without default).
pub fn get_bool(m: &ArgMatches, id: &str) -> bool {
    m.get_one::<bool>(id).copied().unwrap_or(false)
}

/// The integer value of a flag, if set.
pub fn get_int(m: &ArgMatches, id: &str) -> Option<i64> {
    m.get_one::<i64>(id).copied()
}

/// Every value of a repeatable flag, or the positional arguments.
pub fn get_strings(m: &ArgMatches, id: &str) -> Vec<String> {
    m.get_many::<String>(id)
        .map(|v| v.cloned().collect())
        .unwrap_or_default()
}

/// A JSON number for a float, formatted without a trailing ".0"
/// (300.0 is sent as 300, 3.5 as 3.5).
pub fn float_value(f: f64) -> Value {
    f.to_string()
        .parse::<Number>()
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn field_value(m: &ArgMatches, f: &Field) -> Value {
    match f.ty {
        FieldType::Int => Value::from(get_int(m, f.flag).unwrap_or_default()),
        FieldType::Float => float_value(m.get_one::<f64>(f.flag).copied().unwrap_or_default()),
        FieldType::Bool => Value::Bool(get_bool(m, f.flag)),
        FieldType::String | FieldType::Date => Value::String(get_string(m, f.flag)),
    }
}

/// The explicitly set fields as a JSON object, validating enums and dates.
/// Dotted params become nested objects.
pub fn collect_body(m: &ArgMatches, fields: &[Field]) -> Result<Map<String, Value>> {
    let mut body = Map::new();
    for f in fields.iter().filter(|f| changed(m, f.flag)) {
        let v = field_value(m, f);
        if let Value::String(s) = &v {
            f.validate(s)?;
        }
        set_path(&mut body, f.param, v);
    }
    Ok(body)
}

/// Stores v under a dotted path, creating intermediate objects.
pub fn set_path(m: &mut Map<String, Value>, path: &str, v: Value) {
    match path.split_once('.') {
        None => {
            m.insert(path.to_string(), v);
        }
        Some((head, rest)) => {
            let entry = m
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(next) = entry {
                set_path(next, rest, v);
            }
        }
    }
}

/// Whether a dotted path is present in m.
pub fn has_path(m: &Map<String, Value>, path: &str) -> bool {
    match path.split_once('.') {
        None => m.contains_key(path),
        Some((head, rest)) => match m.get(head) {
            Some(Value::Object(next)) => has_path(next, rest),
            _ => false,
        },
    }
}

/// Verifies that every body_required field ended up in body (from its flag
/// or from --data) and reports all missing ones at once.
pub fn check_required(body: &Map<String, Value>, fields: &[Field]) -> Result<()> {
    let missing: Vec<String> = fields
        .iter()
        .filter(|f| f.body_required && !has_path(body, f.param))
        .map(|f| format!("--{} ({})", f.flag, f.param))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(usage_err!(
            "missing required fields: {}",
            missing.join(", ")
        ))
    }
}

/// Fails unless at least one of the dotted params is in body.
pub fn require_one_of(body: &Map<String, Value>, fields: &[Field], params: &[&str]) -> Result<()> {
    if params.iter().any(|p| has_path(body, p)) {
        return Ok(());
    }
    let names: Vec<String> = params
        .iter()
        .flat_map(|p| fields.iter().filter(move |f| f.param == *p))
        .map(|f| format!("--{}", f.flag))
        .collect();
    Err(usage_err!(
        "one of {} is required (or the equivalent key in --data)",
        names.join(", ")
    ))
}

/// The explicitly set fields as URL query parameters.
pub fn collect_query(m: &ArgMatches, fields: &[Field]) -> Result<Query> {
    let mut q = Query::new();
    for f in fields.iter().filter(|f| changed(m, f.flag)) {
        let v = match field_value(m, f) {
            Value::String(s) => {
                f.validate(&s)?;
                s
            }
            other => other.to_string(),
        };
        q.set(f.param, v);
    }
    Ok(q)
}

/// The standard offset/limit parameters of list endpoints.
pub fn pagination_fields() -> Vec<Field> {
    vec![
        Field::new(
            "offset",
            "offset",
            "Zero-based index of the first item to return (pagination); API default 0",
        )
        .int(),
        Field::new(
            "limit",
            "limit",
            "Number of items per page, from 1 to 100; API default 10",
        )
        .int(),
    ]
}

/// Checks --offset/--limit bounds when they were set.
pub fn validate_pagination(m: &ArgMatches) -> Result<()> {
    if changed(m, "limit")
        && let Some(l) = get_int(m, "limit")
        && !(1..=100).contains(&l)
    {
        return Err(usage_err!("--limit must be between 1 and 100, got {l}"));
    }
    if changed(m, "offset")
        && let Some(o) = get_int(m, "offset")
        && o < 0
    {
        return Err(usage_err!("--offset must be >= 0, got {o}"));
    }
    Ok(())
}

/// Loads a JSON document given inline, as @path, or as "-" (stdin).
pub fn read_json_arg(input: &mut dyn Read, flag: &str, value: &str) -> Result<Vec<u8>> {
    let raw = if value == "-" {
        let mut buf = Vec::new();
        input
            .read_to_end(&mut buf)
            .map_err(|e| usage_err!("--{flag}: {e}"))?;
        buf
    } else if let Some(path) = value.strip_prefix('@') {
        std::fs::read(path).map_err(|e| usage_err!("--{flag}: open {path}: {e}"))?
    } else {
        value.as_bytes().to_vec()
    };
    if serde_json::from_slice::<serde::de::IgnoredAny>(&raw).is_err() {
        return Err(usage_err!("--{flag}: value is not valid JSON"));
    }
    Ok(raw)
}

/// Decodes a JSON object from --data (if any) and deep-merges fields on top
/// of it, so typed flags win over raw JSON while nested objects (e.g.
/// "taxes") keep the keys set only in --data.
pub fn merge_json_object(
    input: &mut dyn Read,
    flag: &str,
    data: &str,
    fields: Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut body = Map::new();
    if !data.is_empty() {
        let raw = read_json_arg(input, flag, data)?;
        body = match serde_json::from_slice::<Value>(&raw) {
            Ok(Value::Object(m)) => m,
            Ok(other) => {
                return Err(usage_err!(
                    "--{flag}: must be a JSON object, got {}",
                    json_type(&other)
                ));
            }
            Err(e) => return Err(usage_err!("--{flag}: must be a JSON object: {e}")),
        };
    }
    deep_merge(&mut body, fields);
    Ok(body)
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Copies src into dst, merging nested objects key by key.
pub fn deep_merge(dst: &mut Map<String, Value>, src: Map<String, Value>) {
    for (k, v) in src {
        match (dst.get_mut(&k), v) {
            (Some(Value::Object(d)), Value::Object(s)) => deep_merge(d, s),
            (_, v) => {
                dst.insert(k, v);
            }
        }
    }
}

/// Splits "key=value".
pub fn parse_key_value(flag: &str, s: &str) -> Result<(String, String)> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(usage_err!("--{flag}: expected key=value, got {s:?}")),
    }
}

/// Converts a -F value into a JSON type: true/false/null, integers and
/// decimals become JSON literals, anything else stays a string.
pub fn typed_value(s: &str) -> Value {
    match s {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    // Only valid JSON numbers qualify: "0123" (leading zero) or "Inf" stay strings.
    let starts_numeric = s.starts_with('-') || s.starts_with(|c: char| c.is_ascii_digit());
    if starts_numeric && let Ok(n) = serde_json::from_str::<Number>(s) {
        return Value::Number(n);
    }
    Value::String(s.to_string())
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (Some(y), Some(m), Some(d)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_values() {
        assert_eq!(typed_value("true"), json!(true));
        assert_eq!(typed_value("null"), Value::Null);
        assert_eq!(
            serde_json::to_string(&typed_value("129.90")).unwrap(),
            "129.90"
        );
        assert_eq!(typed_value("0123"), json!("0123"));
        assert_eq!(typed_value("Inf"), json!("Inf"));
        assert_eq!(typed_value("-"), json!("-"));
        assert_eq!(typed_value(""), json!(""));
    }

    #[test]
    fn dates() {
        for ok in ["2026-10-31", "2024-02-29", "0001-01-01"] {
            assert!(valid_date(ok), "{ok}");
        }
        for bad in [
            "31/10/2026",
            "2026-02-29",
            "2026-13-01",
            "2026-1-01",
            "2026-10-00",
            "+026-10-01",
        ] {
            assert!(!valid_date(bad), "{bad}");
        }
    }

    #[test]
    fn paths_and_merge() {
        let mut m = Map::new();
        set_path(&mut m, "taxes.iss", json!(2));
        set_path(&mut m, "value", json!(1));
        assert!(has_path(&m, "taxes.iss"));
        assert!(!has_path(&m, "taxes.pis"));
        assert!(!has_path(&m, "value.x"));

        let mut dst = json!({"taxes": {"pis": 0, "iss": 5}, "a": 1})
            .as_object()
            .unwrap()
            .clone();
        deep_merge(&mut dst, m);
        assert_eq!(
            Value::Object(dst),
            json!({"taxes": {"pis": 0, "iss": 2}, "a": 1, "value": 1})
        );
    }

    #[test]
    fn float_formatting() {
        assert_eq!(float_value(300.0).to_string(), "300");
        assert_eq!(float_value(3.5).to_string(), "3.5");
        assert_eq!(float_value(0.65).to_string(), "0.65");
    }
}
