use serde::{Deserialize, Serialize};

/// Structured query for capability search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityQuery {
    pub capability_type: Option<String>,
    pub min_confidence: Option<f32>,
    pub max_confidence: Option<f32>,
    pub tier: Option<String>,
    pub keywords: Option<String>,
    pub recent_within_days: Option<i64>,
    pub entity_id: Option<String>,
    pub limit: usize,
}

impl Default for CapabilityQuery {
    fn default() -> Self {
        Self {
            capability_type: None,
            min_confidence: None,
            max_confidence: None,
            tier: None,
            keywords: None,
            recent_within_days: None,
            entity_id: None,
            limit: 10,
        }
    }
}

use anyhow::Result;
use std::ops::Bound;
use tantivy::{Term, query::*, schema::*};

/// Build a Tantivy query from structured CapabilityQuery
pub fn build_query(query: &CapabilityQuery, schema: &Schema) -> Result<Box<dyn Query>> {
    let mut subqueries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    // 1. Entity ID filter
    if let Some(ref entity) = query.entity_id {
        let field = schema.get_field("entity_id")?;
        let term_query = TermQuery::new(
            Term::from_field_text(field, entity),
            IndexRecordOption::Basic,
        );
        subqueries.push((Occur::Must, Box::new(term_query)));
    }

    // 2. Capability type filter
    if let Some(ref cap_type) = query.capability_type {
        let field = schema.get_field("capability_type")?;
        let term_query = TermQuery::new(
            Term::from_field_text(field, cap_type),
            IndexRecordOption::Basic,
        );
        subqueries.push((Occur::Must, Box::new(term_query)));
    }

    // 3. Tier filter
    if let Some(ref tier) = query.tier {
        let field = schema.get_field("tier")?;
        let term_query =
            TermQuery::new(Term::from_field_text(field, tier), IndexRecordOption::Basic);
        subqueries.push((Occur::Must, Box::new(term_query)));
    }

    // 4. Confidence range
    if query.min_confidence.is_some() || query.max_confidence.is_some() {
        let min = query.min_confidence.unwrap_or(0.0) as f64;
        let max = query.max_confidence.unwrap_or(1.0) as f64;

        let range_query = RangeQuery::new_f64_bounds(
            "confidence".to_string(),
            Bound::Included(min),
            Bound::Included(max),
        );
        subqueries.push((Occur::Must, Box::new(range_query)));
    }

    // 5. Recency filter
    if let Some(days) = query.recent_within_days {
        let cutoff = chrono::Utc::now().timestamp() - (days * 24 * 3600);

        let range_query = RangeQuery::new_i64_bounds(
            "timestamp".to_string(),
            Bound::Included(cutoff),
            Bound::Unbounded,
        );
        subqueries.push((Occur::Must, Box::new(range_query)));
    }

    // 6. Keyword search (should match, not must)
    if let Some(ref keywords) = query.keywords {
        let field = schema.get_field("evidence_keywords")?;

        // Simple term matching for keywords
        let terms: Vec<&str> = keywords.split_whitespace().collect();
        let mut keyword_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for term in terms {
            let term_query = TermQuery::new(
                Term::from_field_text(field, term),
                IndexRecordOption::WithFreqs,
            );
            keyword_queries.push((Occur::Should, Box::new(term_query)));
        }

        if !keyword_queries.is_empty() {
            let bool_query = BooleanQuery::new(keyword_queries);
            subqueries.push((Occur::Should, Box::new(bool_query)));
        }
    }

    // If no subqueries, return all documents query
    if subqueries.is_empty() {
        Ok(Box::new(AllQuery))
    } else {
        Ok(Box::new(BooleanQuery::new(subqueries)))
    }
}
