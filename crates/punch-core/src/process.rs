//! Subprocess deadlines shared by external CLI adapters.
use std::{
    cell::RefCell,
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use std::{os::unix::process::CommandExt, process::Command};

#[derive(Default)]
struct ActionState {
    cancelled: bool,
    groups: BTreeSet<u32>,
}
/// Cancellation belongs to one board action, including all of its subsequent CLI calls.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<Mutex<ActionState>>);
thread_local! {
    static CURRENT: RefCell<Option<Cancellation>> = const { RefCell::new(None) };
}
impl Cancellation {
    /// Run existing ops with cancellation on this worker only; restore on unwind too.
    pub fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Cancellation>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| current.replace(self.0.take()));
            }
        }
        let _restore = Restore(CURRENT.with(|current| current.replace(Some(self.clone()))));
        work()
    }
    /// Serialize shutdown with spawn/registration so no child can escape that race.
    pub fn cancel(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.cancelled = true;
        for &group in &state.groups {
            kill_group(group);
        }
    }
}

struct ChildGroup {
    child: std::process::Child,
    cancellation: Option<Cancellation>,
    stopped: bool,
}
impl ChildGroup {
    fn spawn(mut command: Command) -> Result<Self, String> {
        let cancellation = CURRENT.with(|current| current.borrow().clone());
        let mut state = cancellation
            .as_ref()
            .map(|c| c.0.lock().unwrap_or_else(|e| e.into_inner()));
        if state.as_ref().is_some_and(|s| s.cancelled) {
            return Err("board action cancelled; read back before retrying".into());
        }
        let child = command.spawn().map_err(|e| e.to_string())?;
        if let Some(state) = &mut state {
            state.groups.insert(child.id());
        }
        drop(state);
        Ok(Self {
            child,
            cancellation,
            stopped: false,
        })
    }
    fn exited(&self) -> std::io::Result<bool> {
        // SAFETY: all-zero bytes are a valid siginfo_t (plain C struct of integers/unions).
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a live, writable siginfo_t for the whole call, and the id
        // and options are plain values; waitid retains no pointers after returning.
        // WEXITED makes si_pid valid, including zero when no status is ready yet.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(error);
        }
        // SAFETY: the successful WEXITED waitid above supplies this field.
        Ok(unsafe { info.si_pid() } != 0)
    }
    fn stop(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if self.stopped {
            return self.child.wait();
        }
        // WNOWAIT keeps even an exited leader's PID reserved by the OS. Kill and
        // deregister under the cancellation lock before the only reaping wait.
        let mut state = self
            .cancellation
            .as_ref()
            .map(|c| c.0.lock().unwrap_or_else(|e| e.into_inner()));
        kill_group(self.child.id());
        if let Some(state) = &mut state {
            state.groups.remove(&self.child.id());
        }
        drop(state);
        self.stopped = true;
        self.child.wait()
    }
    fn cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|c| c.0.lock().unwrap_or_else(|e| e.into_inner()).cancelled)
    }
}
impl Drop for ChildGroup {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

const OUTPUT_LIMIT: usize = 1024 * 1024;
#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}
impl Capture {
    fn diagnostic(&self) -> String {
        format!(
            "{}{}",
            if self.truncated {
                "[output truncated at 1 MiB] "
            } else {
                ""
            },
            String::from_utf8_lossy(&self.bytes)
        )
    }
}

fn kill_group(id: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{id}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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
    command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGroup::spawn(command)?;
    let stdout = child
        .child
        .stdout
        .take()
        .ok_or_else(|| "missing process stdout".to_owned())?;
    let stderr = child
        .child
        .stderr
        .take()
        .ok_or_else(|| "missing process stderr".to_owned())?;
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Capture::default()));
        let captured = bytes.clone();
        let worker = std::thread::spawn(move || -> std::io::Result<()> {
            let mut buffer = [0; 8192];
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                // Only this reader and the final diagnostic snapshot access the buffer.
                let mut capture = captured.lock().unwrap_or_else(|e| e.into_inner());
                let keep = count.min(OUTPUT_LIMIT.saturating_sub(capture.bytes.len()));
                capture.bytes.extend_from_slice(&buffer[..keep]);
                capture.truncated |= keep < count;
            }
        });
        (bytes, worker)
    };
    let (stdout, stdout_worker) = drain(Box::new(stdout));
    let (stderr, stderr_worker) = drain(Box::new(stderr));
    let start = Instant::now();
    let failure = loop {
        if child.cancelled() {
            break Some("board action cancelled; read back before retrying".into());
        }
        let exited = match child.exited() {
            Ok(exited) => exited,
            Err(error) => break Some(error.to_string()),
        };
        if exited && stdout_worker.is_finished() && stderr_worker.is_finished() {
            break None;
        }
        if start.elapsed() >= timeout {
            break Some(format!("process timeout after {} ms", timeout.as_millis()));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if let Some(error) = failure {
        let _ = child.stop();
        let stdout_truncated = stdout.lock().unwrap_or_else(|e| e.into_inner()).truncated;
        let bytes = stderr.lock().unwrap_or_else(|e| e.into_inner());
        return Err(format!(
            "{error}: {}{}",
            if stdout_truncated {
                "[stdout truncated at 1 MiB] "
            } else {
                ""
            },
            bytes.diagnostic()
        ));
    }
    for worker in [stdout_worker, stderr_worker] {
        worker
            .join()
            .map_err(|_| "process output reader failed".to_owned())?
            .map_err(|e| e.to_string())?;
    }
    let status = child.stop().map_err(|e| e.to_string())?;
    let snapshot = |bytes: std::sync::Arc<std::sync::Mutex<Capture>>| {
        std::mem::take(&mut *bytes.lock().unwrap_or_else(|e| e.into_inner()))
    };
    let stdout = snapshot(stdout);
    let stderr = snapshot(stderr);
    if stdout.truncated || stderr.truncated {
        return Err(format!(
            "process output truncated at 1 MiB per stream: {}{}",
            stderr.diagnostic(),
            stdout.diagnostic()
        ));
    }
    Ok(std::process::Output {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    // A separately grouped helper inherits the pipes after the command leader exits.
    #[test]
    #[allow(clippy::zombie_processes)] // Fixture must exit first; the outer test kills the helper.
    fn detached_pipe_owner_fixture() {
        if std::env::var_os("PUNCH_PIPE_OWNER_FIXTURE").is_none() {
            return;
        }
        let root = std::path::PathBuf::from(std::env::var_os("PUNCH_STATE_DIR").unwrap());
        let helper = Command::new("sleep")
            .arg("15")
            .process_group(0)
            .spawn()
            .unwrap();
        std::fs::write(root.join("helper"), helper.id().to_string()).unwrap();
        std::fs::write(root.join("leader"), std::process::id().to_string()).unwrap();
    }
    #[test]
    fn exited_leader_keeps_identity_until_success_or_cancel_cleanup() {
        use std::time::{Duration, Instant};
        for cancelled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let cancellation = Cancellation::default();
            let worker_cancellation = cancellation.clone();
            let root = dir.path().to_owned();
            let worker = std::thread::spawn(move || {
                worker_cancellation.run(|| {
                    let mut command = Command::new(std::env::current_exe().unwrap());
                    command
                        .env("PUNCH_STATE_DIR", root)
                        .env("PUNCH_PIPE_OWNER_FIXTURE", "1")
                        .args([
                            "--exact",
                            "process::tests::detached_pipe_owner_fixture",
                            "--nocapture",
                        ]);
                    run_command(command, Duration::from_secs(3))
                })
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            let leader = loop {
                if let Ok(value) = std::fs::read_to_string(dir.path().join("leader"))
                    && value.parse::<u32>().is_ok()
                {
                    break value;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            let helper = std::fs::read_to_string(dir.path().join("helper")).unwrap();
            let mut reserved = false;
            while Instant::now() < deadline {
                let output = Command::new("ps")
                    .args(["-p", &leader, "-o", "stat="])
                    .output()
                    .unwrap();
                let state = String::from_utf8(output.stdout).unwrap();
                if state.trim().is_empty() {
                    break;
                }
                if state.trim().starts_with('Z') {
                    reserved = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // Allow the command observer to run after the leader exits.
            std::thread::sleep(Duration::from_millis(50));
            let output = Command::new("ps")
                .args(["-p", &leader, "-o", "stat="])
                .output()
                .unwrap();
            reserved &= String::from_utf8(output.stdout)
                .unwrap()
                .trim()
                .starts_with('Z');
            let mut unrelated = Command::new("sleep")
                .arg("15")
                .process_group(0)
                .spawn()
                .unwrap();
            let stop_helper = || {
                let _ = Command::new("/bin/kill").args(["-KILL", &helper]).status();
            };
            if cancelled {
                cancellation.cancel();
            } else {
                stop_helper();
            }
            let result = worker.join().unwrap();
            let survived = unrelated.try_wait().unwrap().is_none();
            let _ = unrelated.kill();
            let _ = unrelated.wait();
            if cancelled {
                stop_helper();
            }
            assert!(
                reserved,
                "leader was reaped while its group was still registered (cancel={cancelled})"
            );
            assert!(survived, "unrelated group was killed (cancel={cancelled})");
            if cancelled {
                assert!(result.unwrap_err().contains("cancelled"));
            } else {
                assert!(result.unwrap().status.success());
            }
            let output = Command::new("ps")
                .args(["-p", &leader, "-o", "stat="])
                .output()
                .unwrap();
            assert!(output.stdout.is_empty(), "leader not reaped after cleanup");
        }
    }
    #[test]
    fn cancelled_action_cannot_spawn_later_commands_or_cancel_unrelated_work() {
        let dir = tempfile::tempdir().unwrap();
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let command = || {
            let mut command = Command::new("sh");
            command
                .env("PUNCH_STATE_DIR", dir.path())
                .args(["-c", "touch \"$PUNCH_STATE_DIR/late-write\""]);
            command
        };
        let error = cancellation
            .run(|| run_command(command(), std::time::Duration::from_secs(30)))
            .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(!dir.path().join("late-write").exists());
        // Cancellation is action-scoped, including after unwind on the same thread.
        let _ = std::panic::catch_unwind(|| cancellation.run(|| panic!("scope unwind")));
        run_command(command(), std::time::Duration::from_secs(1)).unwrap();
        assert!(dir.path().join("late-write").exists());
    }
    #[test]
    fn shutdown_racing_spawn_kills_the_group_and_refuses_the_next_command() {
        use std::{
            sync::Barrier,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..30 {
            let cancellation = Cancellation::default();
            let worker_cancellation = cancellation.clone();
            let barrier = Arc::new(Barrier::new(2));
            let worker_barrier = barrier.clone();
            let root = dir.path().to_owned();
            let worker = std::thread::spawn(move || {
                worker_cancellation.run(|| {
                    worker_barrier.wait();
                    let script = "while test ! -e \"$PUNCH_STATE_DIR/release\"; do sleep 0.01; done; touch \"$PUNCH_STATE_DIR/late-write\"";
                    let mut command = Command::new("sh");
                    command.env("PUNCH_STATE_DIR", &root).args(["-c", script]);
                    assert!(run_command(command, Duration::from_secs(30)).is_err());
                    let mut next = Command::new("sh");
                    next.env("PUNCH_STATE_DIR", &root)
                        .args(["-c", "touch \"$PUNCH_STATE_DIR/next-command\""]);
                    assert!(run_command(next, Duration::from_secs(30))
                        .unwrap_err().contains("cancelled"));
                })
            });
            barrier.wait();
            let start = Instant::now();
            cancellation.cancel();
            std::fs::write(dir.path().join("release"), "").unwrap();
            worker.join().unwrap();
            assert!(start.elapsed() < Duration::from_secs(1));
            assert!(!dir.path().join("late-write").exists());
            assert!(!dir.path().join("next-command").exists());
            std::fs::remove_file(dir.path().join("release")).unwrap();
        }
    }
    #[test]
    fn large_output_is_bounded_and_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();
        for stream in ["", " >&2"] {
            let mut command = Command::new("sh");
            command.env("PUNCH_STATE_DIR", dir.path()).args([
                "-c",
                &format!("head -c 4194304 /dev/zero{stream}; echo finished"),
            ]);
            let error = run_command(command, std::time::Duration::from_secs(3))
                .expect_err("oversized output must fail");
            assert!(
                error.contains("truncated"),
                "{}",
                &error[..error.len().min(100)]
            );
            assert!(error.len() < 2 * 1024 * 1024 + 256);
        }
    }
    #[test]
    fn timeout_reports_truncated_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command
            .env("PUNCH_STATE_DIR", dir.path())
            .args(["-c", "head -c 4194304 /dev/zero; exec sleep 2"]);
        let error = run_command(command, std::time::Duration::from_millis(200)).unwrap_err();
        assert!(error.contains("timeout"));
        assert!(error.contains("stdout truncated"), "{error}");
    }
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
