//! Subprocess deadlines shared by external CLI adapters.
use std::{os::unix::process::CommandExt, process::Command};

fn kill_group(id: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{id}")])
        .status();
}

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
    let mut child = command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let group = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing process stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "missing process stderr".to_owned())?;
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
                kill_group(group);
                let _ = child.wait();
                // Do not wait on inherited pipes held by grandchildren after killing the process.
                return Err(match result {
                    Err(e) => e.to_string(),
                    _ => format!("process timeout after {} ms", timeout.as_millis()),
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
                    format!("process timeout after {} ms", timeout.as_millis())
                }
                _ => "process output reader failed".into(),
            })?
            .map_err(|e| e.to_string())
    };
    let result = (|| {
        Ok(std::process::Output {
            status,
            stdout: collect(stdout)?,
            stderr: collect(stderr)?,
        })
    })();
    if result.is_err() {
        kill_group(group);
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn timeout_kills_forked_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant-survived");
        let mut command = Command::new("sh");
        command.env("PUNCH_STATE_DIR", dir.path()).args([
            "-c",
            "(sleep 0.3; touch \"$1\") &",
            "fake",
            marker.to_str().unwrap(),
        ]);
        assert!(
            run_command(command, std::time::Duration::from_millis(50))
                .unwrap_err()
                .contains("timeout")
        );
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(!marker.exists(), "forked descendant survived timeout");
    }
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
