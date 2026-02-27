use crate::extraction::models::ExtractedCapability;
use crate::signals::capability_registry::CapabilityRegistry;
use anyhow::Result;
use serde_json;
use sqlx::PgPool;
use std::collections::HashMap;

/// Dynamic N-dimensional capability vector
#[derive(Debug, Clone)]
pub struct CapabilityVector {
    pub entity_id: String,
    /// Ordered by registry canonical order (len = registry.len())
    pub scores: Vec<f32>,
    /// 5-dim meta-capability scores: [Systems, Infrastructure, Data, Application, Research]
    pub meta_scores: Vec<f32>,
}

impl CapabilityVector {
    /// Build an N-dimensional vector from extracted capabilities using the registry's canonical order.
    pub fn from_capabilities(
        entity_id: &str,
        capabilities: &[ExtractedCapability],
        registry: &CapabilityRegistry,
    ) -> Self {
        let mut scores = vec![0.0f32; registry.len()];

        // To ensure crisp domain separation, we only build the vector
        // out of the developer's Top 5 definitively strongest signals.
        let mut sorted_caps: Vec<&ExtractedCapability> = capabilities.iter().collect();
        sorted_caps.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        let top_caps: Vec<&ExtractedCapability> = sorted_caps.into_iter().take(5).collect();

        for cap in &top_caps {
            // Sparsify weak signals to boost profile distinctness
            if cap.confidence > 0.1 {
                if let Some(idx) = registry.index_of(cap.capability_type.as_str()) {
                    scores[idx] = f32::max(scores[idx], cap.confidence);
                }
            }
        }

        // Build score map for meta-vector calculation using confidence
        let score_map: HashMap<String, f32> = top_caps
            .iter()
            .filter(|c| c.confidence > 0.1)
            .filter_map(|c| {
                registry
                    .index_of(c.capability_type.as_str())
                    .map(|_| (c.capability_type.0.clone(), c.confidence))
            })
            .collect();

        let meta_scores = registry.build_meta_vector(&score_map);

        Self {
            entity_id: entity_id.to_string(),
            scores,
            meta_scores,
        }
    }

    /// Compute hybrid cosine similarity with another vector.
    /// final_sim = 0.7 × cosine(18-dim) + 0.3 × cosine(meta-5-dim)
    pub fn hybrid_similarity(&self, other: &Self) -> f32 {
        let cap_sim = cosine_similarity(&self.scores, &other.scores);
        let meta_sim = cosine_similarity(&self.meta_scores, &other.meta_scores);
        0.7 * cap_sim + 0.3 * meta_sim
    }

    /// Build a JSONB scores map (capability_id → score) for DB storage.
    pub fn to_scores_json(&self, registry: &CapabilityRegistry) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (i, &score) in self.scores.iter().enumerate() {
            if score > 0.0 {
                let id = &registry.capabilities[i].id;
                map.insert(id.clone(), serde_json::json!(score));
            }
        }
        serde_json::Value::Object(map)
    }

    /// Build a JSONB meta_scores map for DB storage.
    pub fn to_meta_json(&self) -> serde_json::Value {
        let meta_names = ["Systems", "Infrastructure", "Data", "Application", "Research"];
        let mut map = serde_json::Map::new();
        for (&score, name) in self.meta_scores.iter().zip(meta_names.iter()) {
            map.insert(name.to_string(), serde_json::json!(score));
        }
        serde_json::Value::Object(map)
    }

    /// Load a vector from DB JSONB, ordered by registry.
    pub fn from_json(
        entity_id: &str,
        scores_json: &serde_json::Value,
        meta_json: &serde_json::Value,
        registry: &CapabilityRegistry,
    ) -> Self {
        let mut scores = vec![0.0f32; registry.len()];
        if let Some(obj) = scores_json.as_object() {
            for (id, val) in obj {
                if let (Some(idx), Some(score)) = (registry.index_of(id), val.as_f64()) {
                    scores[idx] = score as f32;
                }
            }
        }

        let meta_names = ["Systems", "Infrastructure", "Data", "Application", "Research"];
        let meta_scores = meta_names
            .iter()
            .map(|name| {
                meta_json
                    .get(name)
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32
            })
            .collect();

        Self {
            entity_id: entity_id.to_string(),
            scores,
            meta_scores,
        }
    }
}

