use anyhow::Result;
use sqlx::PgPool;
use crate::config::SearchConfig;

#[derive(Debug)]
pub struct Archetype {
    pub username: String,
    pub expected_primary: Vec<String>,
    pub expected_secondary: Vec<String>,
}

pub async fn evaluate_archetypes(pool: &PgPool, _config: &SearchConfig) -> Result<()> {
    let archetypes = vec![
        Archetype {
            username: "dtolnay".to_string(),
            expected_primary: vec!["CompilersLanguageTooling".to_string()],
            expected_secondary: vec!["PerformanceEngineering".to_string()],
        },
        Archetype {
            username: "gaearon".to_string(),
            expected_primary: vec!["FrontendEngineering".to_string(), "WebBackendAPI".to_string()],
            expected_secondary: vec![],
        },
        Archetype {
            username: "burntsushi".to_string(),
            expected_primary: vec!["PerformanceEngineering".to_string(), "SearchIndexing".to_string()],
            expected_secondary: vec!["ConcurrentProgramming".to_string()],
        },
        Archetype {
            username: "karpathy".to_string(),
            expected_primary: vec!["MachineLearning".to_string()],
            expected_secondary: vec!["PerformanceEngineering".to_string()],
        }
    ];

    println!("\n🎭 Evaluating User Archetypes:\n");

    let mut total = 0;
    let mut passed = 0;

    for arch in archetypes {
        total += 1;
        println!("User: {}", arch.username);
        
        let mut expected = String::new();
        if !arch.expected_primary.is_empty() {
            expected.push_str(&arch.expected_primary.join(", "));
        }
        
        println!("Expected: {}", expected);

        // Fetch user capabilities
        let query = "
            SELECT capability_type, confidence, tier 
            FROM capabilities 
            WHERE user_login = $1 
            ORDER BY confidence DESC
            LIMIT 3;
        ";

        #[derive(sqlx::FromRow)]
        struct CapRow {
            capability_type: String,
            // PostgreSQL floats from the DB should map to rust f32/f64 depending on table def
            // table defined as FLOAT which is often f64 in sqlx postgres, but let's use f64 just in case
            confidence: f64,
            tier: String,
        }

        let extracted: Vec<CapRow> = sqlx::query_as(query)
            .bind(&arch.username)
            .fetch_all(pool)
            .await?;

        if extracted.is_empty() {
            println!("Actual: [No data in DB]");
            println!("STATUS: ERROR (Missing Data)\n");
            continue;
        }

        let actual_top_3: Vec<String> = extracted.into_iter().map(|r| r.capability_type).collect();
        println!("Actual: {}", actual_top_3.join(", "));

        // Check if ANY expected primary cap is in the actual top 3
        let mut match_found = false;
        for expected_cap in &arch.expected_primary {
            if actual_top_3.contains(expected_cap) {
                match_found = true;
                break;
            }
        }

        if match_found {
            println!("STATUS: PASS\n");
            passed += 1;
        } else {
            println!("STATUS: FAIL\n");
        }
    }

    println!("===================================");
    println!("Archetype Evaluation Score: {}/{}", passed, total);
    println!("===================================\n");

    Ok(())
}
