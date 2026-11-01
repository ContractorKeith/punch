//! SQLite connection, migrations, and queries. All SQL lives here.
mod queries;
use crate::error::{PunchError, STORE_TOO_NEW};
pub(crate) use queries::Repository;
use rusqlite::{Connection, TransactionBehavior};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open() -> Result<Self, PunchError> {
        let root = match std::env::var_os("PUNCH_STATE_DIR") {
            Some(path) => PathBuf::from(path),
            None => dirs::data_dir()
                .ok_or_else(|| PunchError::storage("no data directory"))?
                .join("punch"),
        };
        Self::open_at(&root)
    }
    pub fn open_at(root: &Path) -> Result<Self, PunchError> {
        std::fs::create_dir_all(root)?;
        let mut connection = Connection::open(root.join("punch.db"))?;
        connection.busy_timeout(Duration::from_millis(5000))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        // Check the version before changing journal mode on an unsupported store.
        if schema_version(&connection)? != MIGRATIONS.len() as i64 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // Another opener may have migrated while this connection waited.
            let version = schema_version(&tx)?;
            if version > MIGRATIONS.len() as i64 {
                return Err(PunchError {
                    exit_code: 3,
                    ..PunchError::refusal(
                        STORE_TOO_NEW,
                        version,
                        "store schema is newer than this binary",
                    )
                });
            }
            if version < MIGRATIONS.len() as i64 {
                tx.execute_batch(
                    "CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);",
                )?;
                for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
                    tx.execute_batch(sql)?;
                    tx.execute(
                        "INSERT INTO schema_version(version) VALUES(?1)",
                        [(index + 1) as i64],
                    )?;
                }
            }
            tx.commit()?;
        }
        // Journal-mode changes can return BUSY without calling SQLite's busy handler.
        let deadline = Instant::now() + Duration::from_millis(5000);
        loop {
            match connection.pragma_update(None, "journal_mode", "WAL") {
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::DatabaseBusy
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => {
                    result?;
                    break;
                }
            }
        }
        Ok(Self { connection })
    }
    /// Read a consistent WAL snapshot without reserving the write lock.
    pub(crate) fn read_transaction<T>(
        &mut self,
        f: impl FnOnce(&Repository<'_>) -> Result<T, PunchError>,
    ) -> Result<T, PunchError> {
        self.transaction_with_behavior(TransactionBehavior::Deferred, f)
    }
    /// Serialize mutating operations before reading their current state.
    pub(crate) fn transaction<T>(
        &mut self,
        f: impl FnOnce(&Repository<'_>) -> Result<T, PunchError>,
    ) -> Result<T, PunchError> {
        self.transaction_with_behavior(TransactionBehavior::Immediate, f)
    }
    fn transaction_with_behavior<T>(
        &mut self,
        behavior: TransactionBehavior,
        f: impl FnOnce(&Repository<'_>) -> Result<T, PunchError>,
    ) -> Result<T, PunchError> {
        let tx = self.connection.transaction_with_behavior(behavior)?;
        let result = f(&Repository::new(&tx))?;
        tx.commit()?;
        Ok(result)
    }
}

fn schema_version(connection: &Connection) -> Result<i64, PunchError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |r| r.get(0),
    )?;
    if exists {
        Ok(connection.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_version",
            [],
            |r| r.get(0),
        )?)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests;
