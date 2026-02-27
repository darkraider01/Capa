use serde::{Deserialize, Serialize};

/// Search result with ranking and matched fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub entity_id: String,
    pub capability_type: String,
    pub confidence: f32,
    pub normalized_score: f32,
    pub tier: String,
    pub timestamp: i64,
    pub evidence_keywords: Vec<String>,
    pub evidence_repos: Vec<String>,
    pub final_score: f32,
    pub matched_on: MatchedFields,
}

/// Fields that matched the search query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedFields {
    pub keywords: Vec<String>,
    pub repos: Vec<String>,
}

impl MatchedFields {
    pub fn empty() -> Self {
        Self {
            keywords: Vec::new(),
            repos: Vec::new(),
        }
    }
}

use crate::search::CapabilityQuery;

/// Determine which fields matched the query
pub fn determine_matched_fields(
    query: &CapabilityQuery,
    evidence_keywords: &[String],
    evidence_repos: &[String],
) -> MatchedFields {
    let mut matched_keywords = Vec::new();
    let mut matched_repos = Vec::new();

    if let Some(ref kw) = query.keywords {
        let search_terms: Vec<&str> = kw.split_whitespace().collect();

        // Find matching keywords
        for keyword in evidence_keywords {
            for term in &search_terms {
                if keyword.to_lowercase().contains(&term.to_lowercase()) {
                    matched_keywords.push(keyword.clone());
                    break;
                }
            }
        }

        // Find matching repos
        for repo in evidence_repos {
            for term in &search_terms {
                if repo.to_lowercase().contains(&term.to_lowercase()) {
                    matched_repos.push(repo.clone());
                    break;
                }
            }
        }
    }

    MatchedFields {
        keywords: matched_keywords,
        repos: matched_repos,
    }
}

/// Calculate keyword match score (0-1)
pub fn calculate_keyword_match(evidence_keywords: &[String], query_keywords: &str) -> f32 {
    if evidence_keywords.is_empty() {
        return 0.0;
    }

    let search_terms: Vec<&str> = query_keywords.split_whitespace().collect();
    if search_terms.is_empty() {
        return 0.0;
    }

    let mut matches = 0;
    for keyword in evidence_keywords {
        for term in &search_terms {
            if keyword.to_lowercase().contains(&term.to_lowercase()) {
                matches += 1;
                break;
            }
        }
    }

    (matches as f32 / search_terms.len() as f32).min(1.0)
}
