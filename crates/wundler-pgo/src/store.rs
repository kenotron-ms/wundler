//! SQLite-backed co-request matrix.
//!
//! One row per `(session_id, chunk_id)` pair, capturing the load
//! position of `chunk_id` within `session_id`. All clustering queries
//! reduce to SQL aggregates over this table.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id   TEXT    NOT NULL,
    entry_point  TEXT    NOT NULL,
    chunk_id     TEXT    NOT NULL,
    load_order   INTEGER NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, chunk_id)
);
CREATE INDEX IF NOT EXISTS idx_chunk ON sessions(chunk_id);
CREATE INDEX IF NOT EXISTS idx_entry ON sessions(entry_point, chunk_id);
";

pub struct PgoStore {
    pub(crate) conn: Mutex<Connection>,
}

impl PgoStore {
    /// Open a file-backed SQLite store at the given path, creating it if needed.
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open SQLite database at {:?}", db_path))?;
        Self::init(conn)
    }

    /// Open an in-memory SQLite store (data is lost when dropped).
    pub fn open_in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("failed to open in-memory SQLite database")?;
        Self::init(conn)
    }

    /// Apply the schema and wrap the connection in a mutex.
    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA_SQL)
            .context("failed to apply PGO store schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Return the number of distinct sessions currently stored.
    pub fn session_count(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("mutex poisoned");
        let count: usize = conn
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM sessions",
                [],
                |row| row.get(0),
            )
            .context("failed to count sessions")?;
        Ok(count)
    }

    /// Return the names of all user tables in the database.
    pub fn list_tables(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .context("failed to prepare list_tables query")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("failed to execute list_tables query")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .context("failed to collect table names")?;
        Ok(names)
    }

    /// Return the names of all non-internal indexes in the database.
    pub fn list_indexes(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'index' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .context("failed to prepare list_indexes query")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("failed to execute list_indexes query")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .context("failed to collect index names")?;
        Ok(names)
    }
}
