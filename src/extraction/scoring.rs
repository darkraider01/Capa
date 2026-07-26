use super::config::{CapabilityTier, ScoringWeights, get_star_boost};
use super::models::{CapabilityType, ExtractedCapability, Signal, SignalBreakdown, SignalSource};
use std::collections::{HashMap, HashSet};

/// Per-repo aggregated signal data
#[derive(Debug, Clone, Default)]
pub struct RepoSignals {
    pub name: String,
    pub language: Option<String>,
    pub stars: u64,
    /// keyword channel signals from this repo
    pub keyword_signals: Vec<Signal>,
    /// dependency score per capability_id (already IDF-weighted)
    pub dep_scores: HashMap<String, f32>,
    /// dependency evidence (matched dep names) per capability_id
    pub dep_evidence: HashMap<String, Vec<String>>,
    /// filename score per capability_id
    pub filename_scores: HashMap<String, f32>,
    /// structure score per capability_id (composite-gated)
    pub structure_scores: HashMap<String, f32>,
    /// language score per capability_id (amplify-only)
    pub language_scores: HashMap<String, f32>,
    /// activity score per capability_id
    pub activity_scores: HashMap<String, f32>,
    /// Penalty to subtract if this repo matches negative keywords (e.g leetcode)
    pub negative_signal_penalty: f32,
    /// age decay factor for this repo (e^(-lambda * age_years))
    pub age_decay: f32,
    /// commit count (for density calc)
    pub commit_count: u64,
}

