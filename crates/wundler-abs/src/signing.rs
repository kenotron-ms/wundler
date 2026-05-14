//! Manifest signing infrastructure for the Asset Bundling Server.
//!
//! Provides ed25519 key-pair generation, PKCS8-PEM encoding, and
//! deterministic manifest signing/verification via [`ManifestSigner`] and
//! [`ManifestVerifier`].

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use pkcs8::LineEnding;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use wundler_graph::ChunkManifest;

// ---------------------------------------------------------------------------
// KeyPairPem
// ---------------------------------------------------------------------------

/// A freshly generated ed25519 key pair encoded as PKCS8 PEM strings.
pub struct KeyPairPem {
    /// Private signing key in PKCS8 PEM format (LF line endings).
    pub signing_key_pem: String,
    /// Public verifying key in PKCS8 SubjectPublicKeyInfo PEM format (LF line endings).
    pub verifying_key_pem: String,
}

// ---------------------------------------------------------------------------
// generate_keypair
// ---------------------------------------------------------------------------

/// Generate a new ed25519 key pair and return both halves as PEM strings.
///
/// Uses [`OsRng`] as the cryptographic randomness source.  Each call produces
/// a fresh, independently random key pair.
pub fn generate_keypair() -> KeyPairPem {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let signing_key_pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("ed25519 signing key must encode to PKCS8 PEM without error")
        .to_string();

    let verifying_key_pem = verifying_key
        .to_public_key_pem(LineEnding::LF)
        .expect("ed25519 verifying key must encode to SubjectPublicKeyInfo PEM without error");

    KeyPairPem {
        signing_key_pem,
        verifying_key_pem,
    }
}

// ---------------------------------------------------------------------------
// ManifestSigner
// ---------------------------------------------------------------------------

/// Signs [`ChunkManifest`] values using a private ed25519 signing key.
pub struct ManifestSigner {
    signing_key: SigningKey,
}

impl ManifestSigner {
    /// Parse a PKCS8 PEM-encoded ed25519 private key and construct a signer.
    pub fn from_pem(pem: &str) -> Result<Self> {
        let signing_key =
            SigningKey::from_pkcs8_pem(pem).context("failed to parse ed25519 signing key PEM")?;
        Ok(Self { signing_key })
    }

    /// Produce an ed25519 signature over the canonical byte representation of
    /// `manifest`.
    pub fn sign_manifest(&self, manifest: &ChunkManifest) -> Signature {
        use ed25519_dalek::Signer as _;
        let bytes = manifest_signature_bytes(manifest);
        self.signing_key.sign(&bytes)
    }
}

// ---------------------------------------------------------------------------
// ManifestVerifier
// ---------------------------------------------------------------------------

/// Verifies [`ChunkManifest`] signatures using a public ed25519 verifying key.
pub struct ManifestVerifier {
    verifying_key: VerifyingKey,
}

impl ManifestVerifier {
    /// Parse a SubjectPublicKeyInfo PEM-encoded ed25519 public key and
    /// construct a verifier.
    pub fn from_pem(pem: &str) -> Result<Self> {
        let verifying_key = VerifyingKey::from_public_key_pem(pem)
            .context("failed to parse ed25519 verifying key PEM")?;
        Ok(Self { verifying_key })
    }

    /// Verify `sig` against `manifest`.
    ///
    /// Returns `Ok(())` if valid, or an `Err` describing the failure.
    pub fn verify(&self, manifest: &ChunkManifest, sig: &Signature) -> Result<()> {
        use ed25519_dalek::Verifier as _;
        let bytes = manifest_signature_bytes(manifest);
        self.verifying_key
            .verify(&bytes, sig)
            .map_err(|e| anyhow!("manifest signature verification failed: {e}"))
    }
}

// ---------------------------------------------------------------------------
// manifest_signature_bytes
// ---------------------------------------------------------------------------

/// Build the canonical, deterministic byte representation of a manifest used
/// for signing and verification.
///
/// Format:
/// ```text
/// {build_id}\n
/// {chunk_id} {hash_hex}\n   (chunks sorted by id ascending)
/// ```
///
/// Advisory PGO fields (`co_request_score`, `median_load_order`,
/// `suggested_merge`) are deliberately excluded: they may be updated by the
/// optimisation pipeline without invalidating chunk integrity.
pub fn manifest_signature_bytes(manifest: &ChunkManifest) -> Vec<u8> {
    let mut buf = String::new();

    // Write build_id line.
    buf.push_str(&manifest.build_id);
    buf.push('\n');

    // Collect and sort chunks by id for deterministic output.
    let mut chunks: Vec<_> = manifest.chunks.iter().collect();
    chunks.sort_by(|a, b| a.id.cmp(&b.id));

    for chunk in chunks {
        buf.push_str(&chunk.id);
        buf.push(' ');
        buf.push_str(chunk.hash.as_str());
        buf.push('\n');
    }

    buf.into_bytes()
}
