use anyhow::Result;
use sqlx::PgPool;

pub struct CapabilityProfile {
    pub entity_id: String,
    pub primary: Vec<CapabilitySummary>,
    pub secondary: Vec<CapabilitySummary>,
    pub emerging: Vec<CapabilitySummary>,
    pub tech_stack: Vec<String>,
}

pub struct CapabilitySummary {
    pub capability_type: String,
    pub normalized_score: f32,
    pub tier: String,
    pub evidence_repos: Vec<String>,
    pub evidence_keywords: Vec<String>,
}

pub async fn build_profile(pool: &PgPool, username: &str) -> Result<CapabilityProfile> {
    let rows = sqlx::query!(
        r#"
        SELECT 
            capability_type,
            normalized_score,
            tier,
            evidence,
            evidence_repos
        FROM capabilities
        WHERE user_login = $1
        ORDER BY normalized_score DESC
        "#,
        username
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Err(anyhow::anyhow!(
            "No capabilities found for user '{}'",
            username
        ));
    }

    let mut summaries = Vec::new();
    for row in rows {
        let evidence_keywords: Vec<String> =
            serde_json::from_value(row.evidence).unwrap_or_default();
        let evidence_repos: Vec<String> = row
            .evidence_repos
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();

        summaries.push(CapabilitySummary {
            capability_type: row.capability_type,
            normalized_score: row.normalized_score as f32,
            tier: row.tier,
            evidence_repos,
            evidence_keywords,
        });
    }

    // Sort to be absolutely sure we rank primarily by normalized_score DESC
    summaries.sort_by(|a, b| {
        b.normalized_score
            .partial_cmp(&a.normalized_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Collect all evidence keywords across all capabilities for tech stack detection
    let all_keywords: Vec<String> = summaries
        .iter()
        .flat_map(|s| s.evidence_keywords.iter().cloned())
        .collect();

    let mut primary = Vec::new();
    let mut secondary = Vec::new();
    let mut emerging = Vec::new();

    // Categorization Logic: Top 2 -> Primary, Next 2 -> Secondary, Remaining -> Emerging
    for (i, sum) in summaries.into_iter().enumerate() {
        if i < 2 {
            primary.push(sum);
        } else if i < 4 {
            secondary.push(sum);
        } else {
            emerging.push(sum);
        }
    }

    Ok(CapabilityProfile {
        entity_id: username.to_string(),
        primary,
        secondary,
        emerging,
        tech_stack: build_tech_stack(pool, username, &all_keywords).await,
    })
}

/// Detect tech stack from repository languages and framework keywords in evidence
async fn build_tech_stack(pool: &PgPool, username: &str, all_keywords: &[String]) -> Vec<String> {
    let mut stack: Vec<String> = Vec::new();

    // 1. Top programming languages from the repositories table
    let lang_rows = sqlx::query(
        r#"SELECT language, COUNT(*) as cnt
           FROM repositories
           WHERE user_login = $1 AND language IS NOT NULL
           GROUP BY language
           ORDER BY cnt DESC
           LIMIT 6"#
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for row in &lang_rows {
        use sqlx::Row;
        if let Ok(lang) = row.try_get::<String, _>("language") {
            if !lang.is_empty() {
                stack.push(lang);
            }
        }
    }

    // 2. Framework detection from aggregated evidence keywords
    let keywords_lower: Vec<String> = all_keywords.iter().map(|k| k.to_lowercase()).collect();
    let frameworks: &[(&str, &[&str])] = &[
        ("React",      &["react", "jsx", "hooks", "redux"]),
        ("Next.js",    &["nextjs", "next.js", "next"]),
        ("Vue",        &["vue", "vuex", "nuxt"]),
        ("Tokio",      &["tokio", "async-std"]),
        ("Actix",      &["actix", "actix-web"]),
        ("Node.js",    &["node", "nodejs", "express", "fastify"]),
        ("Django",     &["django", "drf"]),
        ("FastAPI",    &["fastapi", "pydantic"]),
        ("PyTorch",    &["pytorch", "torch"]),
        ("TensorFlow", &["tensorflow", "keras", "tf"]),
        ("PostgreSQL", &["postgres", "postgresql", "pg"]),
        ("Redis",      &["redis", "cache"]),
        ("Kubernetes", &["kubernetes", "k8s", "kubectl"]),
        ("Docker",     &["docker", "dockerfile", "container"]),
        ("gRPC",       &["grpc", "protobuf", "proto"]),
        ("GraphQL",    &["graphql", "apollo"]),
        ("Wasm",       &["wasm", "webassembly"]),
    ];

    for (framework, triggers) in frameworks {
        if !stack.contains(&framework.to_string())
            && triggers.iter().any(|t| keywords_lower.iter().any(|k| k.contains(t)))
        {
            stack.push(framework.to_string());
        }
    }

    stack
}
