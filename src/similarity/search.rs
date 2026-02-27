use anyhow::Result;
use sqlx::PgPool;

use super::math::{SharedCapability, calculate_shared_capabilities};
use super::vector_builder::{load_all_vectors, load_vector, CapabilityVector};
use crate::signals::capability_registry::CapabilityRegistry;

/// Result from a similarity search
#[derive(Debug)]
pub struct SimilarityResult {
    pub entity_id: String,
    pub similarity_score: f32,
    pub shared_capabilities: Vec<SharedCapability>,
}

/// Find similar entities based on hybrid capability vectors (18-dim + 5-dim meta).
pub async fn find_similar_entities(
    pool: &PgPool,
    target_entity: &str,
    limit: usize,
    registry: &CapabilityRegistry,
) -> Result<Vec<SimilarityResult>> {
    // 1. Load target vector
    let target_vector = load_vector(pool, target_entity, registry).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "Entity '{}' not found in capability_vectors. Run --profile first.",
            target_entity
        )
    })?;

    // 2. Load all other vectors
    let all_vectors = load_all_vectors(pool, registry).await?;

    // 3. Build display names array (ordered by registry) for shared-capabilities reporting
    let display_names: Vec<String> = registry
        .capabilities
        .iter()
        .map(|c| c.display_name.clone())
        .collect();
    let display_name_refs: Vec<&str> = display_names.iter().map(|s| s.as_str()).collect();

    // 4. Compute hybrid similarity for all other entities
    let mut results: Vec<SimilarityResult> = all_vectors
        .into_iter()
        .filter(|v| v.entity_id != target_entity)
        .map(|other| {
            // Hybrid: 0.7 × cosine(18-dim) + 0.3 × cosine(meta-5-dim)
            let score = target_vector.hybrid_similarity(&other);

            // Report shared strengths using capability display names
            let overlaps = calculate_shared_capabilities(
                &target_vector.scores,
                &other.scores,
                &display_name_refs,
            );

            SimilarityResult {
                entity_id: other.entity_id,
                similarity_score: score,
                shared_capabilities: overlaps,
            }
        })
        .collect();

    // 5. Sort descending by similarity
    results.sort_by(|a, b| {
        b.similarity_score
            .partial_cmp(&a.similarity_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 6. Truncate to top K
    results.truncate(limit);

    Ok(results)
}
