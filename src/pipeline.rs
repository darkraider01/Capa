use anyhow::Result;
use sqlx::PgPool;

pub async fn ingest_user(
    client: &crate::github_client::GithubClient,
    pool: &PgPool,
    username: &str,
) -> Result<()> {
    // Insert user first
    crate::storage::insert_user(pool, username).await?;
    println!("✓ Inserted user: {}", username);

    // Fetch and insert repositories
    let repos = client.fetch_repos(username).await?;
    println!("✓ Found {} repositories", repos.len());

    for repo in repos {
        if repo.fork {
            println!("  ⏭ Skipping forked repo: {}", repo.name);
            continue;
        }

        crate::storage::insert_repo(pool, &repo, username).await?;
        println!("  ✓ Inserted repo: {}", repo.name);

        // Fetch and insert commits for each repository
        let commits = client
            .fetch_commits(username, &repo.name)
            .await
            .unwrap_or_default();

        println!("    ✓ Found {} commits", commits.len());

        for commit in commits {
            crate::storage::insert_commit(pool, &commit, repo.id).await?;
        }
    }

    println!("✓ Completed ingestion for user: {}", username);
    Ok(())
}
