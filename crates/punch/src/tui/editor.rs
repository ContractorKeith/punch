//! Private temporary notes file and synchronous editor invocation.
use punch_core::error::PunchError;
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
struct NotesFile(PathBuf);
impl Drop for NotesFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn edit_with(editor: &str, notes: &str) -> Result<String, PunchError> {
    if editor.trim().is_empty() {
        return Err(PunchError::usage("set $EDITOR to edit notes"));
    }
    let (path, mut file) = loop {
        let path = std::env::temp_dir().join(format!(
            "punch-notes-{}-{}.txt",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => break (NotesFile(path), file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    file.write_all(notes.as_bytes())?;
    drop(file);
    // EDITOR is a user-owned shell command; the file is passed as a positional arg.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("punch-editor")
        .arg(&path.0)
        .status()?;
    if !status.success() {
        return Err(PunchError::usage(format!(
            "editor exited with {status}; notes were not saved"
        )));
    }
    Ok(std::fs::read_to_string(&path.0)?)
}
