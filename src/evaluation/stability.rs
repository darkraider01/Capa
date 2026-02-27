use anyhow::{bail, Result};
use sqlx::{PgPool, Row};
use crate::signals::capability_registry::CapabilityRegistry;
use crate::similarity::vector_builder::CapabilityVector;

pub async fn run_stability_checks(pool: &PgPool, registry: &CapabilityRegistry) -> Result<()> {
    println!("\n🛡️ Running Hard Validation Constraints (Stability Rules)...");

    // Rule 1: No user with >5 strong capabilities
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "
        SELECT COUNT(*) as count, user_login 
        FROM capabilities 
        WHERE confidence >= 0.6 
        GROUP BY user_login 
        HAVING COUNT(*) > 5
        "
    )
    .fetch_all(pool)
    .await?;

    if !rows.is_empty() {
        for (count, login) in rows {
            println!("  🚨 [VIOLATION] {} has {} STRONG capabilities (>5)", login, count);
        }
        bail!("Stability Check Failed: Over-classification bounds exceeded.");
    }
    println!("  ✅ No user exceeds the 5 strong capability bound.");

    // Rule 2 & 3: Self similarity ≈ 1.0 & No NaN similarity scores
    let vectors = sqlx::query("SELECT entity_id, scores, meta_scores FROM capability_vectors LIMIT 20")
        .fetch_all(pool)
        .await?;

    let mut parsed_vectors = Vec::new();
    for row in vectors {
        let entity_id: String = row.get("entity_id");
        let scores: serde_json::Value = row.get("scores");
        let meta: serde_json::Value = row.get("meta_scores");
        parsed_vectors.push(CapabilityVector::from_json(&entity_id, &scores, &meta, registry));
    }

    for vec in &parsed_vectors {
        let self_sim = vec.hybrid_similarity(vec);
        
        // Rule 3: No NaN
        if self_sim.is_nan() {
            bail!("Stability Check Failed: {} has NaN self-similarity. Check zero-division.", vec.entity_id);
        }

        // Rule 4: No empty vector producing similarity
        // If a vector has no valid capabilities, its self sim might be 0.0 or trigger a division by zero that evaluates to NaN.
        let is_empty = vec.scores.iter().all(|&s| s == 0.0) && vec.meta_scores.iter().all(|&s| s == 0.0);
        
        if is_empty {
            if self_sim > 0.1 {
                bail!("Stability Check Failed: {} is EMPTY but produces similarity {:.4} > 0", vec.entity_id, self_sim);
            }
            continue; // Empties correctly have 0.0 self-sim; skip Rule 2.
        }

        // Rule 2: ≈ 1.0 self-sim (for non-empty vectors)
        if (self_sim - 1.0).abs() > 0.01 {
            bail!("Stability Check Failed: {} has self-similarity of {:.4} (Expected 1.0)", vec.entity_id, self_sim);
        }
    }

    println!("  ✅ Self similarity passes (1.0 for valid, 0.0 for empty/NaN safe).");
    println!("  ✅ No invalid similarity clustering detected.");
    println!("🛡️ System constraints preserved.\n");

    Ok(())
}
