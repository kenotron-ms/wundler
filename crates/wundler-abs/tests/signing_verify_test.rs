//! Integration tests for `AppState::load_signed_from_disk` — verifies that
//! the ABS refuses to load a manifest when the signature does not match.
//!
//! Tests:
//! 1. `load_with_valid_signature_succeeds`  — matching signer/verifier → Ok
//! 2. `load_with_wrong_signature_fails`     — verifier uses a DIFFERENT keypair → Err
//! 3. `load_with_tampered_manifest_fails`   — file contents tampered after signing → Err
//! 4. `load_without_verifier_skips_check`   — verify=None → always Ok (no sig needed)

use std::io::Write as _;

use ed25519_dalek::Signature;
use tempfile::NamedTempFile;
use wundler_abs::signing::{generate_keypair, ManifestSigner, ManifestVerifier};
use wundler_abs::state::AppState;
use wundler_graph::ChunkManifest;

// ---------------------------------------------------------------------------
// BASE_JSON — minimal valid manifest: build_id='b1', one chunk 'a'
// ---------------------------------------------------------------------------

const BASE_JSON: &str = r#"{
    "build_id": "b1",
    "chunks": [
        {
            "id": "a",
            "modules": ["mod1"],
            "hash": "hash1",
            "load_condition": "Lazy",
            "co_request_score": null,
            "median_load_order": null,
            "suggested_merge": null
        }
    ],
    "entry_chunks": {
        "home": ["a"]
    },
    "module_index": {
        "mod1": "a"
    }
}"#;

// ---------------------------------------------------------------------------
// Helper: write bytes to a named temp file and return it (kept alive by caller)
// ---------------------------------------------------------------------------

fn write_to_temp(bytes: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create temp file");
    f.write_all(bytes).expect("write temp file");
    f
}

// ---------------------------------------------------------------------------
// Test 1: valid signature → Ok
// ---------------------------------------------------------------------------

/// A signer produces a signature over the manifest; the matching verifier
/// must accept it, and the returned `AppState` must expose the correct
/// `build_id`.
#[tokio::test]
async fn load_with_valid_signature_succeeds() {
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem)
        .expect("ManifestSigner::from_pem with freshly generated key");

    // Parse BASE_JSON so we can sign the canonical bytes.
    let manifest: ChunkManifest =
        serde_json::from_str(BASE_JSON).expect("BASE_JSON must be valid ChunkManifest JSON");
    let sig: Signature = signer.sign_manifest(&manifest);

    // Write the same JSON to disk.
    let tmp = write_to_temp(BASE_JSON.as_bytes());

    let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem)
        .expect("ManifestVerifier::from_pem with matching public key");

    let state = AppState::load_signed_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        Some((&verifier, &sig)),
    )
    .await
    .expect("load_signed_from_disk must succeed when signature matches");

    let guard = state.manifest.read().await;
    assert_eq!(
        guard.build_id, "b1",
        "loaded manifest must have build_id='b1'"
    );
}

// ---------------------------------------------------------------------------
// Test 2: wrong keypair → Err
// ---------------------------------------------------------------------------

/// If the verifier uses a DIFFERENT key than the signer, verification must
/// fail and `load_signed_from_disk` must return an error.
#[tokio::test]
async fn load_with_wrong_signature_fails() {
    // Signing key: keypair A
    let kp_a = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp_a.signing_key_pem)
        .expect("ManifestSigner::from_pem for keypair A");

    let manifest: ChunkManifest =
        serde_json::from_str(BASE_JSON).expect("BASE_JSON must be valid ChunkManifest JSON");
    let sig: Signature = signer.sign_manifest(&manifest);

    // Verifying key: keypair B (different!)
    let kp_b = generate_keypair();
    let verifier = ManifestVerifier::from_pem(&kp_b.verifying_key_pem)
        .expect("ManifestVerifier::from_pem for keypair B");

    let tmp = write_to_temp(BASE_JSON.as_bytes());

    let result = AppState::load_signed_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        Some((&verifier, &sig)),
    )
    .await;

    assert!(
        result.is_err(),
        "load_signed_from_disk must fail when verifier key does not match signer key"
    );
}

// ---------------------------------------------------------------------------
// Test 3: tampered manifest on disk → Err
// ---------------------------------------------------------------------------

/// Sign the original manifest; then write a TAMPERED version (build_id
/// replaced with 'b2-EVIL') to disk.  Verification must reject the tampered
/// file even though the signature itself is structurally valid.
#[tokio::test]
async fn load_with_tampered_manifest_fails() {
    let kp = generate_keypair();
    let signer =
        ManifestSigner::from_pem(&kp.signing_key_pem).expect("ManifestSigner::from_pem");

    // Sign the ORIGINAL manifest.
    let original: ChunkManifest =
        serde_json::from_str(BASE_JSON).expect("BASE_JSON must be valid ChunkManifest JSON");
    let sig: Signature = signer.sign_manifest(&original);

    // Write a TAMPERED version to disk — build_id changed to 'b2-EVIL'.
    let tampered = BASE_JSON.replace("\"b1\"", "\"b2-EVIL\"");
    let tmp = write_to_temp(tampered.as_bytes());

    let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem)
        .expect("ManifestVerifier::from_pem");

    let result = AppState::load_signed_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        Some((&verifier, &sig)),
    )
    .await;

    assert!(
        result.is_err(),
        "load_signed_from_disk must fail when the on-disk manifest has been tampered with"
    );
}

// ---------------------------------------------------------------------------
// Test 4: no verifier → always succeeds (signature not required)
// ---------------------------------------------------------------------------

/// Passing `None` for the verifier must skip signature checking entirely and
/// return `Ok` for any well-formed manifest.
#[tokio::test]
async fn load_without_verifier_skips_check() {
    let tmp = write_to_temp(BASE_JSON.as_bytes());

    let state = AppState::load_signed_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        None, // no verification requested
    )
    .await
    .expect("load_signed_from_disk with verify=None must always succeed for valid JSON");

    let guard = state.manifest.read().await;
    assert_eq!(
        guard.build_id, "b1",
        "manifest loaded without verification must still have the correct build_id"
    );
}
