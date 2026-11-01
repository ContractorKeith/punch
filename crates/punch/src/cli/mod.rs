//! Command definitions and presenters; storage access goes through ops.
use clap::{Parser, Subcommand};
use punch_core::{error::PunchError, model::*, ops, store::Store};
use serde::Serialize;
#[derive(Parser)]
#[command(
    name = "punch",
    version,
    about = "A terminal punch list across every project"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long = "as", env = "PUNCH_AGENT", global = true)]
    pub slot: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand)]
pub enum Command {
    /// Run the synchronous stdio MCP server (always an agent).
    Mcp,
    /// Print the embedded agent guide; agents should pass --json.
    Guide,
    /// Print an MCP registration command without changing configuration.
    Setup {
        #[arg(value_enum)]
        target: SetupTarget,
    },
    Sync {
        #[arg(short = 'p', long)]
        project: Option<String>,
    },
    Claim {
        reference: String,
        #[arg(long = "slot")]
        claim_slot: Option<String>,
    },
    Unclaim {
        reference: String,
    },
    #[command(about = "Open the interactive board")]
    Board,
    #[command(about = "List ready tasks; agents should pass --json")]
    Ready {
        #[arg(short = 'p', long)]
        project: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Move {
        reference: String,
        column: Column,
    },
    Edit {
        reference: String,
        #[command(flatten)]
        args: EditCommand,
    },
    #[command(about = "Append a timestamped note (not idempotent)")]
    Note {
        reference: String,
        text: String,
    },
    Archive {
        reference: String,
    },
    Unarchive {
        reference: String,
    },
    Add(AddCommand),
    #[command(about = "List tasks; agents should pass --json")]
    List(ListCommand),
    #[command(about = "Show a task; agents should pass --json")]
    Show {
        reference: String,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
}
#[derive(Clone, clap::ValueEnum)]
pub enum SetupTarget {
    Claude,
    Codex,
}

#[derive(clap::Args)]
pub struct EditCommand {
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    notes: Option<String>,
    #[arg(long)]
    priority: Option<Priority>,
    #[arg(long, conflicts_with = "clear_due")]
    due: Option<String>,
    #[arg(long)]
    clear_due: bool,
    #[arg(long, value_delimiter = ',', conflicts_with = "clear_after")]
    after: Option<Vec<String>>,
    #[arg(long)]
    clear_after: bool,
}
#[derive(clap::Args)]
pub struct AddCommand {
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    title: Option<String>,
    #[arg(
        short = 'p',
        long,
        required_unless_present = "file",
        conflicts_with = "file"
    )]
    project: Option<String>,
    #[arg(long,conflicts_with_all=["notes","priority","due","after","assignee","agent"])]
    file: Option<String>,
    #[arg(long, default_value = "")]
    notes: String,
    #[arg(long, default_value = "2")]
    priority: Priority,
    #[arg(long)]
    due: Option<String>,
    #[arg(long, value_delimiter = ',')]
    after: Vec<String>,
    #[arg(long)]
    assignee: Option<String>,
    #[arg(long, requires = "assignee")]
    agent: bool,
}
#[derive(clap::Args)]
pub struct ListCommand {
    #[arg(short = 'p', long)]
    project: Option<String>,
    #[arg(long)]
    column: Option<Column>,
    #[arg(long)]
    all: bool,
}
#[derive(Subcommand)]
pub enum ProjectCommand {
    Add {
        slug: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        github: Option<String>,
        #[arg(long)]
        scope: Option<SyncScope>,
    },
    #[command(about = "List projects; agents should pass --json")]
    List,
    Archive {
        slug: String,
    },
}
pub struct Reply {
    pub value: serde_json::Value,
    pub exit_code: u8,
}
#[derive(Serialize)]
struct Success<T> {
    ok: bool,
    changed: bool,
    data: T,
    warnings: Vec<String>,
}
fn present<T: Serialize>(out: Outcome<T>) -> Result<Reply, PunchError> {
    let value = serde_json::to_value(Success {
        ok: true,
        changed: out.changed,
        data: out.data,
        warnings: out.warnings,
    })
    .map_err(PunchError::storage)?;
    Ok(Reply {
        value,
        exit_code: 0,
    })
}
pub fn execute(cli: &Cli) -> Result<Reply, PunchError> {
    execute_with_plan(cli, None)
}

