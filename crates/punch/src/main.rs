//! CLI adapter over punch-core operations.
mod cli;
mod mcp;
mod tui;
use clap::{CommandFactory, Parser};
use punch_core::error::PunchError;
use std::process::ExitCode;

fn failure(error: PunchError, json: bool) -> ExitCode {
    if json {
        println!(
            "{}",
            serde_json::json!({"ok":false,"refusals":[error.detail()]})
        );
    } else {
        eprintln!("{}: {}", error.code, error.message);
    }
    ExitCode::from(error.exit_code)
}
fn main() -> ExitCode {
    let raw: Vec<_> = std::env::args_os().collect();
    let json = raw
        .iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "--json");
    let args = match cli::Cli::try_parse_from(raw) {
        Ok(args) => args,
        Err(e) => {
            if e.exit_code() == 0 {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"ok":true,"changed":false,"data":e.to_string(),"warnings":[]})
                    );
                } else {
                    let _ = e.print();
                }
                return ExitCode::SUCCESS;
            }
            return failure(PunchError::usage(e), json);
        }
    };
    if !args.json && (args.command.is_none() || matches!(args.command, Some(cli::Command::Board))) {
        let actor = args
            .slot
            .as_ref()
            .map_or(punch_core::model::Actor::Person, |s| {
                punch_core::model::Actor::Agent(s.clone())
            });
        return match tui::run(actor) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => failure(e, false),
        };
    }
    if args.command.is_none() {
        if args.json {
            println!(
                "{}",
                serde_json::json!({"ok":true,"changed":false,"data":cli::Cli::command().render_help().to_string(),"warnings":[]})
            );
        } else {
            let _ = cli::Cli::command().print_help();
            println!();
        }
        return ExitCode::SUCCESS;
    }
    if matches!(args.command, Some(cli::Command::Mcp)) {
        return match mcp::run(args.slot.as_deref().unwrap_or("mcp")) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}: {}", e.code, e.message);
                ExitCode::from(e.exit_code)
            }
        };
    }
    match cli::execute(&args) {
        Ok(reply) => {
            if args.json {
                println!("{}", reply.value);
            } else {
                cli::print_text(&reply.value["data"]);
            }
            ExitCode::from(reply.exit_code)
        }
        Err(e) => failure(e, args.json),
    }
}
