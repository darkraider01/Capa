use sqlx::postgres::PgPoolOptions;
use anyhow::Result;
use std::collections::HashMap;

#[derive(sqlx::FromRow, Debug)]
struct ScoreRow {
    capability_type: String,
    raw_score: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    
    let pool = PgPoolOptions::new()
        .connect(&database_url)
        .await?;
        
    let rows: Vec<ScoreRow> = sqlx::query_as(r#"
        SELECT capability_type, raw_score 
        FROM capabilities
        WHERE raw_score IS NOT NULL
    "#)
    .fetch_all(&pool)
    .await?;

    let mut type_stats: HashMap<String, Vec<f64>> = HashMap::new();
    
    for row in rows {
        type_stats.entry(row.capability_type)
            .or_default()
            .push(row.raw_score);
    }
    
    println!("Type,Count,Mean,StdDev,Min,Max");
    
    for (cap_type, scores) in type_stats {
        let count = scores.len();
        let sum: f64 = scores.iter().sum();
        let mean = sum / count as f64;
        
        let variance: f64 = scores.iter()
            .map(|val| {
                let diff = mean - *val;
                diff * diff
            })
            .sum::<f64>() / count as f64;
            
        let std_dev = variance.sqrt();
        
        let min = scores.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max = scores.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        
        println!("{},{},{:.4},{:.4},{:.4},{:.4}", 
            cap_type, count, mean, std_dev, min, max);
    }
    
    Ok(())
}
