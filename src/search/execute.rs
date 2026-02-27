use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::schema::*;

use super::ranking::calculate_final_score;
use super::results::{calculate_keyword_match, determine_matched_fields};
use super::{CapabilityIndex, CapabilityQuery, SearchResult};

/// Execute a capability search query
pub fn search_capabilities(
    index: &CapabilityIndex,
    query: &CapabilityQuery,
    config: &crate::config::SearchConfig,
) -> Result<Vec<SearchResult>> {
    let reader = index.index.reader()?;
    let searcher = reader.searcher();

    // Build Tantivy query
    let tantivy_query = super::build_query(query, &index.schema)?;

    // Execute search (retrieve more for re-ranking)
    let top_docs = searcher.search(&tantivy_query, &TopDocs::with_limit(query.limit * 3))?;

    // Get ranking weights
    let weights = config.get_ranking_weights();
    let use_calibration = config.ranking.calibration.enabled;

    // Extract and re-rank results
    let mut results = Vec::new();

    for (_score, doc_address) in top_docs {
        let doc = searcher.doc(doc_address)?;

        // Extract fields
        let entity_id = get_text_field(&doc, "entity_id", &index.schema)?;
        let capability_type = get_text_field(&doc, "capability_type", &index.schema)?;
        let tier = get_text_field(&doc, "tier", &index.schema)?;
        let confidence = get_f64_field(&doc, "confidence", &index.schema)? as f32;
        let normalized_score =
            get_f64_field_optional(&doc, "normalized_score", &index.schema)?.unwrap_or(0.0) as f32;
        let timestamp = get_i64_field(&doc, "timestamp", &index.schema)?;
        let keywords_str = get_text_field(&doc, "evidence_keywords", &index.schema)?;
        let repos_str = get_text_field(&doc, "evidence_repos", &index.schema)?;

        let evidence_keywords: Vec<String> = keywords_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let evidence_repos: Vec<String> = repos_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        // Calculate keyword match score
        let keyword_match_score = if let Some(ref kw) = query.keywords {
            calculate_keyword_match(&evidence_keywords, kw)
        } else {
            0.0
        };

        // Decide which base score to use
        let base_score = if use_calibration {
            normalized_score
        } else {
            confidence
        };

        // Calculate final ranking score
        let final_score =
            calculate_final_score(base_score, timestamp, keyword_match_score, weights);

        // Determine what matched
        let matched_on = determine_matched_fields(query, &evidence_keywords, &evidence_repos);

        results.push(SearchResult {
            entity_id,
            capability_type,
            confidence,
            normalized_score,
            tier,
            timestamp,
            evidence_keywords,
            evidence_repos,
            final_score,
            matched_on,
        });
    }

    // Sort by final score (descending)
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit to requested size
    results.truncate(query.limit);

    Ok(results)
}

/// Helper to extract optional f64 field from document
fn get_f64_field_optional(
    doc: &tantivy::TantivyDocument,
    field_name: &str,
    schema: &Schema,
) -> Result<Option<f64>> {
    let field = schema.get_field(field_name)?;
    Ok(doc.get_first(field).and_then(|v| v.as_f64()))
}

/// Helper to extract text field from document
fn get_text_field(
    doc: &tantivy::TantivyDocument,
    field_name: &str,
    schema: &Schema,
) -> Result<String> {
    let field = schema.get_field(field_name)?;
    let value = doc
        .get_first(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Field {} not found or not text", field_name))?;
    Ok(value.to_string())
}

/// Helper to extract f64 field from document
fn get_f64_field(doc: &tantivy::TantivyDocument, field_name: &str, schema: &Schema) -> Result<f64> {
    let field = schema.get_field(field_name)?;
    let value = doc
        .get_first(field)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| anyhow::anyhow!("Field {} not found or not f64", field_name))?;
    Ok(value)
}

/// Helper to extract i64 field from document
fn get_i64_field(doc: &tantivy::TantivyDocument, field_name: &str, schema: &Schema) -> Result<i64> {
    let field = schema.get_field(field_name)?;
    let value = doc
        .get_first(field)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Field {} not found or not i64", field_name))?;
    Ok(value)
}
