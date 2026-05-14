//! # wundler-abs
//!
//! The Asset Bundling Server (ABS) is a lightweight HTTP service that serves
//! **manifests**, never code. Given an entry-point and the set of content
//! hashes the client already holds, ABS replies with a list of fetch URLs
//! and optional prefetch hints — telling the browser exactly which chunks to
//! request and which to warm the cache with, without ever shipping source
//! bytes itself.

pub mod types;
