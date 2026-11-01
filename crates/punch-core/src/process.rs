//! Subprocess deadlines shared by external CLI adapters.
use std::process::Command;

// Drain both pipes while polling: large process responses must not block on pipe capacity.
pub(crate) fn run_command(
    mut command: Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    let failed = |e: String| e;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| failed(e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("missing process stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| failed("missing process stderr".into()))?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let result = pipe.read_to_end(&mut bytes).map(|_| bytes);
            let _ = send.send(result);
        });
        receive
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
                    _ => failed(format!("process timeout after {} ms", timeout.as_millis())),
                });
            }
        }
    };
    // A process descendant can inherit a pipe even after process exits; draining shares the deadline.
    let collect = |receive: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        receive
            .recv_timeout(timeout.saturating_sub(start.elapsed()))
            .map_err(|e| match e {
                std::sync::mpsc::RecvTimeoutError::Timeout => {
                    failed(format!("process timeout after {} ms", timeout.as_millis()))
                }
                _ => failed("process output reader failed".into()),
            })?
            .map_err(|e| failed(e.to_string()))
    };
    Ok(std::process::Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn inherited_gh_output_pipes_obey_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command
            .env("PUNCH_STATE_DIR", dir.path())
            .args(["-c", "sleep 0.4 &"]);
        let start = std::time::Instant::now();
        let error = run_command(command, std::time::Duration::from_millis(50)).unwrap_err();
        assert!(error.contains("timeout"));
        assert!(start.elapsed() < std::time::Duration::from_millis(300));
    }
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
        assert!(error.contains("timeout"));
        assert!(start.elapsed() < std::time::Duration::from_millis(300));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!marker.exists(), "the timed out process survived");
    }
}
