use crate::config::SearchConfig;
use serde::{Deserialize, Serialize};

/// Experience tier cohort classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExperienceTier {
    Learning,    // Entry Level (< 100 commits, low stars)
    Developing,  // Mid Level (100–500 commits, moderate repos/stars)
    Practicing,  // Senior Level (500–1500 commits, solid repos/stars)
    Established, // Lead / Principal Level (>= 1500 commits, heavy repos/stars)
}

impl ExperienceTier {
    pub fn derive_from_profile(total_commits: usize, total_repos: usize, total_stars: usize) -> Self {
        if total_commits >= 1500 || total_stars >= 250 || (total_commits >= 800 && total_repos >= 20) {
            ExperienceTier::Established
        } else if total_commits >= 500 || total_stars >= 50 || (total_commits >= 300 && total_repos >= 10) {
            ExperienceTier::Practicing
        } else if total_commits >= 100 || total_stars >= 10 || total_repos >= 3 {
            ExperienceTier::Developing
        } else {
            ExperienceTier::Learning
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExperienceTier::Learning => "Learning",
            ExperienceTier::Developing => "Developing",
            ExperienceTier::Practicing => "Practicing",
            ExperienceTier::Established => "Established",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "LEARNING" => Some(ExperienceTier::Learning),
            "DEVELOPING" => Some(ExperienceTier::Developing),
            "PRACTICING" => Some(ExperienceTier::Practicing),
            "ESTABLISHED" => Some(ExperienceTier::Established),
            _ => None,
        }
    }
}

/// Calibrates a raw score using cohort-relative or global Z-score normalization.
///
/// Z = (raw_score - mean) / std_dev mapped linearly to [0.0, 1.0].
pub fn calibrate_score_for_cohort(
    raw_score: f32,
    capability_type: &str,
    cohort: Option<ExperienceTier>,
    config: &SearchConfig,
) -> f32 {
    if !config.ranking.calibration.enabled {
        return raw_score;
    }

    let min_samples = config.ranking.calibration.min_samples;

    // First try cohort-qualified key if cohort is provided
    if let Some(tier) = cohort {
        let cohort_key = format!("{}::{}", capability_type, tier.as_str());
        if let Some(stats) = config.ranking.calibration.stats.get(&cohort_key) {
            if stats.sample_count >= min_samples && stats.std_dev > 0.0 {
                let z_score = (raw_score - stats.mean) / stats.std_dev;
                let normalized = (z_score + 3.0) / 6.0;
                return normalized.clamp(0.0, 1.0);
            } else {
                #[cfg(debug_assertions)]
                eprintln!(
                    "Cohort stats for '{}' insufficient (sample_count={}, min={}); falling back to global stats",
                    cohort_key,
                    stats.sample_count,
                    min_samples
                );
            }
        }
    }

    // Fall back to global capability stats key
    if let Some(stats) = config.ranking.calibration.stats.get(capability_type) {
        if stats.sample_count >= min_samples && stats.std_dev > 0.0 {
            let z_score = (raw_score - stats.mean) / stats.std_dev;
            let normalized = (z_score + 3.0) / 6.0;
            return normalized.clamp(0.0, 1.0);
        }
    }

    // Default to raw score if no stats available
    raw_score
}

