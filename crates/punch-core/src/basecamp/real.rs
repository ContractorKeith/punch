//! basecamp 0.21: card create/move/update with explicit project and table scope.
use super::Basecamp;
use crate::model::BasecampLink;
use serde_json::Value;
use std::process::{Command, Stdio};

pub struct RealBasecamp;

fn run(args: &[&str]) -> Result<Value, String> {
    run_raw(args).map_err(|e| super::diagnostic(&e))
}

fn run_raw(args: &[&str]) -> Result<Value, String> {
    let mut command = Command::new("basecamp");
    command
        .args(args)
        .env("BASECAMP_NONINTERACTIVE", "1")
        .stdin(Stdio::null());
    let output = crate::process::run_command(command, std::time::Duration::from_secs(30))
        .map_err(|e| format!("basecamp: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "basecamp {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid basecamp JSON: {e}"))?;
    if value["ok"] != true || !(value["data"].is_object() || value["data"].is_array()) {
        return Err(format!("invalid basecamp response: {value}"));
    }
    Ok(value["data"].clone())
}

impl Basecamp for RealBasecamp {
    fn find_card(&self, link: &BasecampLink, title: &str) -> Result<Option<String>, String> {
        let data = run(&[
            "cards",
            "list",
            "--project",
            &link.bucket_id,
            "--card-table",
            &link.card_table_id,
            "--all",
            "--json",
        ])?;
        let cards = data.as_array().ok_or("invalid basecamp card list")?;
        cards
            .iter()
            .find(|card| card["title"].as_str() == Some(title))
            .map(card_id)
            .transpose()
    }

    fn create(&self, link: &BasecampLink, column: &str, title: &str) -> Result<String, String> {
        let card = run(&[
            "cards",
            "create",
            "--project",
            &link.bucket_id,
            "--card-table",
            &link.card_table_id,
            "--column",
            column,
            "--json",
            "--",
            title,
        ])?;
        card_id(&card)
    }

    fn move_card(&self, link: &BasecampLink, id: &str, column: &str) -> Result<(), String> {
        run(&[
            "cards",
            "move",
            id,
            "--project",
            &link.bucket_id,
            "--card-table",
            &link.card_table_id,
            "--to",
            column,
            "--json",
        ])?;
        Ok(())
    }
    fn update_title(&self, link: &BasecampLink, id: &str, title: &str) -> Result<(), String> {
        run(&[
            "cards",
            "update",
            id,
            "--project",
            &link.bucket_id,
            "--card-table",
            &link.card_table_id,
            "--title",
            title,
            "--json",
        ])?;
        Ok(())
    }
}

fn card_id(card: &Value) -> Result<String, String> {
    match &card["id"] {
        Value::Number(n) if n.as_u64().is_some_and(|id| id > 0) => Ok(n.to_string()),
        Value::String(s) if s.parse::<u64>().is_ok_and(|id| id > 0) => Ok(s.clone()),
        _ => Err("basecamp returned no valid card ID".into()),
    }
}