// MCP supplies a validated, bounded plan while sharing the CLI dispatcher.
pub(crate) fn execute_with_plan(cli: &Cli, plan: Option<&str>) -> Result<Reply, PunchError> {
    if matches!(cli.command, Some(Command::Guide)) {
        return present(Outcome {
            changed: false,
            data: include_str!("../guide.md").trim_end(),
            warnings: vec![],
        });
    }
    if let Some(Command::Setup { target }) = &cli.command {
        let command = match target {
            SetupTarget::Claude => "claude mcp add punch -- punch mcp",
            SetupTarget::Codex => "codex mcp add punch -- punch mcp",
        };
        return present(Outcome {
            changed: false,
            data: command,
            warnings: vec![],
        });
    }
    let actor = cli
        .slot
        .as_ref()
        .map_or(Actor::Person, |s| Actor::Agent(s.clone()));
    if matches!(cli.command, Some(Command::Board)) {
        return Err(PunchError::usage("board is interactive; use list --json"));
    }
    let mut store = Store::open()?;
    match cli
        .command
        .as_ref()
        .ok_or_else(|| PunchError::usage("missing command"))?
    {
        Command::Setup { .. } => unreachable!("setup is handled before opening the store"),
        Command::Guide => unreachable!("guide is handled before opening the store"),
        Command::Mcp => Err(PunchError::usage("MCP uses the stdio transport")),
        Command::Sync { project } => {
            let out = ops::sync(
                &mut store,
                &actor,
                &punch_core::github::RealGh,
                project.as_deref(),
            )?;
            let failed = out.data.iter().any(|r| r.status == "failed");
            let mut reply = present(out)?;
            if failed {
                reply.exit_code = 3;
            }
            Ok(reply)
        }
        Command::Claim {
            reference,
            claim_slot,
        } => present(ops::claim(
            &mut store,
            &actor,
            &punch_core::github::RealGh,
            reference,
            claim_slot.as_deref(),
        )?),
        Command::Unclaim { reference } => present(ops::unclaim(
            &mut store,
            &actor,
            &punch_core::github::RealGh,
            reference,
        )?),
        Command::Board => Err(PunchError::usage("board is interactive; use list --json")),
        Command::Move { reference, column } => {
            present(ops::move_task(&mut store, &actor, reference, *column)?)
        }
        Command::Edit { reference, args: a } => present(ops::edit(
            &mut store,
            &actor,
            reference,
            ops::Edit {
                title: a.title.clone(),
                notes: a.notes.clone(),
                priority: a.priority,
                due: if a.clear_due {
                    Some(None)
                } else {
                    a.due.clone().map(Some)
                },
                after: if a.clear_after {
                    Some(vec![])
                } else {
                    a.after.clone()
                },
            },
        )?),
        Command::Note { reference, text } => {
            present(ops::note(&mut store, &actor, reference, text)?)
        }
        Command::Archive { reference } => present(ops::archive(&mut store, &actor, reference)?),
        Command::Unarchive { reference } => present(ops::unarchive(&mut store, &actor, reference)?),
        Command::Ready { project, limit } => present(ops::ready(
            &mut store,
            &actor,
            ops::Ready {
                project: project.clone(),
                limit: *limit,
            },
        )?),
        Command::Add(a) => {
            if let Some(path) = &a.file {
                let text = if let Some(plan) = plan {
                    plan.to_owned()
                } else if path == "-" {
                    std::io::read_to_string(std::io::stdin())?
                } else {
                    std::fs::read_to_string(path)?
                };
                let values: Vec<serde_json::Value> =
                    serde_json::from_str(&text).map_err(PunchError::usage)?;
                let items = values
                    .into_iter()
                    .map(|v| serde_json::from_value(v).map_err(PunchError::usage))
                    .collect();
                let outcome = ops::add_plan(&mut store, &actor, items)?;
                let failed = outcome
                    .data
                    .iter()
                    .any(|item| matches!(item, ops::PlanItem::Failed { .. }));
                let mut reply = present(outcome)?;
                reply.exit_code = u8::from(failed);
                Ok(reply)
            } else {
                present(ops::add(
                    &mut store,
                    &actor,
                    ops::Add {
                        title: a
                            .title
                            .clone()
                            .ok_or_else(|| PunchError::usage("title is required"))?,
                        project: a
                            .project
                            .clone()
                            .ok_or_else(|| PunchError::usage("project is required"))?,
                        notes: a.notes.clone(),
                        priority: a.priority,
                        due: a.due.clone(),
                        after: a.after.clone(),
                        assignee: a.assignee.clone(),
                        agent: a.agent,
                    },
                )?)
            }
        }
        Command::List(a) => present(ops::list(
            &mut store,
            &actor,
            ops::List {
                project: a.project.clone(),
                column: a.column,
                all: a.all,
            },
        )?),
        Command::Show { reference } => present(ops::show(&mut store, &actor, reference)?),
        Command::Project { command } => match command {
            ProjectCommand::Add {
                slug,
                name,
                github,
                scope,
            } => present(ops::project_add(
                &mut store,
                &actor,
                ops::ProjectAdd {
                    slug: slug.clone(),
                    name: name.clone(),
                    github: github.clone(),
                    scope: *scope,
                },
            )?),
            ProjectCommand::List => present(ops::project_list(&mut store, &actor)?),
            ProjectCommand::Archive { slug } => {
                present(ops::project_archive(&mut store, &actor, slug)?)
            }
        },
    }
}