pub fn calibrate_score(raw_score: f32, capability_type: &str, config: &SearchConfig) -> f32 {
    calibrate_score_for_cohort(raw_score, capability_type, None, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CalibratedRankingConfig, CalibrationConfig, HybridWeights, IngestionConfig,
        PerformanceConfig, RankingConfig, SemanticConfig, TypeStats,
    };
    use std::collections::HashMap;

    fn mock_config() -> SearchConfig {
        let mut stats = HashMap::new();
        stats.insert(
            "TestType".to_string(),
            TypeStats {
                mean: 0.5,
                std_dev: 0.1,
                sample_count: 10,
            },
        );

        SearchConfig {
            ranking: RankingConfig {
                confidence_weight: 0.7,
                recency_weight: 0.2,
                keyword_weight: 0.1,
                calibration: CalibrationConfig {
                    enabled: true,
                    min_samples: 10,
                    stats,
                },
                calibrated: CalibratedRankingConfig {
                    confidence_z_weight: 0.6,
                    recency_weight: 0.3,
                    keyword_weight: 0.1,
                },
            },
            semantic: SemanticConfig {
                enabled: false,
                model: String::new(),
                embedding_dim: 0,
                similarity_threshold: 0.0,
                hybrid_weights: HybridWeights {
                    confidence_z_weight: 0.0,
                    semantic_score_weight: 0.0,
                    recency_weight: 0.0,
                },
            },
            performance: PerformanceConfig {
                cache_size: 0,
                thread_pool_size: 0,
                preload_index: false,
            },
            ingestion: IngestionConfig {
                rate_limit_ms: 0,
                batch_size: 0,
                retry_attempts: 0,
                max_users: 0,
                categories: HashMap::new(),
            },
        }
    }

    #[test]
    fn test_calibrate_score_mean() {
        let config = mock_config();
        let score = calibrate_score(0.5, "TestType", &config);
        assert!((score - 0.5).abs() < 1e-6); // Z=0 maps to 0.5
    }

    #[test]
    fn test_calibrate_score_high() {
        let config = mock_config();
        let score = calibrate_score(0.8, "TestType", &config); // Z=3.0
        assert!((score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_calibrate_score_low() {
        let config = mock_config();
        let score = calibrate_score(0.2, "TestType", &config); // Z=-3.0
        assert!((score - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_calibrate_score_clamping() {
        let config = mock_config();
        let score_high = calibrate_score(1.0, "TestType", &config); // Z=5.0
        assert_eq!(score_high, 1.0);
        let score_low = calibrate_score(0.0, "TestType", &config); // Z=-5.0
        assert_eq!(score_low, 0.0);
    }

    #[test]
    fn test_calibrate_score_insufficient_samples() {
        let mut config = mock_config();
        // Lower sample_count below min_samples (5 < 10)
        config
            .ranking
            .calibration
            .stats
            .get_mut("TestType")
            .unwrap()
            .sample_count = 5;

        let score = calibrate_score(0.8, "TestType", &config);
        // Must fall back to raw score (0.8) instead of calibrated Z-score (1.0)
        assert_eq!(score, 0.8);
    }

    #[test]
    fn test_derive_experience_tier_cascade_resolution() {
        // 1. Established checks
        assert_eq!(ExperienceTier::derive_from_profile(1600, 5, 10), ExperienceTier::Established);
        assert_eq!(ExperienceTier::derive_from_profile(100, 2, 300), ExperienceTier::Established);
        assert_eq!(ExperienceTier::derive_from_profile(850, 22, 5), ExperienceTier::Established);

        // 2. Practicing checks (Disagreement case: high commits=700, low stars=5 -> Practicing)
        assert_eq!(ExperienceTier::derive_from_profile(700, 5, 5), ExperienceTier::Practicing);
        assert_eq!(ExperienceTier::derive_from_profile(50, 2, 60), ExperienceTier::Practicing);
        assert_eq!(ExperienceTier::derive_from_profile(350, 12, 5), ExperienceTier::Practicing);

        // 3. Developing checks
        assert_eq!(ExperienceTier::derive_from_profile(150, 2, 5), ExperienceTier::Developing);
        assert_eq!(ExperienceTier::derive_from_profile(50, 4, 2), ExperienceTier::Developing);

        // 4. Learning fallthrough
        assert_eq!(ExperienceTier::derive_from_profile(50, 1, 5), ExperienceTier::Learning);
    }

    #[test]
    fn test_calibrate_score_cohort_relative() {
        let mut config = mock_config();
        // Insert cohort-specific stats for Learning vs Established cohorts
        config.ranking.calibration.stats.insert(
            "TestType::Learning".to_string(),
            TypeStats {
                mean: 0.15,
                std_dev: 0.05,
                sample_count: 10,
            },
        );
        config.ranking.calibration.stats.insert(
            "TestType::Established".to_string(),
            TypeStats {
                mean: 0.35,
                std_dev: 0.10,
                sample_count: 10,
            },
        );

        let raw = 0.20;
        // Learning cohort: Z = (0.20 - 0.15) / 0.05 = +1.0 -> (1.0 + 3.0)/6.0 = 0.6667
        let learning_score = calibrate_score_for_cohort(raw, "TestType", Some(ExperienceTier::Learning), &config);
        assert!((learning_score - (4.0 / 6.0)).abs() < 1e-4);

        // Established cohort: Z = (0.20 - 0.35) / 0.10 = -1.5 -> (-1.5 + 3.0)/6.0 = 0.2500
        let established_score = calibrate_score_for_cohort(raw, "TestType", Some(ExperienceTier::Established), &config);
        assert!((established_score - 0.2500).abs() < 1e-4);
    }

    #[test]
    fn test_calibrate_score_cohort_fallback() {
        let mut config = mock_config();
        // Cohort key "TestType::Developing" has sample_count=3 < min_samples=10
        config.ranking.calibration.stats.insert(
            "TestType::Developing".to_string(),
            TypeStats {
                mean: 0.2,
                std_dev: 0.05,
                sample_count: 3,
            },
        );

        // Should fall back to global "TestType" stats (mean=0.5, std=0.1) -> raw=0.5 yields Z=0 -> 0.5
        let score = calibrate_score_for_cohort(0.5, "TestType", Some(ExperienceTier::Developing), &config);
        assert!((score - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_search_config_cohort_key_toml_roundtrip() {
        let mut config = mock_config();
        config.ranking.calibration.stats.insert(
            "MachineLearning::Developing".to_string(),
            TypeStats {
                mean: 0.25,
                std_dev: 0.08,
                sample_count: 25,
            },
        );

        let toml_str = toml::to_string(&config).expect("SearchConfig with cohort key should serialize to TOML");
        assert!(toml_str.contains("\"MachineLearning::Developing\""));

        let deserialized: SearchConfig = toml::from_str(&toml_str).expect("Serialized TOML should deserialize cleanly");
        let stats = deserialized
            .ranking
            .calibration
            .stats
            .get("MachineLearning::Developing")
            .expect("Cohort key MachineLearning::Developing must exist after round-trip");
        assert_eq!(stats.sample_count, 25);
        assert!((stats.mean - 0.25).abs() < 1e-6);
    }
}

