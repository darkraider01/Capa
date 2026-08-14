use anyhow::Result;
use sqlx::{PgPool, Row};
use crate::github_client::GithubClient;
use std::collections::BTreeMap;
use serde_json::json;

pub async fn explain_user(pool: &PgPool, username: &str, _gh: &GithubClient, json_mode: bool) -> Result<()> {
    if !json_mode {
        println!("\n🔍 Explainability Mode for User: {}", username);
        println!("================================================");
    }

    // Fetch capabilities including evidence_repos
    let query = "
        SELECT 
            capability_type,
            COALESCE(keyword_score, 0.0) as keyword_score,
            COALESCE(repo_score, 0.0) as filename_score,
            COALESCE(structural_score, 0.0) as structure_score,
            COALESCE(language_score, 0.0) as language_score,
            COALESCE(dependency_score, 0.0) as dependency_score,
            COALESCE(activity_score, 0.0) as activity_score,
            COALESCE(raw_score, 0.0) as raw_score,
            confidence as final_score,
            evidence_repos
        FROM capabilities
        WHERE user_login = $1
        ORDER BY confidence DESC
    ";

    let rows = sqlx::query(query)
        .bind(username)
        .fetch_all(pool)
        .await?;

    if rows.is_empty() {
        if json_mode {
            println!("{}", json!({ "error": format!("No capabilities found for user {}", username) }).to_string());
        } else {
            println!("No capabilities found in database for user {}. Try running ingestion first.", username);
        }
        return Ok(());
    }

    // BTreeMap (not HashMap) so the serialized JSON has a deterministic key order —
    // HashMap's iteration order is randomized per-process, which was silently
    // changing the prompt text sent to Gemini (and thus its sha256 cache key) on
    // every `cargo run`, even when the underlying capability scores were identical.
    let mut caps_map: BTreeMap<String, f64> = BTreeMap::new();
    let mut projects_map: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    let evidence_list: Vec<String> = Vec::new();

    for row in &rows {
        let cap_type: String = row.get("capability_type");
        let final_score: f64 = row.get("final_score");
        caps_map.insert(cap_type.clone(), final_score);

        // Parse evidence_repos JSONB array to map capabilities to individual projects
        let repos_json: Option<serde_json::Value> = row.try_get("evidence_repos").ok();
        if let Some(serde_json::Value::Array(repo_arr)) = repos_json {
            for repo_val in repo_arr {
                if let Some(repo_name) = repo_val.as_str() {
                    projects_map
                        .entry(repo_name.to_string())
                        .or_default()
                        .insert(cap_type.clone(), final_score);
                }
            }
        }
    }

    if json_mode {
        // Output strict JSON for Python LLM to ingest with per-project vectors
        let payload = json!({
            "target": username,
            "capabilities": caps_map,
            "projects": projects_map,
            "evidence": evidence_list,
        });
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    for row in rows {
        let cap_type: String = row.get("capability_type");
        let kw: f64 = row.get("keyword_score");
        let file: f64 = row.get("filename_score");
        let struc: f64 = row.get("structure_score");
        let lang: f64 = row.get("language_score");
        let dep: f64 = row.get("dependency_score");
        let act: f64 = row.get("activity_score");
        let raw: f64 = row.get("raw_score");
        let final_score: f64 = row.get("final_score");

        println!("\n{}:", cap_type);
        println!("  dependency_score: {:.2}", dep);
        println!("  filename_score:   {:.2}", file);
        println!("  structure_score:  {:.2}", struc);
        println!("  keyword_score:    {:.2}", kw);
        println!("  activity_score:   {:.2}", act);
        println!("  language_score:   {:.2}", lang);
        println!("  -----------------------");
        println!("  raw_score:        {:.2}", raw);
        println!("  final_score:      {:.2}", final_score);
    }

    Ok(())
}
