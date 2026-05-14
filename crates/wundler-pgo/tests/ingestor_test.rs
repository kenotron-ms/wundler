use std::io::Write;

use tempfile::NamedTempFile;
use wundler_pgo::ingestor::ingest_log;
use wundler_pgo::store::PgoStore;

fn write_jsonl(lines: &[&str]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    for line in lines {
        writeln!(f, "{line}").unwrap();
    }
    f
}

const EVENT_A: &str = r#"{"session_id":"s1","entry_point":"home","chunks_served":["a","b"],"client_had":[],"timestamp_ms":1000}"#;
const EVENT_B: &str = r#"{"session_id":"s2","entry_point":"home","chunks_served":["b","c"],"client_had":[],"timestamp_ms":2000}"#;
const EVENT_C: &str = r#"{"session_id":"s3","entry_point":"about","chunks_served":["d"],"client_had":[],"timestamp_ms":3000}"#;
const MALFORMED: &str = r#"not valid json {{{ "#;

#[test]
fn ingest_three_valid_one_malformed() {
    let store = PgoStore::open_in_memory().unwrap();
    let log = write_jsonl(&[EVENT_A, EVENT_B, MALFORMED, EVENT_C]);

    let stats = ingest_log(&store, log.path()).unwrap();

    assert_eq!(stats.lines_read, 4, "should count all 4 lines");
    assert_eq!(stats.sessions_inserted, 3, "3 valid sessions inserted");
    assert_eq!(stats.sessions_skipped_duplicate, 0, "no duplicates");
    assert_eq!(stats.parse_errors, 1, "1 malformed line");
}

#[test]
fn ingest_idempotent_second_run_skips_all() {
    let store = PgoStore::open_in_memory().unwrap();
    let log = write_jsonl(&[EVENT_A, EVENT_B]);

    ingest_log(&store, log.path()).unwrap();
    let stats2 = ingest_log(&store, log.path()).unwrap();

    assert_eq!(stats2.sessions_skipped_duplicate, 2, "both sessions duplicate on re-ingest");
    assert_eq!(stats2.sessions_inserted, 0, "no new insertions");
}

#[test]
fn ingest_populates_store() {
    let store = PgoStore::open_in_memory().unwrap();
    let log = write_jsonl(&[EVENT_A, EVENT_B]);

    ingest_log(&store, log.path()).unwrap();

    assert_eq!(store.session_count().unwrap(), 2);
    assert_eq!(store.chunk_load_count("b").unwrap(), 2);
}

#[test]
fn ingest_empty_file_returns_zero_stats() {
    let store = PgoStore::open_in_memory().unwrap();
    let log = write_jsonl(&[]);

    let stats = ingest_log(&store, log.path()).unwrap();

    assert_eq!(stats.lines_read, 0);
    assert_eq!(stats.sessions_inserted, 0);
    assert_eq!(stats.parse_errors, 0);
}
