use clap::{CommandFactory, Parser, error::ErrorKind};
use gendock::{
    app,
    cli::{Cli, output},
};
use serde_json::{Value, json};

fn argument_error(error: &clap::Error) -> Value {
    use clap::error::{ContextKind, ContextValue};
    let mut details = serde_json::Map::new();
    for (kind, value) in error.context() {
        let key = match kind {
            ContextKind::InvalidArg => "arguments",
            ContextKind::InvalidSubcommand => "invalid_command",
            ContextKind::InvalidValue => "invalid_value",
            ContextKind::ValidValue => "allowed_values",
            ContextKind::ValidSubcommand => "allowed_commands",
            ContextKind::SuggestedArg => "suggested_argument",
            ContextKind::SuggestedSubcommand => "suggested_command",
            ContextKind::SuggestedValue => "suggested_value",
            ContextKind::ExpectedNumValues => "expected_values",
            ContextKind::ActualNumValues => "actual_values",
            _ => continue,
        };
        let value = match value {
            ContextValue::String(v) => json!(v),
            ContextValue::Strings(v) => json!(v),
            ContextValue::Number(v) => json!(v),
            ContextValue::Bool(v) => json!(v),
            _ => continue,
        };
        details.insert(key.into(), value);
    }
    let message = match error.kind() {
        ErrorKind::MissingRequiredArgument => "Required arguments are missing.",
        ErrorKind::UnknownArgument => "An argument is not recognized.",
        ErrorKind::InvalidSubcommand => "A command is not recognized.",
        ErrorKind::ValueValidation | ErrorKind::InvalidValue => "An argument value is invalid.",
        ErrorKind::ArgumentConflict => "Arguments conflict with each other.",
        _ => "Command arguments are invalid.",
    };
    output::failure(
        "invalid_arguments",
        message,
        false,
        Value::Object(details),
        vec![output::NextStep::help(vec!["gendock".into()])],
    )
}

fn main() -> std::process::ExitCode {
    let args = std::env::args_os().collect::<Vec<_>>();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            let (value, exit) = match error.kind() {
                ErrorKind::DisplayVersion => (
                    output::success(json!({"name":"gendock","version":env!("CARGO_PKG_VERSION")})),
                    0,
                ),
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    let mut root = Cli::command();
                    root.build();
                    let mut selected = &mut root;
                    for arg in args.iter().skip(1) {
                        let Some(name) = arg.to_str() else { break };
                        if selected.find_subcommand(name).is_none() {
                            break;
                        }
                        selected = selected
                            .find_subcommand_mut(name)
                            .expect("subcommand exists");
                    }
                    // Help is a human-facing discovery surface, not an operation result.
                    let _ = selected.print_help();
                    println!();
                    return std::process::ExitCode::SUCCESS;
                }
                _ => (argument_error(&error), 2),
            };
            output::emit(&value);
            return std::process::ExitCode::from(exit);
        }
    };
    match app::execute_at(cli.command, cli.url) {
        Ok(result) => {
            if result.emit {
                output::emit(&result.value);
            }
            std::process::ExitCode::from(result.exit)
        }
        Err(error) => {
            output::emit(&output::operation_error(&error));
            std::process::ExitCode::FAILURE
        }
    }
}
