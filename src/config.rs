use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User list configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserListConfig {
    pub user_lists: Vec<UserCategory>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserCategory {
    pub category: String,
    pub description: String,
    pub users: Vec<String>,
}

impl UserListConfig {
    pub fn load() -> Result<Self> {
        let config_str = std::fs::read_to_string("config/users.toml")?;
        Ok(toml::from_str(&config_str)?)
    }

    pub fn get_all_users(&self) -> Vec<String> {
        self.user_lists
            .iter()
            .flat_map(|cat| cat.users.clone())
            .collect()
    }

    pub fn get_users_by_category(&self, category: &str) -> Vec<String> {
        self.user_lists
            .iter()
            .find(|cat| cat.category == category)
            .map(|cat| cat.users.clone())
            .unwrap_or_default()
    }

    pub fn get_enabled_users(&self, enabled_categories: &HashMap<String, bool>) -> Vec<String> {
        self.user_lists
            .iter()
            .filter(|cat| {
                enabled_categories
                    .get(&cat.category)
                    .copied()
                    .unwrap_or(true)
            })
            .flat_map(|cat| cat.users.clone())
            .collect()
    }
}

/// Search engine configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchConfig {
    pub ranking: RankingConfig,
    pub semantic: SemanticConfig,
    pub performance: PerformanceConfig,
    pub ingestion: IngestionConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RankingConfig {
    pub confidence_weight: f32,
    pub recency_weight: f32,
    pub keyword_weight: f32,
    pub calibration: CalibrationConfig,
    pub calibrated: CalibratedRankingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalibrationConfig {
    pub enabled: bool,
    pub min_samples: usize,
    pub stats: HashMap<String, TypeStats>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TypeStats {
    pub mean: f32,
    pub std_dev: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalibratedRankingConfig {
    pub confidence_z_weight: f32,
    pub recency_weight: f32,
    pub keyword_weight: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SemanticConfig {
    pub enabled: bool,
    pub model: String,
    pub embedding_dim: usize,
    pub similarity_threshold: f32,
    pub hybrid_weights: HybridWeights,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HybridWeights {
    pub confidence_z_weight: f32,
    pub semantic_score_weight: f32,
    pub recency_weight: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PerformanceConfig {
    pub cache_size: usize,
    pub thread_pool_size: usize,
    pub preload_index: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IngestionConfig {
    pub rate_limit_ms: u64,
    pub batch_size: usize,
    pub retry_attempts: usize,
    pub max_users: usize,
    pub categories: HashMap<String, bool>,
}

impl SearchConfig {
    pub fn load() -> Result<Self> {
        let config_str = std::fs::read_to_string("config/search.toml")?;
        Ok(toml::from_str(&config_str)?)
    }

    pub fn get_ranking_weights(&self) -> (f32, f32, f32) {
        if self.ranking.calibration.enabled {
            (
                self.ranking.calibrated.confidence_z_weight,
                self.ranking.calibrated.recency_weight,
                self.ranking.calibrated.keyword_weight,
            )
        } else {
            (
                self.ranking.confidence_weight,
                self.ranking.recency_weight,
                self.ranking.keyword_weight,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_user_config() {
        let config = UserListConfig::load().unwrap();
        assert!(!config.user_lists.is_empty());
    }

    #[test]
    fn test_load_search_config() {
        let config = SearchConfig::load().unwrap();
        assert!(config.ingestion.rate_limit_ms > 0);
    }
}
