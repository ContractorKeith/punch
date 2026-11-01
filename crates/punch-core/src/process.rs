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
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = bytes.clone();
        let worker = std::thread::spawn(move || -> std::io::Result<()> {
            let mut buffer = [0; 8192];
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                // Only this reader and the final diagnostic snapshot access the buffer.
                captured
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(&buffer[..count]);
            }
        });
        (bytes, worker)
    };
    let (stdout, stdout_worker) = drain(Box::new(stdout));
    let (stderr, stderr_worker) = drain(Box::new(stderr));
    let start = Instant::now();
    let mut status = None;
    let failure = loop {
        match child.try_wait() {
            Ok(current) => {
                if current.is_some() {
                    status = current;
                }
            }
            Err(error) => break Some(error.to_string()),
        }
        if status.is_some() && stdout_worker.is_finished() && stderr_worker.is_finished() {
            break None;
        }
        if start.elapsed() >= timeout {
            break Some(format!("process timeout after {} ms", timeout.as_millis()));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if let Some(error) = failure {
        kill_group(group);
        let _ = child.wait();
        let bytes = stderr.lock().unwrap_or_else(|e| e.into_inner());
        return Err(format!("{error}: {}", String::from_utf8_lossy(&bytes)));
    }
    for worker in [stdout_worker, stderr_worker] {
        worker
            .join()
            .map_err(|_| "process output reader failed".to_owned())?
            .map_err(|e| e.to_string())?;
    }
    let snapshot = |bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>| {
        std::mem::take(&mut *bytes.lock().unwrap_or_else(|e| e.into_inner()))
    };
    Ok(std::process::Output {
        status: status.ok_or("missing process status")?,
        stdout: snapshot(stdout),
        stderr: snapshot(stderr),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn timeout_preserves_stderr() {
        let dir = tempfile::tempdir().unwrap();
        for script in [
            "echo diagnostic-before-timeout >&2; exec sleep 2",
            "echo diagnostic-before-timeout >&2; sleep 2 &",
        ] {
            let mut command = Command::new("sh");
            command
                .env("PUNCH_STATE_DIR", dir.path())
                .args(["-c", script]);
            let error = run_command(command, std::time::Duration::from_millis(50)).unwrap_err();
            assert!(error.contains("timeout"));
            assert!(error.contains("diagnostic-before-timeout"), "{error}");
        }
    }
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
