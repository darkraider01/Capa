use std::collections::HashMap;

/// Activity-based signal: commit message pattern → capability inference
#[derive(Debug, Default)]
pub struct ActivityScores(pub HashMap<String, f32>);

/// Commit message pattern → (capability_id, weight)
/// Patterns are checked as lowercase substrings.
const ACTIVITY_PATTERNS: &[(&str, &str, f32)] = &[
    // Performance engineering
    ("bench", "PerformanceEngineering", 0.8),
    ("perf", "PerformanceEngineering", 0.7),
    ("optimize", "PerformanceEngineering", 0.5),
    ("profil", "PerformanceEngineering", 0.6), // profile / profiling
    ("flamegraph", "PerformanceEngineering", 0.9),
    // DevOps & automation
    ("ci:", "DevOpsAutomation", 0.7),
    ("cd:", "DevOpsAutomation", 0.6),
    ("workflow", "DevOpsAutomation", 0.6),
    ("github actions", "DevOpsAutomation", 0.9),
    ("deploy", "DevOpsAutomation", 0.5),
    ("release", "DevOpsAutomation", 0.4),
    ("dockerfile", "DevOpsAutomation", 0.7),
    ("helm", "DevOpsAutomation", 0.7),
    // Library / maintainer signals → maps to domain-specific capabilities
    ("bump", "DevOpsAutomation", 0.3),      // version bumps
    ("upgrade", "WebBackendAPI", 0.2),       // upgrades often in backend services
    ("dockerfile", "CloudInfrastructure", 0.5),
    // Compiler / tooling
    ("refactor", "CompilersLanguageTooling", 0.2),
    ("ast", "CompilersLanguageTooling", 0.7),
    ("tokenize", "CompilersLanguageTooling", 0.8),
    ("parse", "CompilersLanguageTooling", 0.5),
    // Database
    ("migration", "DatabaseUsage", 0.7),
    ("schema", "DatabaseUsage", 0.5),
    ("sql", "DatabaseUsage", 0.5),
    // Security
    ("cve", "SecurityEngineering", 0.9),
    ("vuln", "SecurityEngineering", 0.8),
    ("exploit", "SecurityEngineering", 0.9),
    ("auth", "SecurityEngineering", 0.5),
    ("tls", "SecurityEngineering", 0.6),
    // ML / training
    ("train", "MachineLearning", 0.6),
    ("model", "MachineLearning", 0.4),
    ("dataset", "MachineLearning", 0.6),
    ("epoch", "MachineLearning", 0.9),
    ("loss", "MachineLearning", 0.7),
];

/// Analyse commit messages to produce activity-based capability scores.
///
/// Score formula:  `pattern_matches / total_commits` (normalized, not absolute)
/// This prevents large repos from dominating via sheer volume.
pub fn analyze_activity(commit_messages: &[String]) -> ActivityScores {
    let total = commit_messages.len();
    if total == 0 {
        return ActivityScores::default();
    }

    // Count pattern hits per (capability, pattern)
    let mut pattern_hits: HashMap<(&str, &str), u64> = HashMap::new();

    for message in commit_messages {
        let msg_lower = message.to_lowercase();
        for &(pattern, cap_id, _weight) in ACTIVITY_PATTERNS {
            if msg_lower.contains(pattern) {
                *pattern_hits.entry((cap_id, pattern)).or_insert(0) += 1;
            }
        }
    }

    // Aggregate into per-capability score: normalized hit rate × pattern weight
    let mut cap_scores: HashMap<String, f32> = HashMap::new();

    for &(pattern, cap_id, weight) in ACTIVITY_PATTERNS {
        let hits = pattern_hits.get(&(cap_id, pattern)).copied().unwrap_or(0);
        if hits == 0 {
            continue;
        }

        // Normalized: hits / total_commits
        let rate = hits as f32 / total as f32;
        // Scale by pattern weight and accumulate
        let contribution = rate * weight;

        let entry = cap_scores.entry(cap_id.to_string()).or_insert(0.0);
        *entry = (*entry + contribution).min(1.0);
    }

    ActivityScores(cap_scores)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_commits_signal_performance() {
        let messages: Vec<String> = (0..10)
            .map(|i| {
                if i < 3 {
                    "bench: add hash benchmark".to_string()
                } else {
                    "fix: minor cleanup".to_string()
                }
            })
            .collect();

        let scores = analyze_activity(&messages);
        let perf = scores
            .0
            .get("PerformanceEngineering")
            .copied()
            .unwrap_or(0.0);
        assert!(perf > 0.0, "bench commits should signal PerformanceEngineering");
    }

    #[test]
    fn test_normalization_same_rate_different_volume() {
        // 3/10 vs 30/100 bench commits → same normalized rate
        let small: Vec<String> = (0..10)
            .map(|i| {
                if i < 3 {
                    "bench: test".to_string()
                } else {
                    "fix".to_string()
                }
            })
            .collect();
        let large: Vec<String> = (0..100)
            .map(|i| {
                if i < 30 {
                    "bench: test".to_string()
                } else {
                    "fix".to_string()
                }
            })
            .collect();

        let small_scores = analyze_activity(&small);
        let large_scores = analyze_activity(&large);

        let small_perf = small_scores
            .0
            .get("PerformanceEngineering")
            .copied()
            .unwrap_or(0.0);
        let large_perf = large_scores
            .0
            .get("PerformanceEngineering")
            .copied()
            .unwrap_or(0.0);

        // Scores should be approximately equal (within 5%)
        let diff = (small_perf - large_perf).abs();
        assert!(
            diff < 0.05,
            "normalized scores should be equal regardless of volume: {} vs {}",
            small_perf,
            large_perf
        );
    }

    #[test]
    fn test_empty_commits() {
        let scores = analyze_activity(&[]);
        assert!(scores.0.is_empty());
    }
}
