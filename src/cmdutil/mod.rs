//! The plumbing shared by every command: dependency factory, JSON output,
//! error rendering and declarative flag helpers.

mod errors;
mod factory;
mod fields;
mod output;
mod resource;

use clap::{Arg, Command};

pub use errors::{
    EXIT_AUTH, EXIT_ERROR, EXIT_NETWORK, EXIT_OK, EXIT_USAGE, Error, Kind, Result, handle_error,
};
pub use factory::{Factory, GlobalFlags, object};
pub use fields::{
    Field, FieldType, add_fields, bool_arg, changed, check_required, collect_body, collect_query,
    deep_merge, float_value, get_bool, get_int, get_string, get_strings, has_path, mark_required,
    merge_json_object, pagination_fields, parse_key_value, read_json_arg, require_one_of, set_path,
    string_arg, string_list_arg, typed_value, validate_pagination,
};
pub use output::print_json;
pub use resource::{new_group, print_group_help, resource_path, run_list, run_request};

/// A command with the conventions shared by the whole tree: usage line,
/// short and long descriptions, and "last occurrence wins" for repeated flags.
pub fn new_command(
    name: &'static str,
    usage: &'static str,
    short: &'static str,
    long: impl Into<String>,
) -> Command {
    Command::new(name)
        .override_usage(usage)
        .about(short)
        .long_about(long.into())
        .args_override_self(true)
}

/// Appends an "Examples:" section to the help.
pub fn with_examples(cmd: Command, examples: &'static str) -> Command {
    cmd.after_help(format!("Examples:\n{examples}"))
}

/// Positional arguments, validated by each command (so the error message can
/// show the expected ID format). Hidden from the flag list; documented in the
/// long description.
pub fn positional_args() -> Arg {
    Arg::new("args")
        .num_args(0..)
        .action(clap::ArgAction::Append)
        .hide(true)
}
