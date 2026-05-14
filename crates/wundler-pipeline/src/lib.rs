//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