pub fn print_text(data: &serde_json::Value) {
    if let Some(text) = data.as_str() {
        println!("{text}");
    } else if let Some(items) = data.as_array() {
        if items.is_empty() {
            println!("No items.");
        }
        for item in items {
            print_text(item);
        }
    } else if let Some(task) = data.get("task") {
        if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
            print!("{status}: ");
        } else if data["existing"] == true {
            print!("existing: ");
        }
        print_text(task);
    } else if data.get("cards").is_some() {
        println!(
            "{}: {} ({} cards)",
            data["project"].as_str().unwrap_or(""),
            data["status"].as_str().unwrap_or(""),
            data["cards"]
        );
        for r in data["refusals"].as_array().into_iter().flatten() {
            println!(
                "  {}: {}",
                r["code"].as_str().unwrap_or(""),
                r["message"].as_str().unwrap_or("")
            );
        }
    } else if data["status"] == "failed" {
        for r in data["refusals"].as_array().into_iter().flatten() {
            println!(
                "failed: {}: {}",
                r["code"].as_str().unwrap_or(""),
                r["message"].as_str().unwrap_or("")
            );
        }
    } else if let Some(title) = data.get("title").and_then(|v| v.as_str()) {
        println!(
            "t{}  {}  [{}]  p{}  {}",
            data["id"],
            data["project"].as_str().unwrap_or(""),
            data["column"].as_str().unwrap_or(""),
            data["priority"],
            title
        );
        if let Some(due) = data["due"].as_str() {
            println!("  Due: {due}");
        }
        if let Some(assignee) = data["assignee"].as_str() {
            println!("  Assignee: {assignee}");
        }
        if let Some(after) = data["after"].as_array().filter(|a| !a.is_empty()) {
            println!(
                "  After: {}",
                after
                    .iter()
                    .map(|id| format!("t{id}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if let Some(notes) = data["notes"].as_str().filter(|s| !s.is_empty()) {
            println!("  {notes}");
        }
        if let Some(url) = data["gh_cache"]["url"].as_str() {
            println!("  {url}");
        }
    } else if let Some(slug) = data["slug"].as_str() {
        println!(
            "{slug}  {}{}",
            data["name"].as_str().unwrap_or(""),
            if data["archived"] == true {
                " (archived)"
            } else {
                ""
            }
        );
    }
}
