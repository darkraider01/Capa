use anyhow::Result;
use sqlx::PgPool;
use std::time::Duration;

use crate::config::SearchConfig;
use crate::github_client::GithubClient;

/// Ingest multiple users with progress tracking and error handling
pub async fn ingest_multiple_users(
    pool: &PgPool,
    github_client: &GithubClient,
    user_list: &[String],
    config: &SearchConfig,
) -> Result<IngestionStats> {
    let mut stats = IngestionStats::default();
    let total = user_list.len().min(config.ingestion.max_users);

    println!("📥 Starting multi-entity ingestion");
    println!("   Total users: {}", total);
    println!(
        "   Rate limit: {}ms between users",
        config.ingestion.rate_limit_ms
    );
    println!("");

    for (i, username) in user_list.iter().take(total).enumerate() {
        println!("📥 [{}/{}] Ingesting: {}", i + 1, total, username);

        let mut attempts = 0;
        let mut success = false;

        while attempts < config.ingestion.retry_attempts && !success {
            match crate::pipeline::ingest_user(github_client, pool, username).await {
                Ok(_) => {
                    stats.successful.push(username.clone());
                    println!("  ✓ Success");
                    success = true;
                }
                Err(e) => {
                    attempts += 1;
                    if attempts < config.ingestion.retry_attempts {
                        eprintln!(
                            "  ⚠ Attempt {}/{} failed: {}",
                            attempts, config.ingestion.retry_attempts, e
                        );
                        eprintln!("    Retrying...");
                        tokio::time::sleep(Duration::from_millis(config.ingestion.rate_limit_ms))
                            .await;
                    } else {
                        let error_msg: String = e.to_string();
                        eprintln!("  ✗ Failed after {} attempts: {}", attempts, error_msg);
                        stats.failed.push((username.clone(), error_msg));
                    }
                }
            }
        }

        // Rate limiting between users
        if i < total - 1 {
            tokio::time::sleep(Duration::from_millis(config.ingestion.rate_limit_ms)).await;
        }

        // Progress update every 10 users
        if (i + 1) % 10 == 0 {
            println!("");
            println!("📊 Progress: {}/{} users ingested", i + 1, total);
            println!(
                "   Success: {} | Failed: {}",
                stats.successful.len(),
                stats.failed.len()
            );
            println!("");
        }
    }

    println!("");
    println!("✅ Multi-entity ingestion complete!");
    println!("   Total processed: {}", total);
    println!("   Successful: {}", stats.successful.len());
    println!("   Failed: {}", stats.failed.len());

    if !stats.failed.is_empty() {
        println!("");
        println!("❌ Failed users:");
        for (username, error) in &stats.failed {
            println!("   - {}: {}", username, error);
        }
    }

    Ok(stats)
}

/// Extract capabilities for multiple users
pub async fn extract_multiple_capabilities(
    pool: &PgPool,
    usernames: &[String],
    config: &SearchConfig,
    registry: &crate::signals::capability_registry::CapabilityRegistry,
) -> Result<usize> {
    let mut total_capabilities = 0;

    println!("");
    println!("🧠 Starting multi-entity capability extraction");
    println!("   Users to process: {}", usernames.len());
    println!("");

    for (i, username) in usernames.iter().enumerate() {
        println!(
            "🧠 [{}/{}] Extracting capabilities for: {}",
            i + 1,
            usernames.len(),
            username
        );

        match crate::extraction::extract_user_capabilities(pool, username).await {
            Ok(mut capabilities) => {
                let count: usize = capabilities.len();
                total_capabilities += count;

                // Apply score calibration
                for cap in &mut capabilities {
                    cap.normalized_score = crate::calibration::calibrate_score(
                        cap.confidence,
                        cap.capability_type.as_str(),
                        config,
                    );
                    cap.tier = crate::extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
                }

                // Store capabilities
                if let Err(e) =
                    crate::extraction::storage::store_capabilities(pool, &capabilities).await
                {
                    eprintln!("  ⚠ Failed to store capabilities: {}", e);
                } else {
                    println!("  ✓ Extracted {} capabilities", count);

                    // Build and store capability vector for similarity search
                    let vector = crate::similarity::CapabilityVector::from_capabilities(
                        username,
                        &capabilities,
                        registry,
                    );
                    if let Err(e) = crate::similarity::store_vector(pool, &vector, registry).await {
                        eprintln!("  ⚠ Failed to store capability vector: {}", e);
                    } else {
                        println!("  ✓ Stored capability vector");
                    }
                }
            }
            Err(e) => {
                eprintln!("  ✗ Extraction failed: {}", e);
            }
        }
    }

    println!("");
    println!("✅ Multi-entity extraction complete!");
    println!("   Total capabilities extracted: {}", total_capabilities);

    Ok(total_capabilities)
}

/// Ingestion statistics
#[derive(Debug, Default)]
pub struct IngestionStats {
    pub successful: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl IngestionStats {
    pub fn success_rate(&self) -> f32 {
        let total = self.successful.len() + self.failed.len();
        if total == 0 {
            return 0.0;
        }
        self.successful.len() as f32 / total as f32
    }
}
