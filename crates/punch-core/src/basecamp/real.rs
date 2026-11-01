//! basecamp 0.21: card create/move/update with explicit project and table scope.
use super::Basecamp;
use crate::model::BasecampLink;
use serde_json::Value;
use std::process::{Command, Stdio};

pub struct RealBasecamp;

fn run(args: &[&str]) -> Result<Value, String> {
    let output = Command::new("basecamp")
        .args(args)
        .env("BASECAMP_NONINTERACTIVE", "1")
        .stdin(Stdio::null())
        .output()
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
    if value["ok"] != true || !value["data"].is_object() {
        return Err(format!("invalid basecamp response: {value}"));
    }
    Ok(value["data"].clone())
}

impl Basecamp for RealBasecamp {
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
        match &card["id"] {
            Value::Number(n) if n.as_u64().is_some_and(|id| id > 0) => Ok(n.to_string()),
            Value::String(s) if s.parse::<u64>().is_ok_and(|id| id > 0) => Ok(s.clone()),
            _ => Err("basecamp create returned no valid card ID".into()),
        }
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
