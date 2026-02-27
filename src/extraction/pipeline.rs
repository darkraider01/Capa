use anyhow::Result;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

use crate::extraction::{
    config::{ScoringWeights, SignalConfig},
    heuristics,
    models::{ExtractedCapability, Signal, SignalSource},
    scoring::{self, RepoData, RepoSignals},
};
use crate::github_client::GithubClient;
use crate::signals::{
    activity_analyzer,
    capability_registry::CapabilityRegistry,
    dependency_parser,
    file_scanner,
    language_signal,
    project_structure,
};

// Supported manifest filenames (checked at top of repo tree)
const MANIFEST_FILES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "composer.json",
    "Gemfile",
];

/// Full multi-channel extraction for a user.
/// Requires a live GitHub client for tree/content/language fetches.
pub async fn extract_user_capabilities_full(
    pool: &PgPool,
    github_client: &GithubClient,
    username: &str,
    registry: &CapabilityRegistry,
) -> Result<Vec<ExtractedCapability>> {
    println!("🧠 [Multi-Signal] Extracting for: {}", username);

    let signal_config = SignalConfig::default();
    let weights = ScoringWeights::default();

    let repos = fetch_user_repos(pool, username).await?;
    println!("  📊 {} repositories", repos.len());

    let total_user_commits: u64 = repos.iter().map(|r| r.commit_count).sum();

    // Fetch dep IDF frequencies from the database (across all stored repos)
    let dep_frequencies = fetch_dep_frequencies(pool).await.unwrap_or_default();
    let total_repos_in_db = fetch_total_repo_count(pool).await.unwrap_or(1).max(1);

    let mut all_repo_signals: Vec<RepoSignals> = Vec::new();

    for repo in &repos {
        let repo_age_years = repo_age_years(repo.pushed_at.as_deref());
        let age_decay = (-weights.age_decay_lambda * repo_age_years).exp();

        // ── 1. Keyword channel ─────────────────────────────────────────────
        let mut keyword_signals: Vec<Signal> = Vec::new();
        let combo_text = format!("{} {}", repo.name, repo.description.as_deref().unwrap_or(""));
        keyword_signals.extend(heuristics::detect_all_capabilities(
            &combo_text,
            SignalSource::RepoName(repo.name.clone()),
            &signal_config,
            registry,
        ));

        let commits = fetch_repo_commits(pool, repo.id).await?;
        for commit in &commits {
            keyword_signals.extend(heuristics::detect_all_capabilities(
                &commit.message,
                SignalSource::CommitMessage(repo.name.clone(), commit.sha.clone()),
                &signal_config,
                registry,
            ));
        }

        // ── 2-5. Tree-based channels (dep, filename, structure, language) ──
        // Fetch tree with depth ≤ 2 (max 1 API call per repo)
        let (dep_scores, filename_scores, structure_scores) = match github_client
            .fetch_repo_tree(username, &repo.name)
            .await
        {
            Ok(tree) => {
                // 2. Dependency channel
                let manifest_paths: Vec<String> = tree
                    .iter()
                    .filter(|p| {
                        let basename = p.split('/').last().unwrap_or(p);
                        MANIFEST_FILES.iter().any(|m| {
                            m.eq_ignore_ascii_case(basename)
                                || p.to_lowercase().ends_with(".csproj")
                        })
                    })
                    .take(3) // max 3 manifests per repo
                    .cloned()
                    .collect();

                let mut dep_cap_scores: HashMap<String, f32> = HashMap::new();
                for manifest_path in &manifest_paths {
                    if let Ok(Some(content)) = github_client
                        .fetch_file_content(username, &repo.name, manifest_path)
                        .await
                    {
                        let deps =
                            dependency_parser::parse_dependencies(manifest_path, &content);

                        // Store deps in the DB for IDF tracking
                        let _ = store_repo_deps(pool, repo.id, &deps).await;

                        let signals = dependency_parser::dep_signals(
                            &deps,
                            registry,
                            &dep_frequencies,
                            total_repos_in_db,
                        );
                        for (cap_id, score) in signals.0 {
                            let entry = dep_cap_scores.entry(cap_id).or_insert(0.0);
                            *entry = entry.max(score);
                        }
                    }
                }

                // 3. Filename channel
                let filename_signals = file_scanner::scan_filenames(&tree, registry);

                // 4. Structure channel (gated by dep co-occurrence)
                let structure_signals =
                    project_structure::detect_structure(&tree, registry, &dep_cap_scores);

                (dep_cap_scores, filename_signals.0, structure_signals.0)
            }
            Err(e) => {
                eprintln!("  ⚠ Tree fetch failed for {}: {}", repo.name, e);
                (HashMap::new(), HashMap::new(), HashMap::new())
            }
        };

        // ── 5. Language channel ────────────────────────────────────────────
        let language_scores = match github_client
            .fetch_languages(username, &repo.name)
            .await
        {
            Ok(lang_bytes) => language_signal::language_signals(&lang_bytes, registry).0,
            Err(_) => HashMap::new(),
        };

        // ── 6. Activity channel ────────────────────────────────────────────
        let commit_messages: Vec<String> = commits.iter().map(|c| c.message.clone()).collect();
        let activity_scores = activity_analyzer::analyze_activity(&commit_messages).0;

        // ── 7. Negative Signals ─────────────────────────────────────────
        // Evaluate if this repository should be penalized (e.g. leetcode, adventofcode)
        let mut negative_penalty = 0.0;
        let repo_lower = repo.name.to_lowercase();
        let desc_lower = repo.description.as_deref().unwrap_or("").to_lowercase();
        
        for neg_kw in &registry.negative_signals.keywords {
            if repo_lower.contains(neg_kw) || desc_lower.contains(neg_kw) {
                negative_penalty = 0.25; // Linear drag 
                break;
            }
        }
        
        // Also check commits if repo name alone didn't trigger
        if negative_penalty == 0.0 {
            for commit in &commit_messages {
                let msg_lower = commit.to_lowercase();
                for neg_kw in &registry.negative_signals.keywords {
                    if msg_lower.contains(neg_kw) {
                        negative_penalty = 0.25;
                        break;
                    }
                }
                if negative_penalty > 0.0 {
                    break;
                }
            }
        }

        all_repo_signals.push(RepoSignals {
            name: repo.name.clone(),
            language: repo.language.clone(),
            stars: repo.stars,
            keyword_signals,
            dep_scores,
            filename_scores,
            structure_scores,
            language_scores,
            activity_scores,
            negative_signal_penalty: negative_penalty,
            age_decay,
            commit_count: repo.commit_count,
        });
    }

    // Aggregate all channels → final capabilities
    let cap_ids: Vec<&str> = registry.ids();
    let capabilities = scoring::aggregate_all_signals(
        username.to_string(),
        all_repo_signals,
        total_user_commits,
        &weights,
        &cap_ids,
        signal_config.min_confidence,
    );

    println!("  ✓ {} capabilities extracted (multi-signal)", capabilities.len());
    Ok(capabilities)
}

