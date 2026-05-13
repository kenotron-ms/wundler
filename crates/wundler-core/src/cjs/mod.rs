// CJS interop module — CommonJS module analysis utilities.
pub mod stub;
pub use stub::{detect_cjs_exports, generate_cjs_stub, is_cjs};
