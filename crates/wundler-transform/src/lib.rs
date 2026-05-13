//! Wundler Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub fn hello() -> &'static str {
    "wundler-transform"
}
