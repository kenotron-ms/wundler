use std::io::BufRead;
use std::sync::Arc;
use std::thread;

use tempfile::tempdir;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_abs::types::TelemetryEvent;

fn make_event(session: &str, entry: &str) -> TelemetryEvent {
    TelemetryEvent {
        session_id: session.to_string(),
        entry_point: entry.to_string(),
        chunks_served: vec!["chunk-a".to_string()],
        client_had: vec![],
        timestamp_ms: 1_700_000_000_000,
    }
}

#[test]
fn writes_one_event_per_line() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("telemetry.jsonl");

    let logger = TelemetryLogger::new(&path).expect("open logger");
    logger.log(&make_event("session-1", "src/index.ts")).expect("log 1");
    logger.log(&make_event("session-2", "src/app.ts")).expect("log 2");
    drop(logger);

    let file = std::fs::File::open(&path).expect("open file");
    let lines: Vec<String> = std::io::BufReader::new(file)
        .lines()
        .map(|l| l.expect("read line"))
        .collect();

    assert_eq!(lines.len(), 2, "expected exactly 2 lines");
    // First line must parse cleanly as TelemetryEvent
    let parsed: TelemetryEvent =
        serde_json::from_str(&lines[0]).expect("first line must parse as TelemetryEvent");
    assert_eq!(parsed.session_id, "session-1");
}

#[test]
fn appends_to_existing_file() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("telemetry.jsonl");

    // First session: write one event and close
    {
        let logger = TelemetryLogger::new(&path).expect("open first logger");
        logger.log(&make_event("session-1", "src/index.ts")).expect("log 1");
    }

    // Second session: reopen and write another
    {
        let logger = TelemetryLogger::new(&path).expect("open second logger");
        logger.log(&make_event("session-2", "src/app.ts")).expect("log 2");
    }

    let file = std::fs::File::open(&path).expect("open file");
    let lines: Vec<String> = std::io::BufReader::new(file)
        .lines()
        .map(|l| l.expect("read line"))
        .collect();

    assert_eq!(lines.len(), 2, "expected 2 lines after two separate sessions");
    let first: TelemetryEvent =
        serde_json::from_str(&lines[0]).expect("first line parses");
    let second: TelemetryEvent =
        serde_json::from_str(&lines[1]).expect("second line parses");
    assert_eq!(first.session_id, "session-1");
    assert_eq!(second.session_id, "session-2");
}

#[test]
fn safe_under_parallel_writers() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("telemetry.jsonl");

    let logger = TelemetryLogger::new(&path).expect("open logger");
    let logger = Arc::new(logger);

    const THREADS: usize = 16;
    const EVENTS_PER_THREAD: usize = 32;

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let logger = Arc::clone(&logger);
            thread::spawn(move || {
                for i in 0..EVENTS_PER_THREAD {
                    let session = format!("thread-{}-event-{}", t, i);
                    logger
                        .log(&make_event(&session, "src/index.ts"))
                        .expect("parallel log");
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("thread panicked");
    }

    // Drop logger to flush
    drop(logger);

    let file = std::fs::File::open(&path).expect("open file");
    let lines: Vec<String> = std::io::BufReader::new(file)
        .lines()
        .map(|l| l.expect("read line"))
        .collect();

    assert_eq!(
        lines.len(),
        THREADS * EVENTS_PER_THREAD,
        "expected 512 lines from 16 threads × 32 events"
    );

    // Every line must parse cleanly
    for (i, line) in lines.iter().enumerate() {
        serde_json::from_str::<TelemetryEvent>(line)
            .unwrap_or_else(|e| panic!("line {} did not parse: {}\n  content: {:?}", i, e, line));
    }
}

#[test]
fn missing_parent_dir_is_created() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("nested").join("deep").join("t.jsonl");

    // The parent directories do not exist yet
    assert!(
        !path.parent().unwrap().exists(),
        "parent dir should not exist before logger creation"
    );

    let logger = TelemetryLogger::new(&path).expect("logger creates parent dirs");
    logger.log(&make_event("session-1", "src/index.ts")).expect("log");
    drop(logger);

    assert!(path.exists(), "log file should exist after writing");
}
