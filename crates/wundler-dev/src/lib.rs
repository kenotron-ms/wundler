//! Dev-mode helpers: dependency pre-bundling cache, etc.

pub mod prebundle;

pub use prebundle::{compute_fingerprint, DepPrebundler, PrebundleResult};
