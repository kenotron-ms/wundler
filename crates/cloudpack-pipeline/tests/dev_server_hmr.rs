//! Integration tests for DevServer HMR — SSE-based hot module replacement.

use std::time::Duration;

use futures_util::StreamExt;
use tempfile::TempDir;
use cloudpack_pipeline::dev_server::DevServer;

/// Helper: create a temp directory, write files into it, return the dir.
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
async fn hmr_client_script_is_served() {
    let root = make_root(&[]);
    let server = DevServer {
        root: root.path().to_path_buf(),
        port: 0,
    };
    let (addr, _tx) = server.start_for_test().await.expect("start_for_test");

    let url = format!("http://{}/__cloudpack__/hmr-client.js", addr);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .expect("GET /__cloudpack__/hmr-client.js");

    assert_eq!(resp.status(), 200);

    let body = resp.text().await.expect("body");
    assert!(
        body.contains("EventSource"),
        "body should contain 'EventSource'; got: {body}"
    );
    assert!(
        body.contains("__cloudpack__/hmr"),
        "body should contain '__cloudpack__/hmr'; got: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_endpoint_emits_event_on_file_change() {
    let root = make_root(&[("watched.ts", "export const x: number = 1;\n")]);
    let server = DevServer {
        root: root.path().to_path_buf(),
        port: 0,
    };
    let (addr, _tx) = server.start_for_test().await.expect("start_for_test");

    // Give the watcher time to initialize before we trigger any changes.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Open the SSE stream.
    let url = format!("http://{}/__cloudpack__/hmr", addr);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .expect("GET /__cloudpack__/hmr");
    assert_eq!(response.status(), 200);

    // Spawn a task to modify the file after a short delay so that we are
    // already reading the stream when the event fires.
    let watched_path = root.path().join("watched.ts");
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        tokio::fs::write(&watched_path, "export const x: number = 2;\n")
            .await
            .expect("write watched.ts");
    });

    // Read chunks from the SSE stream until we find one that mentions
    // "watched.ts", or until the 5-second deadline elapses.
    let mut stream = response.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut found = false;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("watched.ts") {
                    found = true;
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        found,
        "SSE stream should have received an event containing 'watched.ts' within 5s"
    );
}
