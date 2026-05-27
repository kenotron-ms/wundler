use cloudpack_pgo::store::PgoStore;
use cloudpack_pgo::types::SessionRecord;

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

#[test]
fn duplicate_session_chunk_pair_is_ignored() {
    let store = PgoStore::open_in_memory().unwrap();
    let first = store
        .insert_session(&rec("s1", "home", &["a", "b", "c"]))
        .unwrap();
    assert_eq!(first, 3, "first insert should add 3 rows");

    let second = store
        .insert_session(&rec("s1", "home", &["a", "b", "c"]))
        .unwrap();
    assert_eq!(second, 0, "re-inserting identical session should return 0");

    assert_eq!(store.session_count().unwrap(), 1, "still only one distinct session");
    assert_eq!(store.chunk_load_count("a").unwrap(), 1, "chunk 'a' counted once");
}

#[test]
fn partial_overlap_only_adds_new_rows() {
    let store = PgoStore::open_in_memory().unwrap();
    let first = store
        .insert_session(&rec("s1", "home", &["a", "b"]))
        .unwrap();
    assert_eq!(first, 2, "first insert should add 2 rows");

    let second = store
        .insert_session(&rec("s1", "home", &["a", "b", "c"]))
        .unwrap();
    assert_eq!(second, 1, "second insert should add only the new chunk 'c'");

    assert_eq!(store.chunk_load_count("a").unwrap(), 1, "chunk 'a' counted once");
    assert_eq!(store.chunk_load_count("c").unwrap(), 1, "chunk 'c' counted once");
}

#[test]
fn idempotent_across_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pgo_test.db");

    // First open: insert session
    {
        let store = PgoStore::open(&db_path).unwrap();
        let rows = store
            .insert_session(&rec("s1", "home", &["a", "b", "c"]))
            .unwrap();
        assert_eq!(rows, 3, "initial insert should add 3 rows");
    } // store dropped here, connection closed

    // Second open: insert same session again
    {
        let store = PgoStore::open(&db_path).unwrap();
        let rows = store
            .insert_session(&rec("s1", "home", &["a", "b", "c"]))
            .unwrap();
        assert_eq!(rows, 0, "re-insert after reopen should return 0");
        assert_eq!(store.session_count().unwrap(), 1, "still only one distinct session");
    }
}
