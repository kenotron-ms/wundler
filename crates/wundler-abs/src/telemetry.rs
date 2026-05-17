//! Append-only JSONL telemetry log.
//!
//! [`TelemetryLogger`] writes one JSON line per serializable event to a file.
//! The underlying [`std::io::BufWriter`] is wrapped in a [`std::sync::Mutex`]
//! and shared via [`std::sync::Arc`], so **clones share the same writer** and
//! concurrent calls from multiple threads never produce torn lines.
//!
//! The file is opened in **append** mode, so multiple restarts of the process
//! accumulate into the same file rather than overwriting it.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::Serialize;

/// Append-only JSONL telemetry logger.
///
/// Cheap to clone — clones share the same underlying writer through an
/// `Arc<Mutex<_>>`, so every clone writes to the same file in a
/// thread-safe manner.
pub struct TelemetryLogger {
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl TelemetryLogger {
    /// Open (or create) the JSONL log at `path`.
    ///
    /// Any missing parent directories are created automatically.
    /// The file is opened with `create(true).append(true)` so existing
    /// content is preserved across restarts.
    pub fn new(path: &Path) -> Result<Self> {
        // Create parent directories if needed. An empty parent component (e.g.
        // when `path` is just a filename with no directory separator) is handled
        // by checking whether the parent is non-empty before calling
        // `create_dir_all`.
        if let Some(parent) = path.parent() {
            // `parent()` on "file.jsonl" returns `Some("")` — skip that case.
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
        })
    }

    /// Serialize `event` as a single JSON line and flush to disk immediately.
    ///
    /// The lock is held for the entire write + flush so that concurrent
    /// callers never interleave bytes and produce a torn line.
    pub fn log<T: Serialize>(&self, event: &T) -> Result<()> {
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');

        let mut guard = self.writer.lock().expect("telemetry lock poisoned");
        guard.write_all(&line)?;
        guard.flush()?;
        Ok(())
    }
}

/// Clones share the same underlying writer — no separate file handle is opened.
impl Clone for TelemetryLogger {
    fn clone(&self) -> Self {
        Self {
            writer: Arc::clone(&self.writer),
        }
    }
}

/// Best-effort flush on drop — swallows lock errors so `Drop` never panics.
impl Drop for TelemetryLogger {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.writer.lock() {
            let _ = guard.flush();
        }
    }
}