/// Store a capability vector in the database (JSONB columns).
pub async fn store_vector(
    pool: &PgPool,
    vector: &CapabilityVector,
    registry: &CapabilityRegistry,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let scores_json = vector.to_scores_json(registry);
    let meta_json = vector.to_meta_json();

    sqlx::query(
        r#"
        INSERT INTO capability_vectors (entity_id, scores, meta_scores, updated_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (entity_id) DO UPDATE SET
            scores = EXCLUDED.scores,
            meta_scores = EXCLUDED.meta_scores,
            updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(&vector.entity_id)
    .bind(&scores_json)
    .bind(&meta_json)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

/// Load a single capability vector from the database.
pub async fn load_vector(
    pool: &PgPool,
    entity_id: &str,
    registry: &CapabilityRegistry,
) -> Result<Option<CapabilityVector>> {
    let row: Option<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "SELECT scores, meta_scores FROM capability_vectors WHERE entity_id = $1",
    )
    .bind(entity_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(scores_json, meta_json)| {
        CapabilityVector::from_json(entity_id, &scores_json, &meta_json, registry)
    }))
}

/// Load all capability vectors from the database.
pub async fn load_all_vectors(
    pool: &PgPool,
    registry: &CapabilityRegistry,
) -> Result<Vec<CapabilityVector>> {
    let rows: Vec<(String, serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "SELECT entity_id, scores, meta_scores FROM capability_vectors",
    )
    .fetch_all(pool)
    .await?;

    let vectors = rows
        .into_iter()
        .map(|(entity_id, scores_json, meta_json)| {
            CapabilityVector::from_json(&entity_id, &scores_json, &meta_json, registry)
        })
        .collect();

    Ok(vectors)
}

/// Compute cosine similarity between two equal-length float slices.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a < 1e-9 || norm_b < 1e-9 {
        return 0.0;
    }

    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::models::{CapabilityType, SignalBreakdown};

    fn make_cap(cap_id: &str, score: f32) -> ExtractedCapability {
        let mut cap = ExtractedCapability::new(
            "test_user".to_string(),
            CapabilityType::new(cap_id),
            score, // confidence -> setting it to the requested score directly so Top-5 sparsity builder keeps it
            crate::extraction::config::CapabilityTier::Emerging,
            SignalBreakdown::zero(),
            vec![],
            vec![],
        );
        cap.normalized_score = score;
        cap
    }

    #[test]
    fn test_vector_building_with_missing_traits() {
        let capabilities = vec![
            make_cap("DistributedAlgorithms", 0.8),
            make_cap("FrontendEngineering", 0.6),
        ];

        // Load registry to get proper indices
        let registry = crate::signals::capability_registry::CapabilityRegistry::load().unwrap();
        let vector = CapabilityVector::from_capabilities("test_user", &capabilities, &registry);

        assert_eq!(vector.entity_id, "test_user");
        assert_eq!(vector.scores.len(), registry.len());

        let da_idx = registry.index_of("DistributedAlgorithms").unwrap();
        assert_eq!(vector.scores[da_idx], 0.8);
    }

    #[test]
    fn test_meta_vector_systems_domain() {
        let registry = crate::signals::capability_registry::CapabilityRegistry::load().unwrap();
        let caps = vec![
            make_cap("DistributedAlgorithms", 0.9),
            make_cap("ConcurrentProgramming", 0.7),
        ];
        let v = CapabilityVector::from_capabilities("u1", &caps, &registry);
        // Systems meta score should be non-zero
        assert!(v.meta_scores[0] > 0.0, "Systems meta score should be > 0");
        assert_eq!(v.meta_scores.len(), 5);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![0.5, 0.3, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_hybrid_similarity_cross_domain() {
        let registry = crate::signals::capability_registry::CapabilityRegistry::load().unwrap();
        // Two users with different specializations but in same Systems meta-category
        let caps1 = vec![make_cap("DistributedAlgorithms", 0.9)];
        let caps2 = vec![make_cap("NetworkingEngineering", 0.9)];
        let v1 = CapabilityVector::from_capabilities("u1", &caps1, &registry);
        let v2 = CapabilityVector::from_capabilities("u2", &caps2, &registry);
        let sim = v1.hybrid_similarity(&v2);
        // Hybrid should be > 0 even though capability vectors don't overlap
        assert!(sim > 0.0, "Hybrid similarity should detect meta-category overlap");
    }
}
