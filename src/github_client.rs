use anyhow::{anyhow, Result};
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;

pub struct GithubClient {
    client: Client,
    token: String,
}

impl GithubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: Client::new(),
            token,
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let res = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "capability-search")
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!("GitHub API returned {}: {}", status, url));
        }

        Ok(res.json::<T>().await?)
    }

    pub async fn fetch_repos(&self, username: &str) -> Result<Vec<crate::models::Repository>> {
        let url = format!(
            "https://api.github.com/users/{}/repos?per_page=100&type=owner&sort=updated",
            username
        );
        self.get(&url).await
    }

    pub async fn fetch_commits(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<crate::models::Commit>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/commits?per_page=100&author={}",
            owner, repo, owner
        );
        self.get(&url).await
    }

    /// Fetch the file tree for a repo (depth ≤ 2 from root).
    /// Uses the recursive Trees API, then filters to paths with ≤ 2 slashes.
    pub async fn fetch_repo_tree(&self, owner: &str, repo: &str) -> Result<Vec<String>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/git/trees/HEAD?recursive=1",
            owner, repo
        );

        #[derive(Deserialize)]
        struct TreeResponse {
            tree: Vec<TreeEntry>,
            #[serde(default)]
            truncated: bool,
        }

        #[derive(Deserialize)]
        struct TreeEntry {
            path: String,
            #[serde(rename = "type")]
            entry_type: String,
        }

        let tree: TreeResponse = self.get(&url).await?;

        // Only keep paths at depth ≤ 2 (≤ 2 slash separators)
        // This drastically reduces the set while capturing manifest files and top-level dirs
        let shallow_paths = tree
            .tree
            .into_iter()
            .filter(|e| {
                let depth = e.path.matches('/').count();
                depth <= 2
            })
            .map(|e| e.path)
            .collect();

        if tree.truncated {
            eprintln!("  ⚠️  Tree truncated for {}/{} — some deep paths omitted", owner, repo);
        }

        Ok(shallow_paths)
    }

    /// Fetch the raw text content of a file in a repo.
    /// Returns None if the file doesn't exist or is too large (>500KB).
    pub async fn fetch_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
    ) -> Result<Option<String>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            owner, repo, path
        );

        #[derive(Deserialize)]
        struct FileContent {
            content: Option<String>,
            encoding: Option<String>,
            size: Option<u64>,
        }

        let file: FileContent = match self.get(&url).await {
            Ok(f) => f,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("404") || msg.contains("Not Found") {
                    return Ok(None);
                }
                return Err(e);
            }
        };

        // Skip files > 500KB
        if file.size.unwrap_or(0) > 512_000 {
            return Ok(None);
        }

        if file.encoding.as_deref() == Some("base64") {
            if let Some(encoded) = file.content {
                // GitHub adds newlines to base64 content
                let clean = encoded.replace('\n', "").replace('\r', "");
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(clean.as_bytes())
                    .map_err(|e| anyhow!("base64 decode error: {}", e))?;
                let text = String::from_utf8_lossy(&decoded).to_string();
                return Ok(Some(text));
            }
        }

        Ok(None)
    }

    /// Fetch language breakdown (bytes per language) for a repository.
    pub async fn fetch_languages(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<HashMap<String, u64>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/languages",
            owner, repo
        );
        self.get(&url).await
    }
}
