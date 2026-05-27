//! Budget enforcement against a [`BuildStatsArtifact`].
//!
//! `[budget]` in `cloudpack.toml` is opt-in: an absent section is `None`, which
//! means **no checks run** and the build behaves exactly as before. Each
//! individual limit field is also `Option<u64>` — `None` means "this rule is
//! disabled".

use serde::Deserialize;

use crate::build_stats::{BudgetCheck, BudgetStatus, BuildStatsArtifact, ChunkRole};

// ---------------------------------------------------------------------------
// Config — populated from `[budget]` in `cloudpack.toml`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BudgetConfig {
    /// Max bytes for the initial bundle (sum of Entry + Commons chunks).
    pub initial_bundle_max_bytes: Option<u64>,
    /// Max bytes for any single Lazy chunk.
    pub lazy_chunk_max_bytes: Option<u64>,
    /// Max bytes for the total bundle (every emitted chunk).
    pub total_bundle_max_bytes: Option<u64>,
}

impl BudgetConfig {
    /// Returns `true` if at least one limit is set.
    pub fn any_limit_set(&self) -> bool {
        self.initial_bundle_max_bytes.is_some()
            || self.lazy_chunk_max_bytes.is_some()
            || self.total_bundle_max_bytes.is_some()
    }
}

// ---------------------------------------------------------------------------
// Violation type
// ---------------------------------------------------------------------------

/// Wrapper around the list of failed [`BudgetCheck`]s.
#[derive(Debug, Clone)]
pub struct BudgetViolation(pub Vec<BudgetCheck>);

impl BudgetViolation {
    /// Produce a multiline, human-readable message suitable for `eprintln!`.
    pub fn actionable_message(&self) -> String {
        let mut out = String::from("Budget violated:\n");
        for c in &self.0 {
            let actual = format_thousands(c.actual);
            let limit = format_thousands(c.limit);
            match &c.offender {
                Some(id) => out.push_str(&format!(
                    "  {}: {} bytes on chunk '{}' (limit {})\n",
                    c.name, actual, id, limit
                )),
                None => out.push_str(&format!(
                    "  {}: {} bytes (limit {})\n",
                    c.name, actual, limit
                )),
            }
        }
        out
    }
}

fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

// ---------------------------------------------------------------------------
// check()
// ---------------------------------------------------------------------------

/// Evaluate every configured limit against `stats`.
///
/// Returns `Ok(())` if every check is `Ok`. Returns
/// `Err(BudgetViolation(failed_checks))` if any check fails.
pub fn check(stats: &BuildStatsArtifact, budget: &BudgetConfig) -> Result<(), BudgetViolation> {
    let mut failures: Vec<BudgetCheck> = Vec::new();

    // 1. Total bundle size
    if let Some(limit) = budget.total_bundle_max_bytes {
        let actual = stats.summary.total_bundle_bytes;
        if actual > limit {
            failures.push(BudgetCheck {
                name: "total_bundle_max_bytes".to_string(),
                limit,
                actual,
                status: BudgetStatus::Violated,
                offender: None,
            });
        }
    }

    // 2. Initial bundle size
    if let Some(limit) = budget.initial_bundle_max_bytes {
        let actual = stats.summary.initial_bundle_bytes;
        if actual > limit {
            failures.push(BudgetCheck {
                name: "initial_bundle_max_bytes".to_string(),
                limit,
                actual,
                status: BudgetStatus::Violated,
                offender: None,
            });
        }
    }

    // 3. Per-lazy-chunk size
    if let Some(limit) = budget.lazy_chunk_max_bytes {
        for c in &stats.chunks {
            if matches!(c.role, ChunkRole::Lazy) && c.size_bytes > limit {
                failures.push(BudgetCheck {
                    name: "lazy_chunk_max_bytes".to_string(),
                    limit,
                    actual: c.size_bytes,
                    status: BudgetStatus::Violated,
                    offender: Some(c.id.clone()),
                });
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(BudgetViolation(failures))
    }
}

/// Build the [`BudgetResult`] block to embed in `BuildStatsArtifact.budget`.
pub fn build_budget_result(
    stats: &BuildStatsArtifact,
    budget: &BudgetConfig,
) -> crate::build_stats::BudgetResult {
    use crate::build_stats::BudgetResult;

    let mut checks: Vec<BudgetCheck> = Vec::new();
    let mut any_violation = false;

    if let Some(limit) = budget.total_bundle_max_bytes {
        let actual = stats.summary.total_bundle_bytes;
        let status = if actual > limit {
            any_violation = true;
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        };
        checks.push(BudgetCheck {
            name: "total_bundle_max_bytes".to_string(),
            limit,
            actual,
            status,
            offender: None,
        });
    }

    if let Some(limit) = budget.initial_bundle_max_bytes {
        let actual = stats.summary.initial_bundle_bytes;
        let status = if actual > limit {
            any_violation = true;
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        };
        checks.push(BudgetCheck {
            name: "initial_bundle_max_bytes".to_string(),
            limit,
            actual,
            status,
            offender: None,
        });
    }

    if let Some(limit) = budget.lazy_chunk_max_bytes {
        let mut worst: u64 = 0;
        let mut had_offender = false;
        for c in &stats.chunks {
            if matches!(c.role, ChunkRole::Lazy) {
                worst = worst.max(c.size_bytes);
                if c.size_bytes > limit {
                    had_offender = true;
                    any_violation = true;
                    checks.push(BudgetCheck {
                        name: "lazy_chunk_max_bytes".to_string(),
                        limit,
                        actual: c.size_bytes,
                        status: BudgetStatus::Violated,
                        offender: Some(c.id.clone()),
                    });
                }
            }
        }
        if !had_offender {
            checks.push(BudgetCheck {
                name: "lazy_chunk_max_bytes".to_string(),
                limit,
                actual: worst,
                status: BudgetStatus::Ok,
                offender: None,
            });
        }
    }

    BudgetResult {
        configured: true,
        checks,
        result: if any_violation {
            BudgetStatus::Violated
        } else {
            BudgetStatus::Ok
        },
    }
}
