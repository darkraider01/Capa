use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

/// A single capability definition loaded from capabilities.toml
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityDefinition {
    pub id: String,
    pub display_name: String,
    pub meta_category: String,
    pub keywords: KeywordTiers,
    pub core_dependencies: Vec<String>,
    pub ecosystem_dependencies: Vec<String>,
    pub file_tokens: Vec<String>,
    pub folders: Vec<String>,
    pub language_hints: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeywordTiers {
    pub strict: Vec<String>,
    pub soft: Vec<String>,
}


#[derive(Debug, Deserialize)]
struct TomlRoot {
    negative_signals: NegativeSignals,
    capabilities: Vec<CapabilityDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NegativeSignals {
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CapabilityRegistry {
    /// Ordered list of all capabilities (order is canonical for vector indexing)
    pub capabilities: Vec<CapabilityDefinition>,
    /// Global negative signals to penalize noisy repos
    pub negative_signals: NegativeSignals,
    /// Fast lookup: capability id → index in `capabilities`
    id_to_index: HashMap<String, usize>,
    /// Fast lookup: dep name (lowercase) → list of (capability id, is_core)
    dep_to_caps: HashMap<String, Vec<(String, bool)>>,
    /// Fast lookup: file token (lowercase) → list of capability ids
    token_to_caps: HashMap<String, Vec<String>>,
    /// Fast lookup: folder name (lowercase) → list of capability ids
    folder_to_caps: HashMap<String, Vec<String>>,
}

impl CapabilityRegistry {
    /// Load registry from the capabilities.toml file.
    pub fn load() -> Result<Self> {
        let toml_path = "config/capabilities.toml";
        let content = std::fs::read_to_string(toml_path)
            .with_context(|| format!("Failed to read {}", toml_path))?;
        let root: TomlRoot =
            toml::from_str(&content).with_context(|| "Failed to parse capabilities.toml")?;

        let capabilities = root.capabilities;
        let negative_signals = root.negative_signals;

        let id_to_index: HashMap<String, usize> = capabilities
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i))
            .collect();

        // Build dep → caps lookup (now tracking if it's a core or ecosystem dep)
        let mut dep_to_caps: HashMap<String, Vec<(String, bool)>> = HashMap::new();
        for cap in &capabilities {
            for dep in &cap.core_dependencies {
                dep_to_caps
                    .entry(dep.to_lowercase())
                    .or_default()
                    .push((cap.id.clone(), true));
            }
            for dep in &cap.ecosystem_dependencies {
                dep_to_caps
                    .entry(dep.to_lowercase())
                    .or_default()
                    .push((cap.id.clone(), false));
            }
        }

        // Build token → caps lookup
        let mut token_to_caps: HashMap<String, Vec<String>> = HashMap::new();
        for cap in &capabilities {
            for token in &cap.file_tokens {
                token_to_caps
                    .entry(token.to_lowercase())
                    .or_default()
                    .push(cap.id.clone());
            }
        }

        // Build folder → caps lookup
        let mut folder_to_caps: HashMap<String, Vec<String>> = HashMap::new();
        for cap in &capabilities {
            for folder in &cap.folders {
                folder_to_caps
                    .entry(folder.to_lowercase())
                    .or_default()
                    .push(cap.id.clone());
            }
        }

        Ok(Self {
            capabilities,
            negative_signals,
            id_to_index,
            dep_to_caps,
            token_to_caps,
            folder_to_caps,
        })
    }

    /// Look up a capability definition by id.
    pub fn get(&self, id: &str) -> Option<&CapabilityDefinition> {
        self.id_to_index.get(id).map(|&i| &self.capabilities[i])
    }

    /// Get the canonical vector index for a capability id.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.id_to_index.get(id).copied()
    }

    /// Total number of capabilities (= vector dimension).
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Capability ids and their `is_core` status that a given dependency name signals.
    /// Input is normalized to lowercase before lookup.
    pub fn caps_for_dep(&self, dep: &str) -> &[(String, bool)] {
        self.dep_to_caps
            .get(&dep.to_lowercase())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Capability ids that a filename token signals.
    pub fn caps_for_token(&self, token: &str) -> &[String] {
        self.token_to_caps
            .get(&token.to_lowercase())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Capability ids that a top-level folder signals.
    pub fn caps_for_folder(&self, folder: &str) -> &[String] {
        self.folder_to_caps
            .get(&folder.to_lowercase())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Keyword tiers for a capability (used by heuristics).
    pub fn keywords(&self, id: &str) -> Option<&KeywordTiers> {
        self.get(id).map(|c| &c.keywords)
    }

    /// All capability ids in canonical order.
    pub fn ids(&self) -> Vec<&str> {
        self.capabilities.iter().map(|c| c.id.as_str()).collect()
    }

    /// Display name for a capability id.
    pub fn display_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id)
            .map(|c| c.display_name.as_str())
            .unwrap_or(id)
    }

    /// Meta-category for a capability id.
    pub fn meta_category<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id)
            .map(|c| c.meta_category.as_str())
            .unwrap_or("Unknown")
    }

    /// Build a 5-dimensional meta-capability vector from an 18-dim score map.
    /// meta[0] = Systems, [1] = Infrastructure, [2] = Data, [3] = Application, [4] = Research
    pub fn build_meta_vector(&self, scores: &HashMap<String, f32>) -> Vec<f32> {
        let meta_groups: [(&str, &[&str]); 5] = [
            (
                "Systems",
                &[
                    "DistributedAlgorithms",
                    "ConcurrentProgramming",
                    "RuntimeSystems",
                    "NetworkingEngineering",
                    "PerformanceEngineering",
                ],
            ),
            (
                "Infrastructure",
                &[
                    "CloudInfrastructure",
                    "DevOpsAutomation",
                    "ServiceScalability",
                    "ObservabilityReliability",
                    "WebBackendAPI",
                ],
            ),
            (
                "Data",
                &[
                    "DatabaseInternals",
                    "DatabaseUsage",
                    "DataEngineering",
                    "SearchIndexing",
                ],
            ),
            (
                "Application",
                &[
                    "FrontendEngineering",
                    "WebBackendAPI",
                    "SecurityEngineering",
                ],
            ),
            (
                "Research",
                &[
                    "MachineLearning",
                    "CompilersLanguageTooling",
                    "PerformanceEngineering",
                ],
            ),
        ];

        meta_groups
            .iter()
            .map(|(_, cap_ids)| {
                let sum: f32 = cap_ids
                    .iter()
                    .map(|id| scores.get(*id).copied().unwrap_or(0.0))
                    .sum();
                // Average across group members, clamped to [0,1]
                (sum / cap_ids.len() as f32).min(1.0)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_loads() {
        let registry = CapabilityRegistry::load().expect("should load capabilities.toml");
        assert_eq!(registry.len(), 18, "Should have 18 capabilities");
    }

    #[test]
    fn test_dep_lookup() {
        let registry = CapabilityRegistry::load().unwrap();
        let caps = registry.caps_for_dep("tokio");
        assert!(
            caps.contains(&("ConcurrentProgramming".to_string(), true)),
            "tokio should map to ConcurrentProgramming (core)"
        );
    }

    #[test]
    fn test_token_lookup() {
        let registry = CapabilityRegistry::load().unwrap();
        let caps = registry.caps_for_token("lexer");
        assert!(
            caps.contains(&"CompilersLanguageTooling".to_string()),
            "lexer token should map to CompilersLanguageTooling"
        );
    }

    #[test]
    fn test_meta_vector_length() {
        let registry = CapabilityRegistry::load().unwrap();
        let mut scores = std::collections::HashMap::new();
        scores.insert("MachineLearning".to_string(), 0.8_f32);
        let meta = registry.build_meta_vector(&scores);
        assert_eq!(meta.len(), 5);
    }
}
