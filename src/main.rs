#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

mod batch_ingestion;
mod calibration;
mod config;
mod extraction;
mod github_client;
mod models;
mod pipeline;
mod profile;
mod search;
mod signals;
mod similarity;
mod storage;
mod evaluation;

use anyhow::Result;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    // Get configuration from environment
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env file");
    let github_token =
        std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN must be set in .env file");

    // Check for multi-entity, verify, analyze, profile, similar or help mode
    let args: Vec<String> = std::env::args().collect();
    let help_mode = args.len() == 1 || (args.len() > 1 && (args[1] == "--help" || args[1] == "-h"));
    let multi_entity_mode = args.len() > 1 && args[1] == "--multi-entity";
    let verify_mode = args.len() > 1 && args[1] == "--verify";
    let analyze_mode = args.len() > 1 && args[1] == "--analyze";
    let profile_mode = args.len() > 2 && args[1] == "--profile";
    let profile_target = if profile_mode {
        Some(args[2].clone())
    } else {
        None
    };
    let similar_mode = args.len() > 2 && args[1] == "--similar";
    let similar_target = if similar_mode {
        Some(args[2].clone())
    } else {
        None
    };
    let reingest_mode = args.len() > 2 && args[1] == "--reingest";
    let reingest_target = if reingest_mode {
        Some(args[2].clone())
    } else {
        None
    };
    let recalibrate_mode = args.len() > 1 && args[1] == "--recalibrate";
    let json_mode = args.iter().any(|arg| arg == "--json");
    
    // Evaluation & Monitoring Layer Modes
    let evaluate_archetypes_mode = args.len() > 1 && args[1] == "--evaluate-archetypes";
    let analyze_distribution_mode = args.len() > 1 && args[1] == "--analyze-distribution";
    let explain_mode = args.len() > 2 && args[1] == "--explain";
    let explain_target = if explain_mode { Some(args[2].clone()) } else { None };
    let snapshot_profiles_mode = args.len() > 1 && args[1] == "--snapshot-profiles";
    let detect_drift_mode = args.len() > 1 && args[1] == "--detect-drift";
    let similarity_matrix_mode = args.len() > 1 && args[1] == "--similarity-matrix";
    let synthetic_tests_mode = args.len() > 1 && args[1] == "--run-synthetic-tests";
    let describe_registry_mode = args.len() > 1 && args[1] == "--describe-registry";

    let target_username = if help_mode
        || multi_entity_mode
        || verify_mode
        || analyze_mode
        || similar_mode
        || profile_mode
        || recalibrate_mode
        || evaluate_archetypes_mode
        || analyze_distribution_mode
        || explain_mode
        || snapshot_profiles_mode
        || detect_drift_mode
        || similarity_matrix_mode
        || synthetic_tests_mode
        || describe_registry_mode
    {
        String::new() // Not used directly in these modes
    } else {
        std::env::var("TARGET_USERNAME").unwrap_or_else(|_| "darkraider01".to_string())
    };

    if help_mode {
        println!("Capa CLI");
        println!("\nUsage:");
        println!("  cargo run -- [OPTIONS]");
        println!("\nOptions:");
        println!("  --help, -h                  Print this help message");
        println!("  --multi-entity              Run batch ingestion for all users in config");
        println!("  --verify                    Run ingestion with data integrity checks");
        println!("  --analyze <user>            Show raw capability extraction scores for a user");
        println!("  --profile <user>            Generate a human-readable Intelligence Report");
        println!(
            "  --similar <user> [--limit N] Find entities with overlapping capability vectors"
        );
        println!("  --reingest <user>           Wipe and re-ingest a user's data");
        println!("  --recalibrate               Recompute calibration stats from DB and save to search.toml");
        println!("  --evaluate-archetypes       Evaluate expected vs actual profiles for fixed set of users");
        println!("  --analyze-distribution      Analyze capability distribution across all users");
        println!("  --explain <user>            Show channel score breakdown for a user");
        println!("  --snapshot-profiles         Take a JSON snapshot of all user capabilities");
        println!("  --detect-drift              Detect capability drift compared to last snapshot");
        println!("  --similarity-matrix         Compute pairwise similarity for reference users");
        println!("  --run-synthetic-tests       Run adversarial synthetic isolated signal tests");
        return Ok(());
    }

    // Determine displayed target
    let display_target = if profile_mode {
        profile_target.clone().unwrap_or_default()
    } else if analyze_mode {
        args.get(2).cloned().unwrap_or_default()
    } else if similar_mode {
        similar_target.clone().unwrap_or_default()
    } else if explain_mode {
        explain_target.clone().unwrap_or_default()
    } else {
        target_username.clone()
    };

    // Removed duplicate print lines
    // Connect to database
    // println!("🔌 Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;
    println!("✓ Database connected");

    if multi_entity_mode {
        println!("📊 Mode: Multi-Entity Ingestion");
    } else if verify_mode {
        println!("🔍 Mode: Verification");
    } else {
        println!("📊 Target user: {}", display_target);
    }

    // Only recreate if tables don't exist (idempotent)
    println!("🔧 Checking schema...");

    // Users table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id SERIAL PRIMARY KEY,
            github_login TEXT UNIQUE NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
    "#,
    )
    .execute(&pool)
    .await?;

    // Repositories table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS repositories (
            id BIGINT PRIMARY KEY,
            user_login TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            stars BIGINT DEFAULT 0,
            language TEXT,
            pushed_at TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (user_login) REFERENCES users(github_login)
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // Migration: add pushed_at to existing tables
    sqlx::query("ALTER TABLE repositories ADD COLUMN IF NOT EXISTS pushed_at TEXT")
        .execute(&pool)
        .await?;

    // Commits table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS commits (
            sha TEXT PRIMARY KEY,
            repo_id BIGINT NOT NULL,
            message TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (repo_id) REFERENCES repositories(id)
        )
    "#,
    )
    .execute(&pool)
    .await?;

    // Capabilities table with ALL columns including new ones
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS capabilities (
            id UUID PRIMARY KEY,
            user_login TEXT NOT NULL,
            capability_type TEXT NOT NULL,
            confidence FLOAT NOT NULL,
            normalized_score FLOAT NOT NULL DEFAULT 0,
            tier TEXT NOT NULL,
            evidence JSONB NOT NULL,
            evidence_repos JSONB,
            keyword_score FLOAT,
            repo_score FLOAT,
            language_score FLOAT,
            structural_score FLOAT,
            dependency_score FLOAT,
            activity_score FLOAT,
            raw_score FLOAT,
            time_decay_factor FLOAT,
            correlation_boost FLOAT,
            created_at BIGINT NOT NULL,
            FOREIGN KEY (user_login) REFERENCES users(github_login)
        )
    "#,
    )
    .execute(&pool)
    .await?;

    // Migration for capability_vectors: If old schema exists (no 'scores' column), drop it so it can be recreated
    let check_vectors_schema: Result<(i64,), _> = sqlx::query_as(
        "SELECT count(column_name) FROM information_schema.columns WHERE table_name='capability_vectors' AND column_name='scores'"
    )
    .fetch_one(&pool)
    .await;

    if let Ok((count,)) = check_vectors_schema {
        if count == 0 {
            println!("🔄 Migrating capability_vectors to use JSONB schema...");
            sqlx::query("DROP TABLE IF EXISTS capability_vectors").execute(&pool).await?;
        }
    }

    // Capability vectors for similarity search — JSONB schema (dynamic dimensions)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS capability_vectors (
            entity_id TEXT PRIMARY KEY,
            scores JSONB NOT NULL DEFAULT '{}',
            meta_scores JSONB NOT NULL DEFAULT '{}',
            updated_at BIGINT NOT NULL,
            FOREIGN KEY (entity_id) REFERENCES users(github_login)
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // Dependencies table for dependency-signal channel
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dependencies (
            id SERIAL PRIMARY KEY,
            repo_id BIGINT NOT NULL,
            name TEXT NOT NULL,
            FOREIGN KEY (repo_id) REFERENCES repositories(id)
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // Ensure normalized_score column exists (migration)
    sqlx::query("ALTER TABLE capabilities ADD COLUMN IF NOT EXISTS normalized_score FLOAT NOT NULL DEFAULT 0")
        .execute(&pool)
        .await?;
        
    // Ensure new scoring columns exist (migration)
    sqlx::query("ALTER TABLE capabilities ADD COLUMN IF NOT EXISTS dependency_score FLOAT")
        .execute(&pool)
        .await?;
    sqlx::query("ALTER TABLE capabilities ADD COLUMN IF NOT EXISTS activity_score FLOAT")
        .execute(&pool)
        .await?;

    // Create indexes
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_repos_user ON repositories(user_login)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_repos_language ON repositories(language)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_commits_repo ON commits(repo_id)")
        .execute(&pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_capabilities_user ON capabilities(user_login)")
        .execute(&pool)
        .await?;

    // Load capability registry (foundation for all signal modules)
    let registry = signals::capability_registry::CapabilityRegistry::load()
        .expect("Failed to load config/capabilities.toml — ensure it exists");
    println!("✓ Capability registry loaded ({} capabilities)", registry.len());

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_capabilities_type ON capabilities(capability_type)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_capabilities_confidence ON capabilities(confidence DESC)",
    )
    .execute(&pool)
    .await?;

    println!("✓ Schema checked\n");

    // Perform wipe if in multi-entity mode now that schema is guaranteed
    if multi_entity_mode {
        println!("🧹 Wiping existing capability tables for a fresh run...");
        sqlx::query("DELETE FROM capability_vectors;")
            .execute(&pool)
            .await?;
        sqlx::query("DELETE FROM capabilities;")
            .execute(&pool)
            .await?;
    }

    // Initialize GitHub client
    let github_client = github_client::GithubClient::new(github_token);

    if analyze_mode {
        println!("\n📊 Analyzing Score Distribution...");

        // Fetch all raw scores
        // We need a struct to hold the results
        #[derive(sqlx::FromRow)]
        struct ScoreRow {
            capability_type: String,
            raw_score: f64,
        }

        let rows: Vec<ScoreRow> = sqlx::query_as(
            r#"
            SELECT capability_type, raw_score 
            FROM capabilities
            WHERE raw_score IS NOT NULL
        "#,
        )
        .fetch_all(&pool)
        .await?;

        if rows.is_empty() {
            println!("⚠ No capabilities found to analyze.");
            return Ok(());
        }

        use std::collections::HashMap;
        let mut type_stats: HashMap<String, Vec<f64>> = HashMap::new();

        for row in rows {
            type_stats
                .entry(row.capability_type)
                .or_default()
                .push(row.raw_score);
        }

        println!(
            "\n{:<30} | {:<8} | {:<10} | {:<10} | {:<10} | {:<10}",
            "Capability Type", "Count", "Mean", "StdDev", "Min", "Max"
        );
        println!("{}", "-".repeat(90));

        for (cap_type, scores) in type_stats {
            let count = scores.len();
            let sum: f64 = scores.iter().sum();
            let mean = sum / count as f64;

            let variance: f64 = scores
                .iter()
                .map(|val| {
                    let diff = mean - *val;
                    diff * diff
                })
                .sum::<f64>()
                / count as f64;

            let std_dev = variance.sqrt();

            let min = scores.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let max = scores.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

            println!(
                "{:<30} | {:<8} | {:<10.4} | {:<10.4} | {:<10.4} | {:<10.4}",
                cap_type, count, mean, std_dev, min, max
            );
        }

        return Ok(());
    }

    if verify_mode {
        println!("\n🔍 Running verification...");

        // Count total capabilities
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM capabilities")
            .fetch_one(&pool)
            .await?;
        println!("✅ Total capabilities in DB: {}", count.0);

        // Check for NULLs
        let nulls: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM capabilities 
            WHERE keyword_score IS NULL 
               OR repo_score IS NULL 
               OR language_score IS NULL 
               OR structural_score IS NULL
               OR raw_score IS NULL
               OR time_decay_factor IS NULL
               OR correlation_boost IS NULL
        "#,
        )
        .fetch_one(&pool)
        .await?;

        if nulls.0 == 0 {
            println!("✅ No NULL values found in score columns");
        } else {
            println!(
                "❌ Found {} rows with NULL values in score columns!",
                nulls.0
            );
        }

        // Check index directory
        let index_path = std::path::Path::new("index");
        if index_path.exists() {
            let entry_count = std::fs::read_dir(index_path)?.count();
            println!("✅ Index directory exists with {} files", entry_count);
        } else {
            println!("❌ Index directory missing!");
        }

        return Ok(());
    }

    let search_config = config::SearchConfig::load()?;

    if multi_entity_mode {
        let user_config = config::UserListConfig::load()?;

        // Multi-entity ingestion mode
        println!("");
        println!("═══════════════════════════════════════");
        println!("  MULTI-ENTITY INGESTION MODE");
        println!("═══════════════════════════════════════");
        println!("");

        // Get enabled users
        let users = user_config.get_enabled_users(&search_config.ingestion.categories);

        println!("📋 Loaded {} users from configuration", users.len());
        println!("   Max to ingest: {}", search_config.ingestion.max_users);
        println!("");

        println!("\n🔄 SKIPPING GITHUB FETCHING: Re-calculating capabilities locally only...\n");
        // let stats = batch_ingestion::ingest_multiple_users(
        //     &pool,
        //     &github_client,
        //     &users,
        //     &search_config,
        // ).await?;

        // Dummy stats for skipped ingestion to allow compilation
        let stats = batch_ingestion::IngestionStats {
            successful: users.clone(),
            failed: Vec::new(),
        };

        println!("");
        println!("📊 Ingestion Statistics:");
        println!("   Success rate: {:.1}%", stats.success_rate() * 100.0);
        println!("");

        // Extract capabilities for all successful users
        let _total_caps = batch_ingestion::extract_multiple_capabilities(
            &pool,
            &stats.successful,
            &search_config,
            &registry,
        )
        .await?;

        println!("");
        println!("🔍 Building search index...");

        // Load all capabilities from database
        let all_capabilities = extraction::storage::load_all_capabilities(&pool).await?;

        // Build index
        let index = search::CapabilityIndex::create("./index")?;
        let mut writer = index.get_writer()?;
        search::index_capabilities(&mut writer, &all_capabilities, &index.schema)?;

        println!(
            "✅ Index built with {} capabilities from {} users!",
            all_capabilities.len(),
            stats.successful.len()
        );

        println!("✅ Multi-entity ingestion and extraction complete!");
        println!("   {} users processed successfully", stats.successful.len());

        // Run validation constraints to ensure the batch run is sane
        crate::evaluation::stability::run_stability_checks(&pool, &registry).await?;

        return Ok(());
    }
    if reingest_mode {
        if let Some(target) = reingest_target {
            println!("\n🔄 Re-ingesting user: {} (wiping old data first)", target);

            // Wipe existing data for the user in dependency order
            sqlx::query("DELETE FROM capability_vectors WHERE entity_id = $1")
                .bind(&target).execute(&pool).await?;
            sqlx::query("DELETE FROM capabilities WHERE user_login = $1")
                .bind(&target).execute(&pool).await?;
            sqlx::query(
                "DELETE FROM commits WHERE repo_id IN \
                 (SELECT id FROM repositories WHERE user_login = $1)"
            )
            .bind(&target).execute(&pool).await?;
            sqlx::query("DELETE FROM repositories WHERE user_login = $1")
                .bind(&target).execute(&pool).await?;
            sqlx::query("DELETE FROM users WHERE github_login = $1")
                .bind(&target).execute(&pool).await?;

            println!("🧹 Old data wiped. Starting fresh ingestion...");

            // Re-ingest cleanly through the pipeline (fork filter is now active)
            match pipeline::ingest_user(&github_client, &pool, &target).await {
                Ok(_) => {
                    println!("✅ Ingestion complete. Extracting capabilities...");
                    match extraction::extract_user_capabilities(&pool, &target).await {
                        Ok(mut capabilities) => {
                            for cap in &mut capabilities {
                                cap.normalized_score = calibration::calibrate_score(
                                    cap.confidence,
                                    cap.capability_type.as_str(),
                                    &search_config,
                                );
                                cap.tier = extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
                            }
                            extraction::storage::store_capabilities(&pool, &capabilities).await?;

                            let vector = similarity::CapabilityVector::from_capabilities(&target, &capabilities, &registry);
                            let _ = similarity::store_vector(&pool, &vector, &registry).await;

                            println!("✅ Re-ingestion complete! Run --profile {} to see the updated report.", target);
                        }
                        Err(e) => eprintln!("❌ Extraction failed: {}", e),
                    }
                }
                Err(e) => eprintln!("❌ Ingestion failed: {}", e),
            }
        }
        return Ok(());
    }

    if recalibrate_mode {
        println!("📐 Running capability score recalibration...");

        // Load all capabilities from DB
        let all_caps = extraction::storage::load_all_capabilities(&pool).await?;

        if all_caps.is_empty() {
            println!("⚠️  No capabilities found in database. Ingest some users first.");
            return Ok(());
        }

        // Group scores by capability type
        let mut scores_by_type: std::collections::HashMap<String, Vec<f32>> =
            std::collections::HashMap::new();
        for cap in &all_caps {
            scores_by_type
                .entry(cap.capability_type.0.clone())
                .or_default()
                .push(cap.confidence);
        }

        // Compute mean and std_dev per capability (with 0.08 floor)
        let mut new_stats: std::collections::HashMap<String, crate::config::TypeStats> =
            std::collections::HashMap::new();

        for (cap_type, scores) in &scores_by_type {
            let n = scores.len() as f32;
            let mean = scores.iter().sum::<f32>() / n;
            let variance = scores.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n;
            let std_dev = variance.sqrt().max(0.08); // 0.08 floor prevents instability

            println!(
                "  {} | n={} | mean={:.3} | std_dev={:.3}",
                cap_type,
                scores.len(),
                mean,
                std_dev
            );

            new_stats.insert(
                cap_type.clone(),
                crate::config::TypeStats { mean, std_dev },
            );
        }

        // Update search_config and write back to search.toml
        let mut updated_config = search_config.clone();
        updated_config.ranking.calibration.stats = new_stats;
        updated_config.ranking.calibration.enabled = true;

        let toml_str = toml::to_string_pretty(&updated_config)
            .unwrap_or_else(|e| format!("# Failed to serialize: {}", e));
        std::fs::write("config/search.toml", toml_str)?;

        println!(
            "\n✅ Calibration complete! Updated {} capability stats in config/search.toml.",
            scores_by_type.len()
        );

        // Recompute normalized scores and tiers for all capabilities in the DB
        println!("🔄 Applying new calibration to existing capabilities...");
        let mut caps_by_user: std::collections::HashMap<String, Vec<extraction::models::ExtractedCapability>> = std::collections::HashMap::new();
        for mut cap in all_caps {
            cap.normalized_score = calibration::calibrate_score(
                cap.confidence,
                cap.capability_type.as_str(),
                &updated_config,
            );
            cap.tier = extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
            caps_by_user.entry(cap.user_login.clone()).or_default().push(cap);
        }

        // Store updated capabilities and verify vectors
        for (user, user_caps) in caps_by_user {
            extraction::storage::store_capabilities(&pool, &user_caps).await?;
            let vector = similarity::CapabilityVector::from_capabilities(&user, &user_caps, &registry);
            let _ = similarity::store_vector(&pool, &vector, &registry).await;
        }

        println!("✅ Existing database records updated with new tiers and scores.");

        // Run validation constraints to ensure recalibration didn't shift rules too hard
        crate::evaluation::stability::run_stability_checks(&pool, &registry).await?;

        return Ok(());
    }

    if evaluate_archetypes_mode {
        crate::evaluation::archetypes::evaluate_archetypes(&pool, &search_config).await?;
        return Ok(());
    }
    if analyze_distribution_mode {
        crate::evaluation::distribution::analyze_distribution(&pool).await?;
        return Ok(());
    }
    if explain_mode {
        if let Some(target) = explain_target {
            crate::evaluation::explain::explain_user(&pool, &target, &github_client, json_mode).await?;
        }
        return Ok(());
    }
    if snapshot_profiles_mode {
        crate::evaluation::snapshots::snapshot_profiles(&pool).await?;
        return Ok(());
    }
    if detect_drift_mode {
        crate::evaluation::snapshots::detect_drift(&pool).await?;
        return Ok(());
    }
    if similarity_matrix_mode {
        crate::evaluation::similarity_matrix::compute_similarity_matrix(&pool, &registry).await?;
        return Ok(());
    }
    if synthetic_tests_mode {
        crate::evaluation::synthetic::run_synthetic_tests(&registry).await?;
        return Ok(());
    }

    // AI Pipeline helper mode
    if describe_registry_mode {
        if json_mode {
            let mut capabilities_json = Vec::new();
            for cap in &registry.capabilities {
                capabilities_json.push(serde_json::json!({
                    "id": cap.id,
                    "display_name": cap.display_name,
                    "meta_category": cap.meta_category,
                    "keywords": cap.keywords.strict
                }));
            }
            let payload = serde_json::json!({
                "capabilities": capabilities_json
            });
            println!("{}", serde_json::to_string(&payload).unwrap());
        } else {
            println!("Registry describing currently requires the --json flag for structural export.");
        }
        return Ok(());
    }

    if profile_mode {
        if let Some(target) = profile_target {
            println!("\n📊 Generating Intelligence Report for: {}", target);

            match profile::build_profile(&pool, &target).await {
                Ok(cap_profile) => {
                    profile::print_profile(&cap_profile);
                }
                Err(e) => {
                    if e.to_string().contains("No capabilities found") {
                        println!("📥 User '{}' not found in database. Starting live ingestion...", target);
                        
                        // 1. Ingest repositories and commits
                        if let Err(ingest_err) = pipeline::ingest_user(&github_client, &pool, &target).await {
                            eprintln!("❌ Failed to ingest user data: {}", ingest_err);
                            return Ok(());
                        }

                        // 2. Extract capabilities
                        println!("🧠 Extracting capabilities for: {}", target);
                        match extraction::extract_user_capabilities(&pool, &target).await {
                            Ok(mut capabilities) => {
                                // 3. Calibrate scores
                                for cap in &mut capabilities {
                                    cap.normalized_score = calibration::calibrate_score(
                                        cap.confidence,
                                        cap.capability_type.as_str(),
                                        &search_config
                                    );
                                    cap.tier = extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
                                }

                                // 4. Store capabilities and vector
                                if let Err(store_err) = extraction::storage::store_capabilities(&pool, &capabilities).await {
                                    eprintln!("❌ Failed to store capabilities: {}", store_err);
                                    return Ok(());
                                }
                                
                                let vector = similarity::CapabilityVector::from_capabilities(&target, &capabilities, &registry);
                                if let Err(vec_err) = similarity::store_vector(&pool, &vector, &registry).await {
                                    eprintln!("❌ Failed to store vector: {}", vec_err);
                                }

                                // 5. Retry generating the profile
                                println!("\n🎉 Live ingestion complete! Generating report...\n");
                                match profile::build_profile(&pool, &target).await {
                                    Ok(fresh_profile) => profile::print_profile(&fresh_profile),
                                    Err(fresh_err) => eprintln!("❌ Failed to generate profile even after ingestion: {}", fresh_err),
                                }
                            },
                            Err(ext_err) => {
                                eprintln!("❌ Failed to extract capabilities: {}", ext_err);
                            }
                        }

                    } else {
                        eprintln!("❌ Failed to generate profile: {}", e);
                    }
                }
            }
        }
        return Ok(());
    }

    if similar_mode {
        if let Some(target) = similar_target {
            println!("\n🔍 Finding users similar to: {}", target);

            // Allow configurable limit, defaulting to 5
            let args: Vec<String> = std::env::args().collect();
            let limit = args
                .windows(2)
                .find(|w| w[0] == "--limit")
                .and_then(|w| w[1].parse::<usize>().ok())
                .unwrap_or(5);

            match similarity::find_similar_entities(&pool, &target, limit, &registry).await {
                Ok(results) => {
                    if json_mode {
                        let payload = serde_json::json!({
                            "target": target,
                            "similar_users": results.iter().map(|res| {
                                serde_json::json!({
                                    "candidate": res.entity_id,
                                    "overlap_score": res.similarity_score,
                                    "shared_capabilities": res.shared_capabilities.iter().map(|s| &s.name).collect::<Vec<_>>()
                                })
                            }).collect::<Vec<_>>()
                        });
                        println!("{}", serde_json::to_string(&payload).unwrap());
                    } else {
                        println!("\n====================");
                        println!("Similar to {}", target);
                        println!("====================");
    
                        if results.is_empty() {
                            println!("No similar users found (or user not indexed).");
                        } else {
                            for (i, res) in results.iter().enumerate() {
                                println!(
                                    "\n{}. {} ({:.3})",
                                    i + 1,
                                    res.entity_id,
                                    res.similarity_score
                                );
    
                                if !res.shared_capabilities.is_empty() {
                                    println!("   Shared Strengths:");
                                    for shared in &res.shared_capabilities {
                                        println!("     • {}", shared.name);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not found in capability_vectors") {
                        println!("📥 User '{}' not found in database. Starting live ingestion...", target);
                        
                        // 1. Ingest repositories and commits
                        if let Err(ingest_err) = pipeline::ingest_user(&github_client, &pool, &target).await {
                            eprintln!("❌ Failed to ingest user data: {}", ingest_err);
                            return Ok(());
                        }

                        // 2. Extract capabilities
                        println!("🧠 Extracting capabilities for: {}", target);
                        match extraction::extract_user_capabilities(&pool, &target).await {
                            Ok(mut capabilities) => {
                                // 3. Calibrate scores
                                for cap in &mut capabilities {
                                    cap.normalized_score = calibration::calibrate_score(
                                        cap.confidence,
                                        cap.capability_type.as_str(),
                                        &search_config
                                    );
                                    cap.tier = extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
                                }

                                // 4. Store capabilities and vector
                                if let Err(store_err) = extraction::storage::store_capabilities(&pool, &capabilities).await {
                                    eprintln!("❌ Failed to store capabilities: {}", store_err);
                                    return Ok(());
                                }
                                
                                let vector = similarity::CapabilityVector::from_capabilities(&target, &capabilities, &registry);
                                if let Err(vec_err) = similarity::store_vector(&pool, &vector, &registry).await {
                                    eprintln!("❌ Failed to store vector: {}", vec_err);
                                }

                                // 5. Retry generating the similarity search
                                println!("\n🎉 Live ingestion complete! Generating similarity report...\n");
                                match similarity::find_similar_entities(&pool, &target, limit, &registry).await {
                                    Ok(fresh_results) => {
                                        if json_mode {
                                            let payload = serde_json::json!({
                                                "target": target,
                                                "similar_users": fresh_results.iter().map(|res| {
                                                    serde_json::json!({
                                                        "candidate": res.entity_id,
                                                        "overlap_score": res.similarity_score,
                                                        "shared_capabilities": res.shared_capabilities.iter().map(|s| &s.name).collect::<Vec<_>>()
                                                    })
                                                }).collect::<Vec<_>>()
                                            });
                                            println!("{}", serde_json::to_string(&payload).unwrap());
                                        } else {
                                            println!("\n====================");
                                            println!("Similar to {}", target);
                                            println!("====================");
                        
                                            if fresh_results.is_empty() {
                                                println!("No similar users found (or user not indexed).");
                                            } else {
                                                for (i, res) in fresh_results.iter().enumerate() {
                                                    println!(
                                                        "\n{}. {} ({:.3})",
                                                        i + 1,
                                                        res.entity_id,
                                                        res.similarity_score
                                                    );
                        
                                                    if !res.shared_capabilities.is_empty() {
                                                        println!("   Shared Strengths:");
                                                        for shared in &res.shared_capabilities {
                                                            println!("     • {}", shared.name);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    Err(fresh_err) => eprintln!("❌ Failed to generate similarity search even after ingestion: {}", fresh_err),
                                }
                            },
                            Err(ext_err) => {
                                eprintln!("❌ Failed to extract capabilities: {}", ext_err);
                            }
                        }
                    } else {
                        eprintln!("❌ Failed to find similar users: {}", e);
                    }
                }
            }
        }
        return Ok(());
    }

    // Single-user mode (existing logic)
    println!("");
    println!("📥 Starting data ingestion...");
    pipeline::ingest_user(&github_client, &pool, &target_username).await?;

    println!("✅ Ingestion complete!");
    println!("");

    // Run capability extraction
    println!("🧠 Starting capability extraction...");
    let mut capabilities = extraction::extract_user_capabilities(&pool, &target_username).await?;

    // Apply score calibration
    for cap in &mut capabilities {
        cap.normalized_score = calibration::calibrate_score(
            cap.confidence,
            cap.capability_type.as_str(),
            &search_config,
        );
        cap.tier = extraction::config::CapabilityTier::from_confidence(cap.normalized_score);
    }

    // Store capabilities
    println!("💾 Storing capabilities...");
    for capability in &capabilities {
        extraction::storage::insert_capability(&pool, capability).await?;
    }

    println!("✅ Capability extraction complete!");
    println!("");

    // Display results with detailed breakdown
    println!("📊 Capability Analysis for: {}", target_username);
    println!("=======================================");

    if capabilities.is_empty() {
        println!("⚠  No capabilities detected");
    } else {
        for cap in &capabilities {
            println!("");
            println!(
                "  {} {} Capability: {} [{}]",
                cap.tier.emoji(),
                get_capability_emoji(&cap.capability_type),
                cap.capability_type.as_str(),
                cap.tier.as_str()
            );
            println!("    Confidence: {:.2}", cap.confidence);
            if search_config.ranking.calibration.enabled {
                println!("    Calibrated: {:.2}", cap.normalized_score);
            }
            println!("");
            println!("    Breakdown:");
            println!(
                "      Keyword Score:    {:.2} (60%)",
                cap.signal_breakdown.keyword_score
            );
            println!(
                "      Repo Boost:       {:.2} (20%)",
                cap.signal_breakdown.filename_score
            );
            println!(
                "      Language Boost:   {:.2} (10%)",
                cap.signal_breakdown.language_score
            );
            println!(
                "      Structural Score: {:.2} (10%)",
                cap.signal_breakdown.structure_score
            );
            println!("");
            println!("    Advanced Scoring:");
            println!(
                "      Raw Score:        {:.2}",
                cap.signal_breakdown.raw_score
            );
            println!(
                "      Sigmoid Boost:    +{:.2}",
                cap.confidence
                    - cap.signal_breakdown.raw_score * cap.signal_breakdown.time_decay_factor
            );
            println!(
                "      Time Decay:       {:.2}x",
                cap.signal_breakdown.time_decay_factor
            );
            if cap.signal_breakdown.correlation_boost > 0.0 {
                println!(
                    "      Correlation:      +{:.2}",
                    cap.signal_breakdown.correlation_boost
                );
            }

            if !cap.evidence_keywords.is_empty() {
                println!("");
                println!("    Evidence Keywords:");
                let keywords_display = cap
                    .evidence_keywords
                    .iter()
                    .take(10)
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("      {}", keywords_display);
            }

            if !cap.evidence_repos.is_empty() {
                println!("");
                println!("    Evidence Repos:");
                for repo in cap.evidence_repos.iter().take(5) {
                    println!("      - {}", repo);
                }
            }
        }
    }

    println!("");

    // Phase 3: Index capabilities for search
    println!("🔍 Building search index...");

    let index = search::CapabilityIndex::create("./index")?;
    let mut writer = index.get_writer()?;
    search::index_capabilities(&mut writer, &capabilities, &index.schema)?;

    println!("✅ Index built successfully!");
    println!("");

    // Test search queries
    println!("🔎 Testing Search Queries");
    println!("=======================================");

    // Query 1: All STRONG capabilities
    println!("");
    println!("Query 1: All STRONG capabilities");
    let query1 = search::CapabilityQuery {
        tier: Some("STRONG".to_string()),
        ..Default::default()
    };

    let results1 = search::search_capabilities(&index, &query1, &search_config)?;
    display_search_results(&results1);

    // Query 2: DistributedSystems capabilities
    println!("");
    println!("Query 2: DistributedSystems capabilities");
    let query2 = search::CapabilityQuery {
        capability_type: Some("DistributedSystems".to_string()),
        ..Default::default()
    };

    let results2 = search::search_capabilities(&index, &query2, &search_config)?;
    display_search_results(&results2);

    // Query 3: Keyword search for "raft"
    println!("");
    println!("Query 3: Keyword search 'raft'");
    let query3 = search::CapabilityQuery {
        keywords: Some("raft".to_string()),
        ..Default::default()
    };

    let results3 = search::search_capabilities(&index, &query3, &search_config)?;
    display_search_results(&results3);

    // Query 4: High confidence (> 0.6)
    println!("");
    println!("Query 4: High confidence (> 0.6)");
    let query4 = search::CapabilityQuery {
        min_confidence: Some(0.6),
        ..Default::default()
    };

    let results4 = search::search_capabilities(&index, &query4, &search_config)?;
    display_search_results(&results4);

    Ok(())
}

fn display_search_results(results: &[search::SearchResult]) {
    if results.is_empty() {
        println!("  No results found");
        return;
    }

    for (i, result) in results.iter().enumerate() {
        println!(
            "  {}. {} - {} [{}]",
            i + 1,
            result.entity_id,
            result.capability_type,
            result.tier
        );
        println!(
            "     Confidence: {:.2} | Final Score: {:.2}",
            result.confidence, result.final_score
        );

        if !result.matched_on.keywords.is_empty() {
            println!(
                "     Matched Keywords: {}",
                result.matched_on.keywords.join(", ")
            );
        }

        if !result.matched_on.repos.is_empty() {
            println!("     Matched Repos: {}", result.matched_on.repos.join(", "));
        }
    }
}

fn get_capability_emoji(cap_type: &extraction::CapabilityType) -> &'static str {
    match cap_type.as_str() {
        "DistributedAlgorithms" | "DistributedSystems" => "🌐",
        "ConcurrentProgramming" | "ConcurrencyEngineering" => "⚡",
        "PerformanceEngineering" | "PerformanceOptimization" => "🚀",
        "WebBackendAPI" | "APIArchitecture" => "🔌",
        "DatabaseUsage" | "DatabaseInternals" | "DatabaseScaling" => "🗄️",
        "MachineLearning" => "🧠",
        "FrontendEngineering" => "🎨",
        "CloudInfrastructure" => "☁️",
        "DevOpsAutomation" => "⚙️",
        "SecurityEngineering" => "🔒",
        "CompilersLanguageTooling" => "🔧",
        "NetworkingEngineering" => "🌍",
        "DataEngineering" => "📊",
        "ObservabilityReliability" => "📡",
        "SearchIndexing" => "🔍",
        "RuntimeSystems" => "⚙️",
        "ServiceScalability" => "📈",
        _ => "💡",
    }
}
