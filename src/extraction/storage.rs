use super::models::ExtractedCapability;
use anyhow::Result;
use sqlx::{PgPool, Row};

/// Ensure f32 value is safe for database insertion (no NaN or Infinity)
fn safe_f32(value: f32) -> f32 {
    if value.is_nan() || value.is_infinite() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Insert an extracted capability into the database with breakdown
pub async fn insert_capability(pool: &PgPool, capability: &ExtractedCapability) -> Result<()> {
    let evidence_json = serde_json::to_value(&capability.evidence_keywords)?;
    let repos_json = serde_json::to_value(&capability.evidence_repos)?;

    // Validate all float values before insertion
    let confidence = safe_f32(capability.confidence);
    let keyword_score = safe_f32(capability.signal_breakdown.keyword_score);
    let repo_score = safe_f32(capability.signal_breakdown.filename_score);
    let language_score = safe_f32(capability.signal_breakdown.language_score);
    let structural_score = safe_f32(capability.signal_breakdown.structure_score);
    let raw_score = safe_f32(capability.signal_breakdown.raw_score);
    let time_decay_factor = safe_f32(capability.signal_breakdown.time_decay_factor);
    let correlation_boost = safe_f32(capability.signal_breakdown.correlation_boost);
    let dependency_score = safe_f32(capability.signal_breakdown.dependency_score);
    let activity_score = safe_f32(capability.signal_breakdown.activity_score);
    let normalized_score = safe_f32(capability.normalized_score);

    sqlx::query(
        r#"
        INSERT INTO capabilities (
            id, user_login, capability_type, confidence, normalized_score, tier,
            evidence, evidence_repos,
            keyword_score, repo_score, language_score, structural_score,
            dependency_score, activity_score,
            raw_score, time_decay_factor, correlation_boost,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        ON CONFLICT (id) DO UPDATE SET
            normalized_score = EXCLUDED.normalized_score,
            tier = EXCLUDED.tier
        "#,
    )
    .bind(capability.id)
    .bind(&capability.user_login)
    .bind(capability.capability_type.as_str())
    .bind(confidence)
    .bind(normalized_score)
    .bind(capability.tier.as_str())
    .bind(evidence_json)
    .bind(repos_json)
    .bind(keyword_score)
    .bind(repo_score)
    .bind(language_score)
    .bind(structural_score)
    .bind(dependency_score)
    .bind(activity_score)
    .bind(raw_score)
    .bind(time_decay_factor)
    .bind(correlation_boost)
    .bind(capability.timestamp)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get all capabilities for a user
pub async fn get_user_capabilities(pool: &PgPool, user_login: &str) -> Result<Vec<(String, f32)>> {
    let rows = sqlx::query(
        r#"
        SELECT capability_type, confidence
        FROM capabilities
        WHERE user_login = $1
        ORDER BY confidence DESC
        "#,
    )
    .bind(user_login)
    .fetch_all(pool)
    .await?;

    let mut capabilities = Vec::new();
    for row in rows {
        let cap_type: String = row.get("capability_type");
        let confidence: f32 = row.get("confidence");
        capabilities.push((cap_type, confidence));
    }

    Ok(capabilities)
}

/// Store multiple capabilities (batch insert)
pub async fn store_capabilities(pool: &PgPool, capabilities: &[ExtractedCapability]) -> Result<()> {
    for capability in capabilities {
        insert_capability(pool, capability).await?;
    }
    Ok(())
}

/// Load all capabilities from database (for multi-entity indexing)
pub async fn load_all_capabilities(pool: &PgPool) -> Result<Vec<ExtractedCapability>> {
    let rows = sqlx::query(
        r#"
        SELECT 
            id, user_login, capability_type, confidence, tier,
            evidence, evidence_repos,
            keyword_score, repo_score, language_score, structural_score,
            dependency_score, activity_score,
            raw_score, time_decay_factor, correlation_boost,
            created_at
        FROM capabilities
        ORDER BY user_login, confidence DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut capabilities = Vec::new();
    for row in rows {
        let id: uuid::Uuid = row.get("id");
        let user_login: String = row.get("user_login");
        let capability_type_str: String = row.get("capability_type");
        let confidence_f64: f64 = row.get("confidence");
        let confidence = confidence_f64 as f32;
        let normalized_score_f64: f64 = row.try_get("normalized_score").unwrap_or(0.0);
        let normalized_score = normalized_score_f64 as f32;
        let tier_str: String = row.get("tier");
        let evidence_json: serde_json::Value = row.get("evidence");
        let repos_json: serde_json::Value = row
            .try_get("evidence_repos")
            .unwrap_or(serde_json::Value::Array(Vec::new()));

        // Database stores timestamp as TIMESTAMPTZ, but we'll extract the epoch
        // Try to get as i64 first, otherwise get as timestamp and convert
        let timestamp: i64 = row
            .try_get::<i64, _>("created_at")
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());

        let capability_type = super::models::CapabilityType::from_str(&capability_type_str);

        let tier = super::config::CapabilityTier::from_str(&tier_str)
            .unwrap_or(super::config::CapabilityTier::Weak);

        let evidence_keywords: Vec<String> =
            serde_json::from_value(evidence_json).unwrap_or_default();
        let evidence_repos: Vec<String> = serde_json::from_value(repos_json).unwrap_or_default();

        let signal_breakdown = super::models::SignalBreakdown {
            keyword_score: row.try_get("keyword_score").unwrap_or(0.0),
            dependency_score: row.try_get("dependency_score").unwrap_or(0.0),
            filename_score: row.try_get("repo_score").unwrap_or(0.0),
            structure_score: row.try_get("structural_score").unwrap_or(0.0),
            language_score: row.try_get("language_score").unwrap_or(0.0),
            activity_score: row.try_get("activity_score").unwrap_or(0.0),
            raw_score: row.try_get("raw_score").unwrap_or(0.0),
            time_decay_factor: row.try_get("time_decay_factor").unwrap_or(1.0),
            correlation_boost: row.try_get("correlation_boost").unwrap_or(0.0),
        };

        capabilities.push(ExtractedCapability {
            id,
            user_login,
            capability_type,
            confidence,
            normalized_score,
            tier,
            evidence_keywords,
            evidence_repos,
            evidence_deps: Vec::new(),
            signal_breakdown,
            timestamp,
        });
    }

    Ok(capabilities)
}