/// Legacy keyword-only extraction (fallback when no GitHub token has tree access)
pub async fn extract_user_capabilities(
    pool: &PgPool,
    username: &str,
) -> Result<Vec<ExtractedCapability>> {
    let registry = CapabilityRegistry::load()?;
    extract_user_capabilities_with_registry(pool, username, &registry).await
}

/// Keyword-only extraction with a pre-loaded registry.
pub async fn extract_user_capabilities_with_registry(
    pool: &PgPool,
    username: &str,
    registry: &CapabilityRegistry,
) -> Result<Vec<ExtractedCapability>> {
    println!("🧠 Extracting capabilities for user: {}", username);

    let signal_config = SignalConfig::default();
    let scoring_weights = ScoringWeights::default();
    let repos = fetch_user_repos(pool, username).await?;
    println!("  📊 Analyzing {} repositories", repos.len());

    let repo_data: Vec<RepoData> = repos
        .iter()
        .map(|r| RepoData {
            name: r.name.clone(),
            language: r.language.clone(),
            stars: r.stars,
        })
        .collect();

    let total_user_commits = repos.iter().map(|r| r.commit_count).sum::<u64>();
    let mut all_signals: Vec<Signal> = Vec::new();

    for repo in &repos {
        let combo = format!("{} {}", repo.name, repo.description.as_deref().unwrap_or(""));
        all_signals.extend(heuristics::detect_all_capabilities(
            &combo,
            SignalSource::RepoName(repo.name.clone()),
            &signal_config,
            registry,
        ));

        let commits = fetch_repo_commits(pool, repo.id).await?;
        for commit in commits {
            all_signals.extend(heuristics::detect_all_capabilities(
                &commit.message,
                SignalSource::CommitMessage(repo.name.clone(), commit.sha.clone()),
                &signal_config,
                registry,
            ));
        }
    }

    let capabilities = scoring::aggregate_signals(
        username.to_string(),
        all_signals,
        &repo_data,
        total_user_commits,
        &signal_config,
        &scoring_weights,
    );

    println!("  ✓ Extracted {} capabilities", capabilities.len());
    Ok(capabilities)
}

