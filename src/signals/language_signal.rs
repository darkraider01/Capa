use crate::signals::capability_registry::CapabilityRegistry;
use std::collections::HashMap;

/// Language-based capability score per repo.
/// IMPORTANT: these are amplifiers only — they NEVER create new capabilities on their own.
/// See `amplify_with_language()` for correct usage.
#[derive(Debug, Default)]
pub struct LanguageScores(pub HashMap<String, f32>);

/// Compute language signals from GitHub's language breakdown (bytes per language).
///
/// Formula: `capability_hint_weight × min(language_fraction, 0.6)`
///
/// The 0.6 cap prevents a single-language repo from dominating.
/// The result is used ONLY as a multiplier on existing evidence.
pub fn language_signals(
    language_bytes: &HashMap<String, u64>,
    registry: &CapabilityRegistry,
) -> LanguageScores {
    if language_bytes.is_empty() {
        return LanguageScores::default();
    }

    let total_bytes: u64 = language_bytes.values().sum();
    if total_bytes == 0 {
        return LanguageScores::default();
    }

    let mut scores: HashMap<String, f32> = HashMap::new();

    for (lang, &bytes) in language_bytes {
        let fraction = bytes as f32 / total_bytes as f32;
        let capped_fraction = fraction.min(0.6);

        // Look up which capabilities this language hints at
        let hints = get_language_hints(lang);
        for (cap_id, hint_weight) in hints {
            // Check registry actually knows about this capability
            if registry.index_of(&cap_id).is_some() {
                let signal = hint_weight * capped_fraction;
                let entry = scores.entry(cap_id).or_insert(0.0);
                *entry = (*entry + signal).min(1.0);
            }
        }
    }

    LanguageScores(scores)
}

/// Apply language scores as amplifiers on an existing score map.
///
/// **Amplify-only rule:** language score is multiplied against the existing base score.
/// If base score is 0, language cannot create a new signal.
///
/// Returns updated score map.
pub fn amplify_with_language(
    base_scores: &HashMap<String, f32>,
    lang_scores: &LanguageScores,
    language_weight: f32, // from ChannelWeights.language (e.g. 0.05)
) -> HashMap<String, f32> {
    let mut result = base_scores.clone();

    for (cap_id, &lang_score) in &lang_scores.0 {
        if let Some(base) = result.get(cap_id).copied() {
            if base > 0.0 {
                // Amplify: add language boost proportional to existing evidence
                let boost = base * lang_score * language_weight;
                let entry = result.entry(cap_id.clone()).or_insert(0.0);
                *entry = (*entry + boost).min(1.0);
            }
            // If base == 0.0 → no amplification (amplify-only rule)
        }
    }

    result
}

/// Hand-crafted language → (capability_id, weight) hints.
/// Weights kept low (0.1–0.3) since language is a very weak signal.
/// Multi-domain languages get multiple low-weight entries.
fn get_language_hints(lang: &str) -> Vec<(String, f32)> {
    match lang.to_lowercase().as_str() {
        "rust" => vec![
            ("ConcurrentProgramming".into(), 0.25),
            ("RuntimeSystems".into(), 0.20),
            ("PerformanceEngineering".into(), 0.20),
            ("SystemsProgramming".into(), 0.20),
            ("CompilersLanguageTooling".into(), 0.15),
        ],
        "go" => vec![
            ("NetworkingEngineering".into(), 0.20),
            ("CloudInfrastructure".into(), 0.20),
            ("WebBackendAPI".into(), 0.15),
            ("ConcurrentProgramming".into(), 0.15),
        ],
        "python" => vec![
            // Python is highly multi-domain — all weights low
            ("MachineLearning".into(), 0.15),
            ("DataEngineering".into(), 0.15),
            ("WebBackendAPI".into(), 0.10),
        ],
        "typescript" | "javascript" => vec![
            ("FrontendEngineering".into(), 0.25),
            ("WebBackendAPI".into(), 0.15),
        ],
        "java" | "kotlin" => vec![
            ("WebBackendAPI".into(), 0.15),
            ("ServiceScalability".into(), 0.10),
            ("DatabaseUsage".into(), 0.10),
        ],
        "c" | "c++" => vec![
            ("RuntimeSystems".into(), 0.25),
            ("PerformanceEngineering".into(), 0.20),
            ("NetworkingEngineering".into(), 0.10),
        ],
        "haskell" | "ocaml" | "ml" => vec![
            ("CompilersLanguageTooling".into(), 0.30),
            ("RuntimeSystems".into(), 0.15),
        ],
        "shell" | "bash" => vec![
            ("DevOpsAutomation".into(), 0.20),
            ("CloudInfrastructure".into(), 0.10),
        ],
        "hcl" | "terraform" => vec![
            ("CloudInfrastructure".into(), 0.30),
            ("DevOpsAutomation".into(), 0.20),
        ],
        "scala" | "elixir" | "erlang" => vec![
            ("ConcurrentProgramming".into(), 0.20),
            ("DataEngineering".into(), 0.15),
        ],
        "css" | "scss" | "html" => vec![("FrontendEngineering".into(), 0.20)],
        "sql" => vec![
            ("DatabaseUsage".into(), 0.25),
            ("DataEngineering".into(), 0.15),
        ],
        "cuda" | "glsl" | "hlsl" => vec![
            ("MachineLearning".into(), 0.20),
            ("PerformanceEngineering".into(), 0.15),
        ],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> CapabilityRegistry {
        CapabilityRegistry::load().unwrap()
    }

    #[test]
    fn test_amplify_only_no_base() {
        // Language should NOT create a signal when base is 0
        let base: HashMap<String, f32> = HashMap::new();
        let mut lang_scores = LanguageScores::default();
        lang_scores
            .0
            .insert("MachineLearning".to_string(), 0.8_f32);
        let result = amplify_with_language(&base, &lang_scores, 0.05);
        assert!(
            !result.contains_key("MachineLearning"),
            "language must not create new signals"
        );
    }

    #[test]
    fn test_amplify_boosts_existing() {
        let mut base: HashMap<String, f32> = HashMap::new();
        base.insert("MachineLearning".to_string(), 0.5_f32);

        let mut lang_scores = LanguageScores::default();
        lang_scores
            .0
            .insert("MachineLearning".to_string(), 0.6_f32);

        let result = amplify_with_language(&base, &lang_scores, 0.05);
        let boosted = result.get("MachineLearning").copied().unwrap_or(0.0);
        assert!(
            boosted > 0.5,
            "language should boost existing ML signal above 0.5"
        );
    }

    #[test]
    fn test_python_signals_low_weight() {
        let reg = registry();
        let mut langs = HashMap::new();
        langs.insert("Python".to_string(), 10000u64);
        let scores = language_signals(&langs, &reg);
        // Python ML weight should be <= 0.15
        let ml = scores.0.get("MachineLearning").copied().unwrap_or(0.0);
        assert!(ml <= 0.15, "Python ML signal should be low weight: {}", ml);
    }
}
