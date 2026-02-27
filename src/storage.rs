use anyhow::Result;
use sqlx::PgPool;

pub async fn insert_repo(
    pool: &PgPool,
    repo: &crate::models::Repository,
    user: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO repositories (id, user_login, name, description, stars, language, pushed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (id) DO UPDATE SET
            pushed_at = EXCLUDED.pushed_at,
            stars = EXCLUDED.stars
        "#,
    )
    .bind(repo.id as i64)
    .bind(user)
    .bind(&repo.name)
    .bind(&repo.description)
    .bind(repo.stargazers_count as i64)
    .bind(&repo.language)
    .bind(&repo.pushed_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_commit(
    pool: &PgPool,
    commit: &crate::models::Commit,
    repo_id: u64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO commits (sha, repo_id, message)
        VALUES ($1, $2, $3)
        ON CONFLICT (sha) DO NOTHING
        "#,
    )
    .bind(&commit.sha)
    .bind(repo_id as i64)
    .bind(&commit.commit.message)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_user(pool: &PgPool, username: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO users (github_login)
        VALUES ($1)
        ON CONFLICT (github_login) DO NOTHING
        "#,
    )
    .bind(username)
    .execute(pool)
    .await?;

    Ok(())
}