// ─── DB Helpers ───────────────────────────────────────────────────────────────

/// Repo age in years from a pushed_at ISO timestamp. Defaults to 2 years if unknown.
fn repo_age_years(pushed_at: Option<&str>) -> f32 {
    let now = chrono::Utc::now();
    pushed_at
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            let diff = now.signed_duration_since(dt.with_timezone(&chrono::Utc));
            (diff.num_days() as f32 / 365.25).max(0.0)
        })
        .unwrap_or(2.0_f32)
}

/// Store dep names in the DB for IDF tracking.
async fn store_repo_deps(pool: &PgPool, repo_id: i64, deps: &[String]) -> Result<()> {
    for dep in deps {
        let _ = sqlx::query(
            "INSERT INTO dependencies (repo_id, name) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(repo_id)
        .bind(dep)
        .execute(pool)
        .await;
    }
    Ok(())
}

/// Fetch dep → count-of-repos-using-it for IDF computation.
async fn fetch_dep_frequencies(pool: &PgPool) -> Result<HashMap<String, u64>> {
    let rows = sqlx::query("SELECT name, COUNT(DISTINCT repo_id) as freq FROM dependencies GROUP BY name")
        .fetch_all(pool)
        .await?;

    let mut map = HashMap::new();
    for row in rows {
        let name: String = row.get("name");
        let freq: i64 = row.get("freq");
        map.insert(name, freq as u64);
    }
    Ok(map)
}

async fn fetch_total_repo_count(pool: &PgPool) -> Result<u64> {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM repositories")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("cnt") as u64)
}

async fn fetch_user_repos(pool: &PgPool, username: &str) -> Result<Vec<RepoInfo>> {
    let rows = sqlx::query(
        r#"
        SELECT r.id, r.name, r.description, r.stars, r.language, r.pushed_at,
               COUNT(c.sha) as commit_count
        FROM repositories r
        LEFT JOIN commits c ON c.repo_id = r.id
        WHERE r.user_login = $1
        GROUP BY r.id, r.name, r.description, r.stars, r.language, r.pushed_at
        "#,
    )
    .bind(username)
    .fetch_all(pool)
    .await?;

    let mut repos = Vec::new();
    for row in rows {
        repos.push(RepoInfo {
            id: row.get("id"),
            name: row.get("name"),
            description: row.get("description"),
            stars: row.get::<i64, _>("stars") as u64,
            language: row.get("language"),
            pushed_at: row.try_get("pushed_at").ok(),
            commit_count: row.get::<i64, _>("commit_count") as u64,
        });
    }
    Ok(repos)
}

async fn fetch_repo_commits(pool: &PgPool, repo_id: i64) -> Result<Vec<CommitInfo>> {
    let rows = sqlx::query("SELECT sha, message FROM commits WHERE repo_id = $1")
        .bind(repo_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| CommitInfo {
            sha: r.get("sha"),
            message: r.get("message"),
        })
        .collect())
}

#[derive(Debug)]
struct RepoInfo {
    id: i64,
    name: String,
    description: Option<String>,
    stars: u64,
    language: Option<String>,
    pushed_at: Option<String>,
    commit_count: u64,
}

#[derive(Debug)]
struct CommitInfo {
    sha: String,
    message: String,
}
