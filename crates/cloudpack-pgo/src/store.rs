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

    /// Insert one session as N rows (one per chunk in `chunk_sequence`).
    /// Returns the number of rows inserted. Duplicate `(session_id, chunk_id)`
    /// pairs are silently skipped (see Task 5).
    pub fn insert_session(&self, record: &crate::types::SessionRecord) -> Result<usize> {
        let mut conn = self.conn.lock().expect("PGO store mutex poisoned");
        let tx = conn.transaction().context("begin insert_session tx")?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR IGNORE INTO sessions \
                     (session_id, entry_point, chunk_id, load_order, timestamp_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .context("prepare insert")?;
            for (i, chunk_id) in record.chunk_sequence.iter().enumerate() {
                let n = stmt
                    .execute(rusqlite::params![
                        record.session_id,
                        record.entry_point,
                        chunk_id,
                        i as i64,
                        record.timestamp_ms as i64,
                    ])
                    .context("execute insert")?;
                inserted += n;
            }
        }
        tx.commit().context("commit insert_session tx")?;
        Ok(inserted)
    }

    /// How many distinct sessions loaded `chunk_id`.
    pub fn chunk_load_count(&self, chunk_id: &str) -> Result<usize> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM sessions WHERE chunk_id = ?1",
                [chunk_id],
                |row| row.get(0),
            )
            .context("chunk_load_count query")?;
        Ok(n as usize)
    }

    /// How many distinct sessions loaded BOTH `chunk_a` and `chunk_b`.
    pub fn co_load_count(&self, chunk_a: &str, chunk_b: &str) -> Result<usize> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT a.session_id) \
                 FROM sessions a \
                 JOIN sessions b ON a.session_id = b.session_id \
                 WHERE a.chunk_id = ?1 AND b.chunk_id = ?2",
                rusqlite::params![chunk_a, chunk_b],
                |row| row.get(0),
            )
            .context("co_load_count query")?;
        Ok(n as usize)
    }

    /// Median load_order (0-based position) of `chunk_id` across all sessions
    /// that loaded it.  Returns `0.0` if the chunk has never been loaded.
    pub fn median_load_order(&self, chunk_id: &str) -> Result<f64> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT load_order FROM sessions WHERE chunk_id = ?1 ORDER BY load_order ASC",
            )
            .context("prepare median_load_order")?;
        let values: Vec<i64> = stmt
            .query_map([chunk_id], |row| row.get::<_, i64>(0))
            .context("query median_load_order")?
            .collect::<rusqlite::Result<Vec<i64>>>()
            .context("collect median_load_order")?;
        if values.is_empty() {
            return Ok(0.0);
        }
        let n = values.len();
        if n % 2 == 1 {
            Ok(values[n / 2] as f64)
        } else {
            Ok((values[n / 2 - 1] + values[n / 2]) as f64 / 2.0)
        }
    }

    /// How many distinct sessions loaded `chunk_id` at `load_order < within_n`.
    pub fn initial_load_count(&self, chunk_id: &str, within_n: usize) -> Result<usize> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM sessions \
                 WHERE chunk_id = ?1 AND load_order < ?2",
                rusqlite::params![chunk_id, within_n as i64],
                |row| row.get(0),
            )
            .context("initial_load_count query")?;
        Ok(n as usize)
    }

    /// All distinct chunk IDs present in the store, sorted alphabetically.
    pub fn all_chunk_ids(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT DISTINCT chunk_id FROM sessions ORDER BY chunk_id")
            .context("prepare all_chunk_ids")?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("query all_chunk_ids")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .context("collect all_chunk_ids")?;
        Ok(ids)
    }

    /// Returns `(min_timestamp_ms, max_timestamp_ms)` across all rows, or
    /// `None` if the store is empty.
    pub fn date_range_ms(&self) -> Result<Option<(u64, u64)>> {
        let conn = self.conn.lock().expect("PGO store mutex poisoned");
        let row: rusqlite::Result<(Option<i64>, Option<i64>)> = conn.query_row(
            "SELECT MIN(timestamp_ms), MAX(timestamp_ms) FROM sessions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match row {
            Ok((Some(min), Some(max))) => Ok(Some((min as u64, max as u64))),
            _ => Ok(None),
        }
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
