use tempfile::NamedTempFile;
use cloudpack_pgo::store::PgoStore;

#[test]
fn opens_in_memory_store() {
    let store = PgoStore::open_in_memory().expect("should open in-memory store");
    let count = store.session_count().expect("should count sessions");
    assert_eq!(count, 0, "fresh store should have 0 sessions");
}

#[test]
fn creates_sessions_table_and_indexes() {
    let store = PgoStore::open_in_memory().expect("should open in-memory store");

    let tables = store.list_tables().expect("should list tables");
    assert!(
        tables.contains(&"sessions".to_string()),
        "tables should contain 'sessions', got: {:?}",
        tables
    );

    let indexes = store.list_indexes().expect("should list indexes");
    assert!(
        indexes.contains(&"idx_chunk".to_string()),
        "indexes should contain 'idx_chunk', got: {:?}",
        indexes
    );
    assert!(
        indexes.contains(&"idx_entry".to_string()),
        "indexes should contain 'idx_entry', got: {:?}",
        indexes
    );
}

#[test]
fn opens_file_backed_store() {
    let tmp = NamedTempFile::new().expect("should create temp file");
    let path = tmp.path().to_path_buf();
    // NamedTempFile keeps the file open; we want to use path but let PgoStore create the DB
    // Drop the NamedTempFile handle so PgoStore can write to the path
    drop(tmp);

    let store = PgoStore::open(&path).expect("should open file-backed store");
    assert!(path.exists(), "database file should exist at {:?}", path);

    let count = store.session_count().expect("should count sessions");
    assert_eq!(count, 0, "fresh file-backed store should have 0 sessions");
}

#[test]
fn double_open_is_idempotent() {
    let tmp = NamedTempFile::new().expect("should create temp file");
    let path = tmp.path().to_path_buf();
    drop(tmp);

    // First open — creates schema
    {
        let store = PgoStore::open(&path).expect("first open should succeed");
        let count = store.session_count().expect("should count sessions");
        assert_eq!(count, 0);
    } // store dropped here, connection closed

    // Second open — schema already exists, CREATE IF NOT EXISTS should be idempotent
    {
        let store = PgoStore::open(&path).expect("second open should succeed");
        let count = store.session_count().expect("should count sessions after reopen");
        assert_eq!(count, 0, "reopened store should still have 0 sessions");
    }
}