/// Aggregate all repo signals into final per-user capabilities
pub fn aggregate_all_signals(
    user_login: String,
    repos: Vec<RepoSignals>,
    total_user_commits: u64,
    weights: &ScoringWeights,
    capability_ids: &[&str],
    min_confidence: f32,
) -> Vec<ExtractedCapability> {
    // Per-capability accumulator {cap_id → channel scores summed across repos}
    let mut cap_keyword: HashMap<String, f32> = HashMap::new();
    let mut cap_dep: HashMap<String, f32> = HashMap::new();
    let mut cap_filename: HashMap<String, f32> = HashMap::new();
    let mut cap_structure: HashMap<String, f32> = HashMap::new();
    let mut cap_language: HashMap<String, f32> = HashMap::new();
    let mut cap_activity: HashMap<String, f32> = HashMap::new();
    // Accumulated negative penalty for capabilities from bad repos
    let mut cap_negative_penalty: HashMap<String, f32> = HashMap::new();
    let mut cap_keywords_evidence: HashMap<String, Vec<String>> = HashMap::new();
    let mut cap_repos_evidence: HashMap<String, Vec<String>> = HashMap::new();
    let mut cap_deps_evidence: HashMap<String, Vec<String>> = HashMap::new();

    let repo_count = repos.len() as f32;

    for repo in &repos {
        // Repo contribution cap: a single repo can contribute at most max_repo_contribution
        // We normalise by dividing each score by repo_count and then capping below.
        let decay = repo.age_decay;
        let penalty = repo.negative_signal_penalty;

        // 1. Keyword channel (from signal text matching)
        let mut kw_by_cap: HashMap<String, f32> = HashMap::new();
        for signal in &repo.keyword_signals {
            let entry = kw_by_cap
                .entry(signal.capability_type.0.clone())
                .or_insert(0.0);
            *entry = entry.max(signal.score); // take max within a repo
            cap_keywords_evidence
                .entry(signal.capability_type.0.clone())
                .or_default()
                .extend(signal.keywords.iter().cloned());
        }
        for (id, score) in &kw_by_cap {
            let capped = (score * decay).min(weights.max_repo_contribution);
            *cap_keyword.entry(id.clone()).or_insert(0.0) += capped / repo_count;
            cap_repos_evidence
                .entry(id.clone())
                .or_default()
                .push(repo.name.clone());
        }

        // 2. Dependency channel
        for (id, score) in &repo.dep_scores {
            let capped = (score * decay).min(weights.max_repo_contribution);
            *cap_dep.entry(id.clone()).or_insert(0.0) += capped / repo_count;
            cap_repos_evidence
                .entry(id.clone())
                .or_default()
                .push(repo.name.clone());
            if let Some(deps) = repo.dep_evidence.get(id) {
                cap_deps_evidence
                    .entry(id.clone())
                    .or_default()
                    .extend(deps.iter().cloned());
            }
        }

        // 3. Filename channel
        for (id, score) in &repo.filename_scores {
            let capped = (score * decay).min(weights.max_repo_contribution);
            *cap_filename.entry(id.clone()).or_insert(0.0) += capped / repo_count;
            cap_repos_evidence
                .entry(id.clone())
                .or_default()
                .push(repo.name.clone());
        }

        // 4. Structure channel (composite-gated, already pre-gated in project_structure)
        for (id, score) in &repo.structure_scores {
            let capped = (score * decay).min(weights.max_repo_contribution);
            *cap_structure.entry(id.clone()).or_insert(0.0) += capped / repo_count;
            cap_repos_evidence
                .entry(id.clone())
                .or_default()
                .push(repo.name.clone());
        }

        // 5. Language channel (amplify-only — applied AFTER raw_score is computed below)
        for (id, score) in &repo.language_scores {
            *cap_language.entry(id.clone()).or_insert(0.0) += score / repo_count;
        }

        // 6. Activity channel
        for (id, score) in &repo.activity_scores {
            *cap_activity.entry(id.clone()).or_insert(0.0) += score / repo_count;
        }

        // Deduplicated negative penalty application (once per repo per capability across the 4 penalized channels)
        if penalty > 0.0 {
            let mut repo_penalized_caps: HashSet<&str> = HashSet::new();
            for id in kw_by_cap.keys() {
                repo_penalized_caps.insert(id.as_str());
            }
            for id in repo.dep_scores.keys() {
                repo_penalized_caps.insert(id.as_str());
            }
            for id in repo.filename_scores.keys() {
                repo_penalized_caps.insert(id.as_str());
            }
            for id in repo.structure_scores.keys() {
                repo_penalized_caps.insert(id.as_str());
            }
            for id in repo_penalized_caps {
                *cap_negative_penalty.entry(id.to_string()).or_insert(0.0) += penalty / repo_count;
            }
        }
    }

    // Collect all capability ids that have any signal at all
    let mut active_caps: HashSet<String> = HashSet::new();
    for id in capability_ids {
        active_caps.insert(id.to_string());
    }

    let mut capabilities: Vec<ExtractedCapability> = Vec::new();

    let repo_data: Vec<RepoData> = repos
        .iter()
        .map(|r| RepoData {
            name: r.name.clone(),
            language: r.language.clone(),
            stars: r.stars,
        })
        .collect();

    for cap_id in &active_caps {
        let kw = *cap_keyword.get(cap_id).unwrap_or(&0.0);
        let dep = *cap_dep.get(cap_id).unwrap_or(&0.0);
        let filename = *cap_filename.get(cap_id).unwrap_or(&0.0);
        let structure = *cap_structure.get(cap_id).unwrap_or(&0.0);
        let activity = *cap_activity.get(cap_id).unwrap_or(&0.0);
        let lang_amplifier = *cap_language.get(cap_id).unwrap_or(&0.0);
        let negative_penalty = *cap_negative_penalty.get(cap_id).unwrap_or(&0.0);

        // Skip if no evidence from any strong channel
        if dep == 0.0 && filename == 0.0 && structure == 0.0 && kw == 0.0 {
            continue;
        }

        let w = &weights.channels;
        let base_raw = dep * w.dependency
                + filename * w.filename
                + structure * w.structure
                + kw * w.keyword
                + activity * w.activity;
                
        // Subtract negative signal penalty, prevent dropping below zero
        let raw_score = safe_f32((base_raw - negative_penalty).max(0.0));

        // Language amplify-only: boosts existing score, cannot create from zero
        let lang_boost = if raw_score > 0.0 {
            raw_score * lang_amplifier * w.language
        } else {
            0.0
        };

        let raw_with_lang = safe_f32(raw_score + lang_boost);

        // Sigmoid
        let sigmoid_confidence = apply_sigmoid(raw_with_lang, weights.alpha, weights.beta);

        // Activity normalization
        let normalized_confidence = safe_f32(apply_activity_normalization(
            sigmoid_confidence,
            total_user_commits,
            weights.normalization_factor,
        ));

        if normalized_confidence >= min_confidence {
            let evidence_keywords = {
                let mut kws: Vec<String> = cap_keywords_evidence
                    .get(cap_id)
                    .cloned()
                    .unwrap_or_default();
                kws.sort();
                kws.dedup();
                kws
            };
            let evidence_repos = {
                let mut rs = cap_repos_evidence.get(cap_id).cloned().unwrap_or_default();
                rs.sort();
                rs.dedup();
                rs
            };

            // Repo score for evidence (star boost)
            let max_stars = repo_data.iter().map(|r| r.stars).max().unwrap_or(0);
            let _repo_boost = get_star_boost(max_stars);

            let tier = CapabilityTier::from_confidence(normalized_confidence);

            let mut cap = ExtractedCapability::new(
                user_login.clone(),
                CapabilityType::new(cap_id),
                normalized_confidence,
                tier,
                SignalBreakdown {
                    keyword_score: kw,
                    dependency_score: dep,
                    filename_score: filename,
                    structure_score: structure,
                    language_score: lang_amplifier * w.language,
                    activity_score: activity,
                    raw_score: raw_with_lang,
                    time_decay_factor: 1.0, // already applied per-repo above
                    correlation_boost: 0.0, // applied later
                },
                evidence_keywords,
                evidence_repos,
            );

            // Attach dep evidence
            let evidence_deps = {
                let mut deps: Vec<String> = cap_deps_evidence
                    .get(cap_id)
                    .cloned()
                    .unwrap_or_default();
                deps.sort();
                deps.dedup();
                deps
            };
            cap.evidence_deps = evidence_deps;

            capabilities.push(cap);
        }
    }

    apply_correlation_boosts(&mut capabilities, weights.correlation_boost_factor);
    capabilities
}

