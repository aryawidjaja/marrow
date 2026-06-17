//! Structured query filters with a token budget.

use marrow_memdocs::{MemoryKind, Status};

/// A structured query over the index. Unset fields don't filter.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub kind: Option<MemoryKind>,
    pub status: Option<Status>,
    pub topic: Option<String>,
    pub project_id: Option<String>,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub org_id: Option<String>,
    pub min_confidence: Option<f64>,
    pub tag: Option<String>,
    /// Exclude memories whose `expires_at` is in the past (default true).
    pub exclude_expired: bool,
    /// Hard cap on returned rows.
    pub limit: Option<usize>,
    /// Stop accumulating results once the estimated token total would exceed this.
    pub max_tokens: Option<usize>,
    /// Hybrid search weight: 0 = keyword only, 1 = semantic only. None uses the store default.
    pub hybrid_weight: Option<f64>,
}

impl Query {
    /// A query scoped to a project, excluding expired memories.
    pub fn for_project(project_id: impl Into<String>) -> Query {
        Query {
            project_id: Some(project_id.into()),
            exclude_expired: true,
            ..Query::default()
        }
    }
}

/// Rough token estimate for budgeting (≈ 4 chars per token).
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
