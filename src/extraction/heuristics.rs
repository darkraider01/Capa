use crate::signals::capability_registry::CapabilityRegistry;
use super::config::SignalConfig;
use super::models::{CapabilityType, Signal, SignalSource, SignalTier};
use std::collections::HashSet;

/// Detect all capability signals from a text string using the registry keyword tiers.
pub fn detect_all_capabilities(
    text: &str,
    source: SignalSource,
    config: &SignalConfig,
    registry: &CapabilityRegistry,
) -> Vec<Signal> {
    let mut signals = Vec::new();

    for cap_def in &registry.capabilities {
        if let Some(signal) = detect_capability(
            text,
            &cap_def.id,
            &cap_def.keywords.strict,
            &cap_def.keywords.soft,
            source.clone(),
            config,
        ) {
            signals.push(signal);
        }
    }

    signals
}

/// Detect a single capability in text using tiered keyword matching.
fn detect_capability(
    text: &str,
    cap_id: &str,
    strict: &[String],
    soft: &[String],
    source: SignalSource,
    config: &SignalConfig,
) -> Option<Signal> {
    let text_lower = text.to_lowercase();

    let mut strict_matches: HashSet<String> = HashSet::new();
    let mut soft_matches: HashSet<String> = HashSet::new();

    for kw in strict {
        if text_lower.contains(kw.as_str()) {
            strict_matches.insert(kw.clone());
        }
    }
    for kw in soft {
        if text_lower.contains(kw.as_str()) {
            soft_matches.insert(kw.clone());
        }
    }

    let base_score = strict_matches.len() as f32 * config.strict_weight
        + soft_matches.len() as f32 * config.soft_weight;

    let source_multiplier = match &source {
        SignalSource::RepoName(_) => config.repo_name_boost,
        SignalSource::RepoDescription(_) => config.repo_desc_boost,
        SignalSource::CommitMessage(_, _) => config.commit_boost,
    };

    let final_score = (base_score * source_multiplier).min(1.0);

    if final_score > 0.05 {
        let mut all_keywords: Vec<String> = Vec::new();
        all_keywords.extend(strict_matches.iter().cloned());
        all_keywords.extend(soft_matches.iter().cloned());

        let tier = if !strict_matches.is_empty() {
            SignalTier::Tier1
        } else {
            SignalTier::Tier2
        };

        Some(Signal {
            capability_type: CapabilityType::new(cap_id),
            score: final_score,
            keywords: all_keywords,
            source,
            tier,
            timestamp: chrono::Utc::now().timestamp(),
        })
    } else {
        None
    }
}

/// Check if a keyword appears as a distinct word in text (case-insensitive).
/// A match is valid if the characters immediately before and after the matched keyword span
/// are non-alphanumeric or string boundaries.
pub fn contains_word(text: &str, keyword: &str) -> bool {
    if text.is_empty() || keyword.is_empty() {
        return false;
    }

    let text_lower = text.to_lowercase();
    let kw_lower = keyword.to_lowercase();
    let kw_bytes = kw_lower.as_bytes();
    let text_bytes = text_lower.as_bytes();

    let mut start = 0;
    while let Some(pos) = text_lower[start..].find(&kw_lower) {
        let match_start = start + pos;
        let match_end = match_start + kw_bytes.len();

        let left_boundary = match_start == 0
            || !text_bytes[match_start - 1].is_ascii_alphanumeric();

        let right_boundary = match_end == text_bytes.len()
            || !text_bytes[match_end].is_ascii_alphanumeric();

        if left_boundary && right_boundary {
            return true;
        }

        start = match_start + 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_word_standalone() {
        assert!(contains_word("leetcode", "leetcode"));
        assert!(contains_word("my-leetcode-solutions", "leetcode"));
        assert!(contains_word("advent_of_code_2023", "advent_of_code"));
    }

    #[test]
    fn test_contains_word_embedded_false_positive() {
        assert!(!contains_word("leetcoder", "leetcode"));
        assert!(!contains_word("competitiveness", "competitive"));
        assert!(!contains_word("uncompetitive", "competitive"));
        assert!(!contains_word("hackerranker", "hackerrank"));
    }

    #[test]
    fn test_contains_word_hyphenated() {
        assert!(contains_word("my-competitive-programming-repo", "competitive-programming"));
        assert!(contains_word("my-competitive-programming-repo", "competitive"));
        assert!(contains_word("interview-prep-tool", "interview-prep"));
    }

    #[test]
    fn test_contains_word_multiple_occurrences() {
        // First occurrence is embedded (invalid), second occurrence is standalone (valid)
        assert!(contains_word("uncompetitiveness-and-competitive-coding", "competitive"));
    }
}

