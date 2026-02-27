use serde::{Deserialize, Serialize};

/// Configuration for signal detection and scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    pub strict_weight: f32,
    pub soft_weight: f32,
    pub repo_name_boost: f32,
    pub repo_desc_boost: f32,
    pub commit_boost: f32,
    pub min_confidence: f32,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            strict_weight: 0.50, // High authority for domain-defining keywords (e.g tcp, grpc)
            soft_weight: 0.20,   // Lower authority for ecosystem keywords (e.g jwt, api)
            repo_name_boost: 1.5,
            repo_desc_boost: 1.2,
            commit_boost: 1.0,
            min_confidence: 0.05, // Lower threshold — more signals pass; filtering is done by channel weights
        }
    }
}

/// Channel weights for combined scoring (must sum to 1.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelWeights {
    pub dependency: f32,  // 0.40 — IDF-weighted, highest authority
    pub filename: f32,    // 0.20 — vendor/test filtered
    pub structure: f32,   // 0.15 — composite-gated
    pub keyword: f32,     // 0.15 — medium authority
    pub activity: f32,    // 0.05 — normalized by commit count
    pub language: f32,    // 0.05 — amplify-only
}

impl Default for ChannelWeights {
    fn default() -> Self {
        Self {
            dependency: 0.40,
            filename: 0.20,
            structure: 0.15,
            keyword: 0.15,
            activity: 0.05,
            language: 0.05,
        }
    }
}

/// Scoring weights for the post-signal pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    // Non-linear scaling (sigmoid)
    pub alpha: f32, // Sharpness
    pub beta: f32,  // Threshold

    // Density scaling (legacy keyword path, kept for compatibility)
    pub density_scaling_factor: f32,

    // Repo age decay
    pub age_decay_lambda: f32, // e^(-lambda * repo_age_years); default 0.3

    // Activity normalization
    pub normalization_factor: f32,

    // Max contribution from a single repo (cap)
    pub max_repo_contribution: f32,

    // Cross-capability correlation boost
    pub correlation_boost_factor: f32,

    // Channel weights
    pub channels: ChannelWeights,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            alpha: 5.0,
            beta: 0.35,
            density_scaling_factor: 3.0,
            age_decay_lambda: 0.3,
            normalization_factor: 0.01,
            max_repo_contribution: 0.35,
            correlation_boost_factor: 0.1,
            channels: ChannelWeights::default(),
        }
    }
}

/// Capability strength tier classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityTier {
    Weak,     // 0.0–0.3
    Emerging, // 0.3–0.6
    Strong,   // 0.6–0.8
    Proven,   // 0.8–1.0
}

impl CapabilityTier {
    pub fn from_confidence(confidence: f32) -> Self {
        if confidence >= 0.8 {
            CapabilityTier::Proven
        } else if confidence >= 0.6 {
            CapabilityTier::Strong
        } else if confidence >= 0.3 {
            CapabilityTier::Emerging
        } else {
            CapabilityTier::Weak
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "WEAK" => Some(CapabilityTier::Weak),
            "EMERGING" => Some(CapabilityTier::Emerging),
            "STRONG" => Some(CapabilityTier::Strong),
            "PROVEN" => Some(CapabilityTier::Proven),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityTier::Weak => "WEAK",
            CapabilityTier::Emerging => "EMERGING",
            CapabilityTier::Strong => "STRONG",
            CapabilityTier::Proven => "PROVEN",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            CapabilityTier::Weak => "🔹",
            CapabilityTier::Emerging => "🔸",
            CapabilityTier::Strong => "⭐",
            CapabilityTier::Proven => "🏆",
        }
    }
}

/// Star count boost multipliers (still used in repo scoring)
pub fn get_star_boost(stars: u64) -> f32 {
    if stars > 100 {
        1.1
    } else if stars > 50 {
        1.05
    } else {
        1.0
    }
}
