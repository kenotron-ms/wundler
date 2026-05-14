//! Integration tests for DevServer — Axum static + on-demand SWC TS→JS transform.

use tempfile::TempDir;
use wundler_pipeline::dev_server::DevServer;

/// Helper: create a temp directory, write a file into it, return the dir.
fn make_root(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, contents) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
    }
    dir
}

#[tokio::test]
async fn serves_typescript_as_transformed_javascript() {
    let root = make_root(&[("src/index.ts", "export const x: number = 1;\n")]);

    let server = DevServer {
        root: root.path().to_path_buf(),
        port: 0,
    };
    let (addr, _tx) = server.start_for_test().await.expect("start_for_test");

    let url = format!("http://{}/src/index.ts", addr);
    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await.expect("GET /src/index.ts");

    assert_eq!(resp.status(), 200);

    let body = resp.text().await.expect("body");
    assert!(
        body.contains("export const x"),
        "body should contain `export const x`; got: {body}"
    );
    assert!(
        !body.contains(": number"),
        "body should NOT contain `: number` (type annotation should be stripped); got: {body}"
    );
}

#[tokio::test]
async fn serves_plain_javascript_unmodified() {
    let root = make_root(&[("plain.js", "export const y = 42;\n")]);

    let server = DevServer {
        root: root.path().to_path_buf(),
        port: 0,
    };
    let (addr, _tx) = server.start_for_test().await.expect("start_for_test");

    let url = format!("http://{}/plain.js", addr);
    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await.expect("GET /plain.js");

    assert_eq!(resp.status(), 200);

    let body = resp.text().await.expect("body");
    assert!(
        body.contains("export const y = 42"),
        "body should contain original JS; got: {body}"
    );
}

#[tokio::test]
async fn returns_404_for_missing_file() {
    let root = make_root(&[]);

    let server = DevServer {
        root: root.path().to_path_buf(),
        port: 0,
    };
    let (addr, _tx) = server.start_for_test().await.expect("start_for_test");

    let url = format!("http://{}/nonexistent.ts", addr);
    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await.expect("GET /nonexistent.ts");

    assert_eq!(resp.status(), 404);
}
