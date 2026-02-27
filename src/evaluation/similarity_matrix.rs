use anyhow::Result;
use sqlx::{PgPool, Row};
use crate::signals::capability_registry::CapabilityRegistry;
use crate::similarity::vector_builder::CapabilityVector;

pub async fn compute_similarity_matrix(pool: &PgPool, registry: &CapabilityRegistry) -> Result<()> {
    let target_users = vec![
        "dtolnay", 
        "gaearon", 
        "karpathy", 
        "burntsushi", 
        "matklad", 
        "sindresorhus", 
        "tj", 
        "defunkt", 
        "torvalds", 
        "ry"
    ];

    println!("\n🧩 Computing Similarity Matrix for mapped reference users:");

    let mut vectors = Vec::new();

    for user in &target_users {
        let query = "SELECT entity_id, scores, meta_scores FROM capability_vectors WHERE entity_id = $1";
        if let Ok(Some(row)) = sqlx::query(query).bind(user).fetch_optional(pool).await {
            let entity_id: String = row.get("entity_id");
            let scores: serde_json::Value = row.get("scores");
            let meta: serde_json::Value = row.get("meta_scores");
            let vec = CapabilityVector::from_json(&entity_id, &scores, &meta, registry);
            vectors.push(vec);
        } else {
            println!("  [WARN] {} not found in db.", user);
        }
    }

    if vectors.len() < 2 {
        println!("Not enough users processed to compute matrix.");
        return Ok(());
    }

    println!("--------------------------------------------------");
    for i in 0..vectors.len() {
        for j in (i + 1)..vectors.len() {
            let v1 = &vectors[i];
            let v2 = &vectors[j];
            let sim = v1.hybrid_similarity(v2);
            println!("{:12} vs {:12} : {:.3}", v1.entity_id, v2.entity_id, sim);
        }
    }
    println!("--------------------------------------------------");

    // Rules check
    let mut all_near_zero = true;
    let mut all_near_one = true;

    for i in 0..vectors.len() {
        for j in (i + 1)..vectors.len() {
            let sim = vectors[i].hybrid_similarity(&vectors[j]);
            if sim > 0.05 { all_near_zero = false; }
            if sim < 0.95 { all_near_one = false; }
        }
    }

    if all_near_zero {
        println!("🚨 FAIL: All similarities cluster near 0. Vectors might be too sparse or orthogonal.");
    } else if all_near_one {
        println!("🚨 FAIL: All similarities cluster near 1. Vectors might be ignoring IDF weights.");
    } else {
        println!("✅ PASS: Matrix variance is healthy.");
    }

    // Specific domain check (dtolnay vs matklad = high; dtolnay vs gaearon = low)
    let find_vec = |name: &str| vectors.iter().find(|v| v.entity_id == name);

    if let (Some(dtolnay), Some(matklad), Some(gaearon)) = (find_vec("dtolnay"), find_vec("matklad"), find_vec("gaearon")) {
        let same_domain = dtolnay.hybrid_similarity(matklad);
        let cross_domain = dtolnay.hybrid_similarity(gaearon);

        println!("\nDomain Validation:");
        println!("  dtolnay vs matklad (Same Domain)  : {:.3}", same_domain);
        println!("  dtolnay vs gaearon (Cross Domain) : {:.3}", cross_domain);
        
        if same_domain > cross_domain {
            println!("  ✅ PASS: Same-domain similarity > Cross-domain similarity");
        } else {
            println!("  🚨 FAIL: Matrix failed domain separation check!");
        }
    }

    println!();
    Ok(())
}
