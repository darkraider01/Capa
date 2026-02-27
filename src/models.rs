use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GithubUser {
    pub login: String,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub stargazers_count: u64,
    pub language: Option<String>,
    #[serde(default)]
    pub fork: bool,
    pub pushed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub commit: CommitData,
}

#[derive(Debug, Deserialize)]
pub struct CommitData {
    pub message: String,
}
