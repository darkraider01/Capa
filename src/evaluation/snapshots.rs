use anyhow::Result;
use sqlx::PgPool;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub capability_type: String,
    pub confidence: f32,
    pub tier: String,
}

#[derive(Serialize, Deserialize)]
pub struct UserSnapshot {
    pub username: String,
    pub top_capabilities: Vec<CapabilitySnapshot>,
}

pub async fn snapshot_profiles(pool: &PgPool) -> Result<()> {
    println!("\n📸 Taking capability snapshots for all users...");
    
    // Ensure dir exists
    let snapshot_dir = Path::new("snapshots");
    if !snapshot_dir.exists() {
        fs::create_dir_all(snapshot_dir)?;
    }

    // Fetch all users
    let users: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT user_login FROM capabilities")
        .fetch_all(pool)
        .await?;

    if users.is_empty() {
        println!("No users found in database to snapshot.");
        return Ok(());
    }

    let mut count = 0;
    for (username,) in users {
        let caps = sqlx::query_as::<_, (String, f64, String)>(
            "SELECT capability_type, confidence, tier 
             FROM capabilities 
             WHERE user_login = $1 
             ORDER BY confidence DESC 
             LIMIT 5"
        )
        .bind(&username)
        .fetch_all(pool)
        .await?;

        let mut top_capabilities = Vec::new();
        for (cap_type, conf, tier) in caps {
            top_capabilities.push(CapabilitySnapshot {
                capability_type: cap_type,
                confidence: conf as f32,
                tier,
            });
        }

        let snapshot = UserSnapshot {
            username: username.clone(),
            top_capabilities,
        };

        let file_path = snapshot_dir.join(format!("{}.json", username));
        let json = serde_json::to_string_pretty(&snapshot)?;
        fs::write(file_path, json)?;
        count += 1;
    }

    println!("✅ Wrote {} snapshot files to snapshots/ directory.\n", count);
    Ok(())
}

pub async fn detect_drift(pool: &PgPool) -> Result<()> {
    println!("\n📉 Detecting Capability Drift vs Last Snapshot...");

    let snapshot_dir = Path::new("snapshots");
    if !snapshot_dir.exists() {
        println!("No snapshots directory found. Run --snapshot-profiles first.");
        return Ok(());
    }

    let users: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT user_login FROM capabilities")
        .fetch_all(pool)
        .await?;

    if users.is_empty() {
        println!("Database is empty.");
        return Ok(());
    }

    let mut total_compared = 0;
    let mut primary_changed = 0;

    for (username,) in users {
        let file_path = snapshot_dir.join(format!("{}.json", username));
        if !file_path.exists() {
            continue;
        }

        let json = fs::read_to_string(&file_path)?;
        let snapshot: UserSnapshot = serde_json::from_str(&json)?;

        // Current primary
        let current_primary: Option<(String,)> = sqlx::query_as(
            "SELECT capability_type FROM capabilities WHERE user_login = $1 ORDER BY confidence DESC LIMIT 1"
        )
        .bind(&username)
        .fetch_optional(pool)
        .await?;

        if let Some(old_primary) = snapshot.top_capabilities.into_iter().next() {
            if let Some((new_primary,)) = current_primary {
                total_compared += 1;
                if old_primary.capability_type != new_primary {
                    println!("  [DRIFT] {} primary changed: {} -> {}", username, old_primary.capability_type, new_primary);
                    primary_changed += 1;
                }
            }
        }
    }

    if total_compared == 0 {
        println!("No matching snapshots found to compare.");
        return Ok(());
    }

    let drift_ratio = primary_changed as f64 / total_compared as f64;
    println!("\nTotal Users Compared:    {}", total_compared);
    println!("Primary Cap Changed:     {}", primary_changed);
    println!("Drift Ratio:             {:.1}%\n", drift_ratio * 100.0);

    if drift_ratio > 0.40 {
        println!("🚨 ALERT: Drift ratio exceeds 40% threshold! Major score regression detected.");
    } else {
        println!("✅ OK: Drift is within stable bounds (< 40%).");
    }

    Ok(())
}
