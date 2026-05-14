use wundler_pgo::store::PgoStore;
use wundler_pgo::types::SessionRecord;

/// Helper to build a `SessionRecord` concisely.
fn rec(session: &str, entry: &str, chunks: &[&str]) -> SessionRecord {
    SessionRecord {
        session_id: session.to_string(),
        entry_point: entry.to_string(),
        chunk_sequence: chunks.iter().map(|s| s.to_string()).collect(),
        timestamp_ms: 0,
    }
}

#[test]
fn inserts_one_row_per_chunk_in_sequence() {
    let store = PgoStore::open_in_memory().unwrap();
    let rows = store.insert_session(&rec("s1", "home", &["a", "b", "c"])).unwrap();
    assert_eq!(rows, 3, "should insert one row per chunk");
    assert_eq!(store.session_count().unwrap(), 1, "one distinct session");
}

#[test]
fn distinct_sessions_are_counted_independently() {
    let store = PgoStore::open_in_memory().unwrap();
    store.insert_session(&rec("s1", "home", &["a", "b"])).unwrap();
    store.insert_session(&rec("s2", "home", &["b", "c"])).unwrap();
    store.insert_session(&rec("s3", "about", &["d"])).unwrap();
    assert_eq!(store.session_count().unwrap(), 3, "three distinct sessions");
}

#[test]
fn chunk_load_count_reflects_inserts() {
    let store = PgoStore::open_in_memory().unwrap();
    // session 1 loads a, b
    store.insert_session(&rec("s1", "home", &["a", "b"])).unwrap();
    // session 2 loads a, c
    store.insert_session(&rec("s2", "home", &["a", "c"])).unwrap();

    assert_eq!(store.chunk_load_count("a").unwrap(), 2, "chunk 'a' loaded by 2 sessions");
    assert_eq!(store.chunk_load_count("b").unwrap(), 1, "chunk 'b' loaded by 1 session");
    assert_eq!(store.chunk_load_count("c").unwrap(), 1, "chunk 'c' loaded by 1 session");
    assert_eq!(store.chunk_load_count("nope").unwrap(), 0, "unknown chunk returns 0");
}

#[test]
fn empty_sequence_inserts_no_rows() {
    let store = PgoStore::open_in_memory().unwrap();
    let rows = store.insert_session(&rec("s1", "home", &[])).unwrap();
    assert_eq!(rows, 0, "empty sequence inserts 0 rows");
    assert_eq!(store.session_count().unwrap(), 0, "no session with empty sequence");
}
