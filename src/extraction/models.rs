use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Dynamic capability type — driven by capabilities.toml, no enum variants needed.
/// Stored as the capability id string (e.g. "MachineLearning", "FrontendEngineering").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CapabilityType(pub String);

impl CapabilityType {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_str(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for CapabilityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Source of a signal (for weighting)
#[derive(Debug, Clone)]
pub enum SignalSource {
    RepoName(String),
    RepoDescription(String),
    CommitMessage(String, String), // (repo_name, sha)
}

/// A single raw signal detected from one source
#[derive(Debug, Clone)]
pub struct Signal {
    pub capability_type: CapabilityType,
    pub score: f32,
    pub keywords: Vec<String>,
    pub source: SignalSource,
    pub tier: SignalTier,
    pub timestamp: i64,
}

/// Tier of the detected signal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalTier {
    Tier1, // High confidence
    Tier2, // Medium confidence
    Tier3, // Low confidence
}

impl SignalTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalTier::Tier1 => "Tier 1",
            SignalTier::Tier2 => "Tier 2",
            SignalTier::Tier3 => "Tier 3",
        }
    }
}

/// Full breakdown of all 6 signal channels + derived scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalBreakdown {
    // Six evidence channels
    pub keyword_score: f32,
    pub dependency_score: f32,
    pub filename_score: f32,
    pub structure_score: f32,
    pub language_score: f32,
    pub activity_score: f32,
    // Derived / post-processing
    pub raw_score: f32,         // Weighted combination before sigmoid
    pub time_decay_factor: f32, // Repo age decay multiplier
    pub correlation_boost: f32, // Cross-capability correlation boost
}

impl SignalBreakdown {
    pub fn zero() -> Self {
        Self {
            keyword_score: 0.0,
            dependency_score: 0.0,
            filename_score: 0.0,
            structure_score: 0.0,
            language_score: 0.0,
            activity_score: 0.0,
            raw_score: 0.0,
            time_decay_factor: 1.0,
            correlation_boost: 0.0,
        }
    }
}

/// Final extracted capability with confidence and evidence
#[derive(Debug, Clone)]
pub struct ExtractedCapability {
    pub id: Uuid,
    pub user_login: String,
    pub capability_type: CapabilityType,
    pub confidence: f32,
    pub normalized_score: f32,
    pub tier: super::config::CapabilityTier,
    pub signal_breakdown: SignalBreakdown,
    pub evidence_keywords: Vec<String>,
    pub evidence_repos: Vec<String>,
    pub evidence_deps: Vec<String>,
    pub timestamp: i64,
}

impl ExtractedCapability {
    pub fn new(
        user_login: String,
        capability_type: CapabilityType,
        confidence: f32,
        tier: super::config::CapabilityTier,
        signal_breakdown: SignalBreakdown,
        evidence_keywords: Vec<String>,
        evidence_repos: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_login,
            capability_type,
            confidence: confidence.clamp(0.0, 1.0),
            normalized_score: 0.0,
            tier,
            signal_breakdown,
            evidence_keywords,
            evidence_repos,
            evidence_deps: Vec::new(),
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
