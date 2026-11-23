mod command;
mod mcp;
use phyro_automation::{edit, inspect, CliError};
mod output;

use serde_json::json;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "mcp") {
        return mcp::run(&arguments[1..]);
    }
    if command::is_help(&arguments) {
        println!("{}", command::HELP);
        return ExitCode::SUCCESS;
    }
    let (response, status) = match command::run(arguments) {
        Ok(response) => (response, ExitCode::SUCCESS),
        Err(error) => (
            json!({"version":1,"ok":false,"error":error}),
            ExitCode::FAILURE,
        ),
    };
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &response).is_err() || writeln!(stdout).is_err() {
        return ExitCode::FAILURE;
    }
    status
}
