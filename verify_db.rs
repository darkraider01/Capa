use sqlx::postgres::PgPoolOptions;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    
    let pool = PgPoolOptions::new()
        .connect(&database_url)
        .await?;
        
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM capabilities")
        .fetch_one(&pool)
        .await?;
        
    println!("Total capabilities: {}", count.0);
    
    // Also check for NULLs in the float columns just in case
    let nulls: (i64,) = sqlx::query_as(r#"
        SELECT COUNT(*) FROM capabilities 
        WHERE keyword_score IS NULL 
           OR repo_score IS NULL 
           OR language_score IS NULL 
           OR structural_score IS NULL
           OR raw_score IS NULL
           OR time_decay_factor IS NULL
           OR correlation_boost IS NULL
    "#)
    .fetch_one(&pool)
    .await?;
    
    println!("Rows with NULL scores: {}", nulls.0);
    
    Ok(())
}
