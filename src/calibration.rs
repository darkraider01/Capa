use crate::config::SearchConfig;

/// Calibrates a raw score using Z-score normalization based on configuration statistics.
///
/// Z = (raw_score - mean) / std_dev
///
/// The resulting Z-score is then mapped to a 0.0 - 1.0 range.
/// We use a linear mapping where Z = -3.0 maps to 0.0 and Z = +3.0 maps to 1.0,
/// with clamping for values outside this range.
pub fn calibrate_score(raw_score: f32, capability_type: &str, config: &SearchConfig) -> f32 {
    if !config.ranking.calibration.enabled {
        return raw_score;
    }

    if let Some(stats) = config.ranking.calibration.stats.get(capability_type) {
        if stats.std_dev > 0.0 {
            let z_score = (raw_score - stats.mean) / stats.std_dev;

            // Map Z-score [-3.0, 3.0] to [0.0, 1.0]
            let normalized = (z_score + 3.0) / 6.0;

            // Clamp to [0.0, 1.0]
            return normalized.clamp(0.0, 1.0);
        }
    }

    // Default to raw score if no stats available or std_dev is 0
    raw_score
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
}
