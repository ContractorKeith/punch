//! Shell adapter, with pagination for timeline and closing-reference connections.
use super::*;
use crate::error::{GH_FAILED, OFFLINE};
use serde_json::{Value, json};
use std::{collections::BTreeSet, process::Command};

#[derive(Default)]
pub struct RealGh;
impl RealGh {
    fn run(&self, args: &[&str]) -> Result<Vec<u8>, PunchError> {
        let mut command = Command::new("gh");
        command.args(args).env("GH_PROMPT_DISABLED", "1");
        let out = run_command(command, std::time::Duration::from_secs(30))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let lower = stderr.to_lowercase();
            let code = if [
                "network",
                "connection",
                "error connecting",
                "no such host",
                "timeout",
                "timed out",
                "could not resolve",
                "offline",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                OFFLINE
            } else {
                GH_FAILED
            };
            return Err(PunchError::refusal(code, "gh", stderr));
        }
        Ok(out.stdout)
    }
    fn json(&self, args: &[&str]) -> Result<Value, PunchError> {
        serde_json::from_slice(&self.run(args)?)
            .map_err(|e| PunchError::refusal(GH_FAILED, "gh", format!("invalid gh JSON: {e}")))
    }
    fn item(&self, mut value: Value, kind: Kind) -> Result<Item, PunchError> {
        value
            .as_object_mut()
            .ok_or_else(|| PunchError::refusal(GH_FAILED, "gh", "expected GitHub card object"))?
            .insert("kind".into(), json!(kind));
        serde_json::from_value(value)
            .map_err(|e| PunchError::refusal(GH_FAILED, "gh", format!("invalid gh card: {e}")))
    }
    fn fields(kind: Kind) -> &'static str {
        if kind == Kind::Pr {
            "number,title,state,assignees,url,updatedAt,isDraft,reviewDecision,author"
        } else {
            "number,title,state,assignees,url,updatedAt"
        }
    }
    fn connection(&self, repo: &str, number: i64, field: &str) -> Result<Vec<Value>, PunchError> {
        let (owner, name) = repo
            .split_once('/')
            .filter(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'))
            .ok_or_else(|| PunchError::usage("GitHub repository must be owner/repo"))?;
        let selection = if field == "issue" {
            "timelineItems(first:100,after:$cursor,itemTypes:[CROSS_REFERENCED_EVENT]) { nodes { ... on CrossReferencedEvent { source { ... on PullRequest { number repository { nameWithOwner } } } } } pageInfo { hasNextPage endCursor } }"
        } else {
            "closingIssuesReferences(first:100,after:$cursor) { nodes { number repository { nameWithOwner } } pageInfo { hasNextPage endCursor } }"
        };
        let query = format!(
            "query($owner:String!,$name:String!,$number:Int!,$cursor:String) {{ repository(owner:$owner,name:$name) {{ {field}(number:$number) {{ {selection} }} }} }}"
        );
        let mut cursor: Option<String> = None;
        let mut nodes = vec![];
        loop {
            let owner = format!("owner={owner}");
            let name = format!("name={name}");
            let number = format!("number={number}");
            let mut args = vec![
                "api",
                "graphql",
                "-f",
                &format!("query={query}"),
                "-f",
                &owner,
                "-f",
                &name,
                "-F",
                &number,
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
            if let Some(c) = &cursor {
                args.extend(["-f".into(), format!("cursor={c}")]);
            }
            let data = self.json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            if data.get("errors").is_some() {
                return Err(PunchError::refusal(GH_FAILED, repo, data.to_string()));
            }
            let connection = &data["data"]["repository"][field][if field == "issue" {
                "timelineItems"
            } else {
                "closingIssuesReferences"
            }];
            let page = connection["nodes"]
                .as_array()
                .ok_or_else(|| PunchError::refusal(GH_FAILED, repo, "missing GitHub connection"))?;
            nodes.extend(page.iter().cloned());
            if connection["pageInfo"]["hasNextPage"] == false {
                break;
            }
            let next = connection["pageInfo"]["endCursor"]
                .as_str()
                .ok_or_else(|| PunchError::refusal(GH_FAILED, repo, "missing pagination cursor"))?
                .to_owned();
            if cursor.as_ref() == Some(&next) {
                return Err(PunchError::refusal(
                    GH_FAILED,
                    repo,
                    "repeated pagination cursor",
                ));
            }
            cursor = Some(next);
        }
        Ok(nodes)
    }
    fn closing(&self, repo: &str, number: i64) -> Result<Vec<i64>, PunchError> {
        self.closing_for(repo, number, repo)
    }
    fn closing_for(
        &self,
        repo: &str,
        number: i64,
        issue_repo: &str,
    ) -> Result<Vec<i64>, PunchError> {
        Ok(self
            .connection(repo, number, "pullRequest")?
            .iter()
            .filter(|n| {
                n["repository"]["nameWithOwner"]
                    .as_str()
                    .is_some_and(|r| r.eq_ignore_ascii_case(issue_repo))
            })
            .filter_map(|n| n["number"].as_i64())
            .collect())
    }
}
impl Gh for RealGh {
    fn list(&self, repo: &str, scope: SyncScope) -> Result<Vec<Item>, PunchError> {
        let mut items = vec![];
        let viewer = if scope == SyncScope::Assigned {
            let data = self.json(&["api", "graphql", "-f", "query=query { viewer { login } }"])?;
            Some(
                data["data"]["viewer"]["login"]
                    .as_str()
                    .ok_or_else(|| PunchError::refusal(GH_FAILED, repo, "missing GitHub viewer"))?
                    .to_owned(),
            )
        } else {
            None
        };
        for (kind, command) in [(Kind::Issue, "issue"), (Kind::Pr, "pr")] {
            // gh paginates internally; avoid its default 30-card limit.
            let mut args = vec![
                command,
                "list",
                "--repo",
                repo,
                "--state",
                "open",
                "--limit",
                "2147483647",
                "--json",
                Self::fields(kind),
            ];
            if scope == SyncScope::Assigned && kind == Kind::Issue {
                args.extend(["--assignee", "@me"]);
            }
            let values = self.json(&args)?;
            for value in values
                .as_array()
                .ok_or_else(|| PunchError::refusal(GH_FAILED, repo, "expected GitHub card array"))?
            {
                let mut item = self.item(value.clone(), kind)?;
                // Filter authors locally: gh's --author uses search, which caps results at 1,000.
                if kind == Kind::Pr {
                    if viewer.as_ref().is_some_and(|v| {
                        item.author
                            .as_ref()
                            .is_none_or(|a| !a.login.eq_ignore_ascii_case(v))
                    }) {
                        continue;
                    }
                    item.closes = self.closing(repo, item.number)?;
                }
                items.push(item);
            }
        }
        Ok(items)
    }
    fn get(&self, repo: &str, kind: Kind, number: i64) -> Result<Item, PunchError> {
        let value = self.json(&[
            if kind == Kind::Pr { "pr" } else { "issue" },
            "view",
            &number.to_string(),
            "--repo",
            repo,
            "--json",
            Self::fields(kind),
        ])?;
        let mut item = self.item(value, kind)?;
        if kind == Kind::Pr {
            item.closes = self.closing(repo, number)?;
        }
        Ok(item)
    }
    fn linked(&self, repo: &str, issue: i64) -> Result<Vec<Item>, PunchError> {
        let numbers: BTreeSet<_> = self
            .connection(repo, issue, "issue")?
            .iter()
            .filter_map(|n| {
                Some((
                    n["source"]["repository"]["nameWithOwner"]
                        .as_str()?
                        .to_owned(),
                    n["source"]["number"].as_i64()?,
                ))
            })
            .collect();
        let mut prs = vec![];
        for (source, n) in numbers {
            let mut pr = self.get(&source, Kind::Pr, n)?;
            if !source.eq_ignore_ascii_case(repo) {
                pr.closes = self.closing_for(&source, n, repo)?;
            }
            if pr.closes.contains(&issue) {
                prs.push(pr);
            }
        }
        Ok(prs)
    }
    fn assign(&self, repo: &str, number: i64, claim: bool) -> Result<(), PunchError> {
        self.run(&[
            "issue",
            "edit",
            &number.to_string(),
            "--repo",
            repo,
            if claim {
                "--add-assignee"
            } else {
                "--remove-assignee"
            },
            "@me",
        ])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hung_gh_is_killed_at_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        let mut command = Command::new("sh");
        command.env("PUNCH_STATE_DIR", dir.path()).args([
            "-c",
            "sleep 0.4; touch \"$1\"",
            "gh",
            marker.to_str().unwrap(),
        ]);
        let start = std::time::Instant::now();
        let error = run_command(command, std::time::Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.code, GH_FAILED);
        assert!(error.message.contains("timeout"));
        assert!(start.elapsed() < std::time::Duration::from_millis(300));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!marker.exists(), "the timed out gh survived");
    }
}

// Drain both pipes while polling: large gh responses must not block on pipe capacity.
fn run_command(
    mut command: Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, PunchError> {
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    let failed = |e: String| PunchError::refusal(GH_FAILED, "gh", e);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| failed(e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("missing gh stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| failed("missing gh stderr".into()))?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = vec![];
            pipe.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = drain(Box::new(stdout));
    let stderr = drain(Box::new(stderr));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                // Do not wait on inherited pipes held by grandchildren after killing gh.
                return Err(match result {
                    Err(e) => failed(e.to_string()),
                    _ => failed(format!("gh timeout after {} ms", timeout.as_millis())),
                });
            }
        }
    };
    let collect = |handle: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        handle
            .join()
            .map_err(|_| failed("gh output reader failed".into()))?
            .map_err(|e| failed(e.to_string()))
    };
    Ok(std::process::Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}
