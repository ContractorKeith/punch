//! CLI adapter over punch-core operations.
mod cli;
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
    match cli::execute(&args) {
        Ok(reply) => {
            if args.json {
                println!("{}", reply.value);
            } else {
                cli::print_text(&reply.value["data"]);
                for warning in reply.value["warnings"].as_array().into_iter().flatten() {
                    eprintln!("{}", warning.as_str().unwrap_or(""));
                }
            }
            ExitCode::from(reply.exit_code)
        }
        Err(e) => failure(e, args.json),
    }
}
