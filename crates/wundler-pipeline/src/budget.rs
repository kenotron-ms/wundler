//! Budget enforcement — full implementation in Task 5.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BudgetConfig {
    pub initial_bundle_max_bytes: Option<u64>,
    pub lazy_chunk_max_bytes: Option<u64>,
    pub total_bundle_max_bytes: Option<u64>,
}