// ─── Legacy adapter for keyword-only path (used by existing pipeline until Task 8) ───────────

/// Aggregate keyword-only signals (legacy compatibility shim)
pub fn aggregate_signals(
    user_login: String,
    signals: Vec<Signal>,
    repos: &[RepoData],
    total_user_commits: u64,
    config: &super::config::SignalConfig,
    weights: &ScoringWeights,
) -> Vec<ExtractedCapability> {
    let mut grouped: HashMap<CapabilityType, Vec<Signal>> = HashMap::new();
    for signal in signals {
        grouped
            .entry(signal.capability_type.clone())
            .or_default()
            .push(signal);
    }

    let mut capabilities = Vec::new();

    for (cap_type, type_signals) in grouped {
        let keyword_score = safe_f32(calculate_keyword_score(&type_signals));
        let repo_score = safe_f32(calculate_repo_score(&type_signals, repos));
        let raw_score = safe_f32(
            keyword_score * weights.channels.keyword + repo_score * weights.channels.filename,
        );

        let sigmoid_confidence = apply_sigmoid(raw_score, weights.alpha, weights.beta);
        let time_decay_factor = safe_f32(calculate_time_decay(&type_signals, 0.05));
        let decayed_confidence = safe_f32(sigmoid_confidence * time_decay_factor);
        let normalized_confidence = safe_f32(apply_activity_normalization(
            decayed_confidence,
            total_user_commits,
            weights.normalization_factor,
        ));

        if normalized_confidence >= config.min_confidence {
            let evidence_keywords = collect_evidence_keywords(&type_signals);
            let evidence_repos = collect_evidence_repos(&type_signals);
            let tier = CapabilityTier::from_confidence(normalized_confidence);

            capabilities.push(ExtractedCapability::new(
                user_login.clone(),
                cap_type,
                normalized_confidence,
                tier,
                SignalBreakdown {
                    keyword_score,
                    dependency_score: 0.0,
                    filename_score: repo_score,
                    structure_score: 0.0,
                    language_score: 0.0,
                    activity_score: 0.0,
                    raw_score,
                    time_decay_factor,
                    correlation_boost: 0.0,
                },
                evidence_keywords,
                evidence_repos,
            ));
        }
    }

    apply_correlation_boosts(&mut capabilities, weights.correlation_boost_factor);
    capabilities
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

pub struct RepoData {
    pub name: String,
    pub language: Option<String>,
    pub stars: u64,
}

fn safe_f32(value: f32) -> f32 {
    if value.is_nan() || value.is_infinite() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn apply_sigmoid(raw_score: f32, alpha: f32, beta: f32) -> f32 {
    let result = 1.0 / (1.0 + (-alpha * (raw_score - beta)).exp());
    safe_f32(result)
}

fn apply_activity_normalization(confidence: f32, total_commits: u64, factor: f32) -> f32 {
    let activity_level = (total_commits as f32 + 1.0).ln();
    let penalty = 1.0 + (activity_level * factor);
    confidence / penalty
}

fn calculate_time_decay(signals: &[Signal], lambda: f32) -> f32 {
    if signals.is_empty() {
        return 1.0;
    }
    let now = chrono::Utc::now().timestamp();
    let total: f32 = signals
        .iter()
        .map(|s| {
            let months = (now - s.timestamp) as f32 / (30.0 * 24.0 * 3600.0);
            (-lambda * months).exp()
        })
        .sum();
    total / signals.len() as f32
}

fn calculate_keyword_score(signals: &[Signal]) -> f32 {
    if signals.is_empty() {
        return 0.0;
    }
    let sum: f32 = signals.iter().map(|s| s.score).sum();
    let count = signals.len() as f32;
    let avg = sum / count;
    let boost = ((count - 1.0) * 0.03).min(0.15);
    (avg + boost).min(1.0)
}

fn calculate_repo_score(signals: &[Signal], repos: &[RepoData]) -> f32 {
    let mut score = 0.0;
    let repo_name_signals = signals
        .iter()
        .filter(|s| matches!(s.source, SignalSource::RepoName(_)))
        .count();
    if repo_name_signals > 0 {
        score += 0.3 * (repo_name_signals as f32).min(3.0) / 3.0;
    }
    let max_stars = repos.iter().map(|r| r.stars).max().unwrap_or(0);
    score += (get_star_boost(max_stars) - 1.0) * 0.5;
    score.min(1.0)
}

fn collect_evidence_keywords(signals: &[Signal]) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for s in signals {
        set.extend(s.keywords.iter().cloned());
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

fn collect_evidence_repos(signals: &[Signal]) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for s in signals {
        let name = match &s.source {
            SignalSource::RepoName(n) | SignalSource::RepoDescription(n) => n.clone(),
            SignalSource::CommitMessage(n, _) => n.clone(),
        };
        set.insert(name);
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

fn apply_correlation_boosts(capabilities: &mut Vec<ExtractedCapability>, boost_factor: f32) {
    // Simple correlation: capabilities in the same meta-category boost each other slightly
    let snap: Vec<(String, f32)> = capabilities
        .iter()
        .map(|c| (c.capability_type.0.clone(), c.confidence))
        .collect();

    for cap in capabilities.iter_mut() {
        let mut total_boost = 0.0f32;
        for (other_id, other_conf) in &snap {
            if other_id != &cap.capability_type.0 {
                total_boost += other_conf * boost_factor * 0.1;
            }
        }
        let total_boost = total_boost.min(0.1); // hard cap on boost
        cap.signal_breakdown.correlation_boost = total_boost;
        cap.confidence = safe_f32(cap.confidence + total_boost);
        cap.tier = CapabilityTier::from_confidence(cap.confidence);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    pub fn test_aggregate_all_signals_populates_evidence_deps() {
        let mut repo = RepoSignals {
            name: "test-repo".to_string(),
            language: Some("Rust".to_string()),
            stars: 10,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::new(),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 5,
        };

        repo.dep_scores.insert("MachineLearning".to_string(), 0.8);
        repo.dep_evidence.insert(
            "MachineLearning".to_string(),
            vec!["torch".to_string(), "numpy".to_string()],
        );

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let ml_cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "MachineLearning")
            .expect("MachineLearning capability should be extracted");

        assert_eq!(ml_cap.evidence_deps, vec!["numpy".to_string(), "torch".to_string()]);
    }

    #[test]
    pub fn test_aggregate_all_signals_populates_evidence_repos_non_keyword() {
        let mut repo = RepoSignals {
            name: "repo-with-deps-only".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::new(),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 5,
        };

        repo.dep_scores.insert("DistributedAlgorithms".to_string(), 0.7);

        let weights = ScoringWeights::default();
        let cap_ids = vec!["DistributedAlgorithms"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "DistributedAlgorithms")
            .expect("DistributedAlgorithms capability should be extracted");

        assert_eq!(cap.evidence_repos, vec!["repo-with-deps-only".to_string()]);
    }

    #[test]
    pub fn test_aggregate_all_signals_applies_correlation_boost_multiple_capabilities() {
        let mut repo = RepoSignals {
            name: "multi-cap-repo".to_string(),
            language: Some("Rust".to_string()),
            stars: 10,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::new(),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 5,
        };

        repo.dep_scores.insert("MachineLearning".to_string(), 0.8);
        repo.dep_scores.insert("DistributedAlgorithms".to_string(), 0.7);

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning", "DistributedAlgorithms"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let ml_cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "MachineLearning")
            .expect("MachineLearning should be extracted");
        let dist_cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "DistributedAlgorithms")
            .expect("DistributedAlgorithms should be extracted");

        assert!(
            ml_cap.signal_breakdown.correlation_boost > 0.0,
            "ML capability should receive a positive correlation boost"
        );
        assert!(
            dist_cap.signal_breakdown.correlation_boost > 0.0,
            "DistributedAlgorithms capability should receive a positive correlation boost"
        );

        // Expected boost calculation: other_conf * boost_factor * 0.1
        // Verify correlation_boost matches the expected boost factor mathematically
        let boost_factor = weights.correlation_boost_factor; // 0.1 by default
        let dist_pre_boost_conf = dist_cap.confidence - dist_cap.signal_breakdown.correlation_boost;
        let expected_ml_boost = (dist_pre_boost_conf * boost_factor * 0.1).min(0.1);

        assert!(
            (ml_cap.signal_breakdown.correlation_boost - expected_ml_boost).abs() < 1e-5,
            "ML boost ({}) did not match expected ({})",
            ml_cap.signal_breakdown.correlation_boost,
            expected_ml_boost
        );
    }

    #[test]
    pub fn test_aggregate_all_signals_no_correlation_boost_single_capability() {
        let mut repo = RepoSignals {
            name: "single-cap-repo".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::new(),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 5,
        };

        repo.dep_scores.insert("DistributedAlgorithms".to_string(), 0.7);

        let weights = ScoringWeights::default();
        let cap_ids = vec!["DistributedAlgorithms"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let dist_cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "DistributedAlgorithms")
            .expect("DistributedAlgorithms should be extracted");

        assert_eq!(
            dist_cap.signal_breakdown.correlation_boost, 0.0,
            "Single capability should have zero correlation boost"
        );
    }

    #[test]
    pub fn test_aggregate_all_signals_negative_penalty_deduped_per_repo() {
        let mut repo = RepoSignals {
            name: "penalized-repo".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::new(),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.25,
            age_decay: 1.0,
            commit_count: 5,
        };

        // Populate evidence across 3 channels for the same capability
        repo.dep_scores.insert("MachineLearning".to_string(), 0.5);
        repo.filename_scores.insert("MachineLearning".to_string(), 0.5);
        repo.structure_scores.insert("MachineLearning".to_string(), 0.5);

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo.clone()],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        // Control case: identical repo without negative penalty
        let mut unpenalized_repo = repo.clone();
        unpenalized_repo.negative_signal_penalty = 0.0;
        let control_caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![unpenalized_repo],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let cap = caps.first().expect("Capability should be extracted");
        let control_cap = control_caps.first().expect("Control capability should be extracted");

        // raw_score difference should reflect a single 0.25 penalty subtraction (before channel weighting subtraction)
        // In aggregate_all_signals: base_raw = channel_sum; raw_score = (base_raw - negative_penalty).max(0.0)
        // With repo_count = 1, negative_penalty is exactly 0.25 (not 3 * 0.25 = 0.75)
        let expected_raw_diff = 0.25;
        let actual_raw_diff = control_cap.signal_breakdown.raw_score - cap.signal_breakdown.raw_score;
        assert!(
            (actual_raw_diff - expected_raw_diff).abs() < 1e-4,
            "Negative penalty was applied {} times instead of 1 time",
            actual_raw_diff / 0.25
        );
    }

    #[test]
    pub fn test_aggregate_all_signals_negative_penalty_sums_across_repos() {
        let repo1 = RepoSignals {
            name: "penalized-repo-1".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.25,
            age_decay: 1.0,
            commit_count: 5,
        };

        let repo2 = RepoSignals {
            name: "penalized-repo-2".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.25,
            age_decay: 1.0,
            commit_count: 5,
        };

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo1, repo2],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let cap = caps.first().expect("Capability should be extracted");

        // Control case: both repos without negative penalty
        let mut repo1_clean = RepoSignals {
            name: "penalized-repo-1".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 5,
        };
        let mut repo2_clean = repo1_clean.clone();
        repo2_clean.name = "penalized-repo-2".to_string();

        let control_caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo1_clean, repo2_clean],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let control_cap = control_caps.first().expect("Control capability should be extracted");

        // With repo_count = 2, each repo contributes 0.25 / 2 = 0.125 penalty. Total penalty = 0.25.
        let actual_raw_diff = control_cap.signal_breakdown.raw_score - cap.signal_breakdown.raw_score;
        assert!(
            (actual_raw_diff - 0.25).abs() < 1e-4,
            "Penalties across multiple repos did not sum correctly. Diff was {}",
            actual_raw_diff
        );
    }
}


