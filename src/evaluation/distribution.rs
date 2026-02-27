use anyhow::Result;
use sqlx::PgPool;
use crate::signals::capability_registry::CapabilityRegistry;

pub async fn analyze_distribution(pool: &PgPool) -> Result<()> {
    println!("\n📊 Capability Distribution Analysis:\n");

    let total_users_row: (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT user_login) FROM capabilities")
        .fetch_one(pool)
        .await?;
    let total_users = total_users_row.0 as f64;

    if total_users == 0.0 {
        println!("No users found in capabilities table.");
        return Ok(());
    }

    #[derive(sqlx::FromRow)]
    struct CapCount {
        capability_type: String,
        user_count: i64,
    }

    let counts: Vec<CapCount> = sqlx::query_as(
        "
        SELECT capability_type, COUNT(DISTINCT user_login) as user_count 
        FROM capabilities 
        WHERE confidence >= 0.5 
        GROUP BY capability_type
        ORDER BY user_count DESC;
        "
    )
    .fetch_all(pool)
    .await?;

    let registry = CapabilityRegistry::load()?;
    let mut distribution = std::collections::HashMap::new();
    
    for id in registry.ids() {
        distribution.insert(id.to_string(), 0);
    }

    let mut total_strong = 0;
    for row in counts {
        println!("{}: {} users", row.capability_type, row.user_count);
        distribution.insert(row.capability_type, row.user_count);
        total_strong += row.user_count;
    }

    println!("\n🚨 Automated Distribution Warnings:");
    let mut warnings = 0;
    let avg_caps = total_strong as f64 / total_users;

    if avg_caps > 5.0 {
        println!("  [WARNING] Overclassification detected! Users average {:.1} strong capabilities (Limit: 5.0).", avg_caps);
        warnings += 1;
    }

    for id in registry.ids() {
        let count = *distribution.get(id).unwrap_or(&0) as f64;
        
        if count == 0.0 {
            println!("  [WARNING] Dead signal: '{}' has 0 users.", id);
            warnings += 1;
        }

        if (count / total_users) > 0.70 {
            println!("  [WARNING] Overfiring: '{}' is shared by {:.1}% of users (>70%).", id, (count / total_users) * 100.0);
            warnings += 1;
        }
    }

    if warnings == 0 {
        println!("  [OK] No distribution anomalies detected. Matrix looks healthy.\n");
    } else {
        println!("\nTotal warnings: {}\n", warnings);
    }

    Ok(())
}
