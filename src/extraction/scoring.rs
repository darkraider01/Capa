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

struct RepoCapSignal {
    repo_name: String,
    keyword: f32,
    dependency: f32,
    filename: f32,
    structure: f32,
    language: f32,
    activity: f32,
    penalty: f32,
    total_signal: f32,
    evidence_keywords: Vec<String>,
    evidence_deps: Vec<String>,
}

/// Build the per-repo, per-capability signal breakdown shared by `aggregate_all_signals`
/// (which averages the top-3 repos per capability into one user-wide number) and
/// `per_repo_capability_scores` (which keeps every repo's own score intact).
fn build_repo_cap_signals(
    repos: &[RepoSignals],
    weights: &ScoringWeights,
) -> HashMap<String, Vec<RepoCapSignal>> {
    // Map: cap_id -> list of RepoCapSignal across repos
    let mut cap_repo_signals: HashMap<String, Vec<RepoCapSignal>> = HashMap::new();

    for repo in repos {
        let decay = repo.age_decay;
        let penalty = repo.negative_signal_penalty;

        // Keyword channel per cap for this repo
        let mut kw_by_cap: HashMap<String, (f32, Vec<String>)> = HashMap::new();
        for signal in &repo.keyword_signals {
            let (entry_score, entry_kws) = kw_by_cap
                .entry(signal.capability_type.0.clone())
                .or_insert_with(|| (0.0, Vec::new()));
            *entry_score = entry_score.max(signal.score);
            entry_kws.extend(signal.keywords.iter().cloned());
        }

        // Collect all cap_ids that have any score/signal in this repo
        let mut repo_cap_ids: HashSet<String> = HashSet::new();
        for id in kw_by_cap.keys() {
            repo_cap_ids.insert(id.clone());
        }
        for id in repo.dep_scores.keys() {
            repo_cap_ids.insert(id.clone());
        }
        for id in repo.filename_scores.keys() {
            repo_cap_ids.insert(id.clone());
        }
        for id in repo.structure_scores.keys() {
            repo_cap_ids.insert(id.clone());
        }
        for id in repo.language_scores.keys() {
            repo_cap_ids.insert(id.clone());
        }
        for id in repo.activity_scores.keys() {
            repo_cap_ids.insert(id.clone());
        }

        for cap_id in repo_cap_ids {
            let (kw_raw, kws) = kw_by_cap.get(&cap_id).cloned().unwrap_or((0.0, Vec::new()));
            let capped_kw = (kw_raw * decay).min(weights.max_repo_contribution);

            let dep_raw = repo.dep_scores.get(&cap_id).copied().unwrap_or(0.0);
            let capped_dep = (dep_raw * decay).min(weights.max_repo_contribution);

            let filename_raw = repo.filename_scores.get(&cap_id).copied().unwrap_or(0.0);
            let capped_filename = (filename_raw * decay).min(weights.max_repo_contribution);

            let structure_raw = repo.structure_scores.get(&cap_id).copied().unwrap_or(0.0);
            let capped_structure = (structure_raw * decay).min(weights.max_repo_contribution);

            let lang_score = repo.language_scores.get(&cap_id).copied().unwrap_or(0.0);
            let act_score = repo.activity_scores.get(&cap_id).copied().unwrap_or(0.0);

            let total_signal = capped_kw * weights.channels.keyword
                + capped_dep * weights.channels.dependency
                + capped_filename * weights.channels.filename
                + capped_structure * weights.channels.structure
                + act_score * weights.channels.activity;

            let deps = repo.dep_evidence.get(&cap_id).cloned().unwrap_or_default();

            cap_repo_signals
                .entry(cap_id)
                .or_default()
                .push(RepoCapSignal {
                    repo_name: repo.name.clone(),
                    keyword: capped_kw,
                    dependency: capped_dep,
                    filename: capped_filename,
                    structure: capped_structure,
                    language: lang_score,
                    activity: act_score,
                    penalty,
                    total_signal,
                    evidence_keywords: kws,
                    evidence_deps: deps,
                });
        }
    }

    cap_repo_signals
}

/// Real per-repo, per-capability strength — independent of the top-3-repo aggregate that
/// `aggregate_all_signals` folds into one user-wide confidence score. Every repo that has
/// any signal for a capability gets its own number here, on the same 0-1 scale as the
/// aggregate `confidence` (same sigmoid + activity-normalization pipeline, just applied to
/// one repo's signal instead of an averaged top-N). This is what makes per-project vectors
/// (e.g. for job-fit cosine similarity) actually reflect which project is strong at what,
/// instead of every evidence repo sharing the same aggregate number.
pub fn per_repo_capability_scores(
    repos: &[RepoSignals],
    total_user_commits: u64,
    weights: &ScoringWeights,
    min_confidence: f32,
) -> Vec<(String, String, f32)> {
    let cap_repo_signals = build_repo_cap_signals(repos, weights);
    let w = &weights.channels;
    let mut out = Vec::new();

    for (cap_id, contribs) in cap_repo_signals {
        for c in contribs {
            // Same "no real evidence" guard as aggregate_all_signals: keyword/dependency/
            // filename/structure are the only channels that establish a capability is
            // actually present. Without this, a repo with zero signal on all four still
            // gets pushed through sigmoid(0, alpha, beta), which isn't 0 — it's the
            // sigmoid's floor value (~0.25-0.27 pre-normalization here) — so every
            // capability the repo has *no* evidence for would still get a phantom
            // non-zero score, indistinguishable from genuinely weak-but-real evidence.
            if c.keyword == 0.0 && c.dependency == 0.0 && c.filename == 0.0 && c.structure == 0.0 {
                continue;
            }

            // Mirrors aggregate_all_signals: subtract this repo's own negative-keyword
            // penalty, then amplify (never create from zero) with its language signal.
            let raw_score = safe_f32((c.total_signal - c.penalty).max(0.0));
            let lang_boost = if raw_score > 0.0 {
                raw_score * c.language * w.language
            } else {
                0.0
            };
            let raw_with_lang = safe_f32(raw_score + lang_boost);
            let sigmoid_confidence = apply_sigmoid(raw_with_lang, weights.alpha, weights.beta);
            let normalized = safe_f32(apply_activity_normalization(
                sigmoid_confidence,
                total_user_commits,
                weights.normalization_factor,
            ));

            if normalized >= min_confidence {
                out.push((c.repo_name, cap_id.clone(), normalized));
            }
        }
    }

    out
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
    let cap_repo_signals = build_repo_cap_signals(&repos, weights);

    // Accumulators per capability (averaged over effective Top-N repo count)
    let mut cap_keyword: HashMap<String, f32> = HashMap::new();
    let mut cap_dep: HashMap<String, f32> = HashMap::new();
    let mut cap_filename: HashMap<String, f32> = HashMap::new();
    let mut cap_structure: HashMap<String, f32> = HashMap::new();
    let mut cap_language: HashMap<String, f32> = HashMap::new();
    let mut cap_activity: HashMap<String, f32> = HashMap::new();
    let mut cap_negative_penalty: HashMap<String, f32> = HashMap::new();
    let mut cap_keywords_evidence: HashMap<String, Vec<String>> = HashMap::new();
    let mut cap_repos_evidence: HashMap<String, Vec<String>> = HashMap::new();
    let mut cap_deps_evidence: HashMap<String, Vec<String>> = HashMap::new();

    for (cap_id, mut contribs) in cap_repo_signals {
        // Sort repos by total_signal descending (break ties by repo_name)
        contribs.sort_by(|a, b| {
            b.total_signal
                .partial_cmp(&a.total_signal)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.repo_name.cmp(&b.repo_name))
        });

        // Take top N (N = 3) repos
        let top_n: Vec<RepoCapSignal> = contribs.into_iter().take(3).collect();
        let effective_repo_count = top_n.len().max(1) as f32;

        for contrib in top_n {
            *cap_keyword.entry(cap_id.clone()).or_insert(0.0) += contrib.keyword / effective_repo_count;
            *cap_dep.entry(cap_id.clone()).or_insert(0.0) += contrib.dependency / effective_repo_count;
            *cap_filename.entry(cap_id.clone()).or_insert(0.0) += contrib.filename / effective_repo_count;
            *cap_structure.entry(cap_id.clone()).or_insert(0.0) += contrib.structure / effective_repo_count;
            *cap_language.entry(cap_id.clone()).or_insert(0.0) += contrib.language / effective_repo_count;
            *cap_activity.entry(cap_id.clone()).or_insert(0.0) += contrib.activity / effective_repo_count;
            *cap_negative_penalty.entry(cap_id.clone()).or_insert(0.0) += contrib.penalty / effective_repo_count;

            cap_keywords_evidence
                .entry(cap_id.clone())
                .or_default()
                .extend(contrib.evidence_keywords);
            cap_deps_evidence
                .entry(cap_id.clone())
                .or_default()
                .extend(contrib.evidence_deps);
            cap_repos_evidence
                .entry(cap_id.clone())
                .or_default()
                .push(contrib.repo_name);
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
        let evidence_keywords = collect_evidence_keywords(&type_signals);
        let evidence_repos = collect_evidence_repos(&type_signals);
        let active_repo_count = evidence_repos.len().min(3).max(1) as f32;

        let raw_score = safe_f32(
            (keyword_score * weights.channels.keyword + repo_score * weights.channels.filename)
                / active_repo_count,
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
            filename_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            structure_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
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
            filename_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            structure_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
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
        let repo1_clean = RepoSignals {
            name: "penalized-repo-1".to_string(),
            language: Some("Rust".to_string()),
            stars: 5,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            structure_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
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

    #[test]
    pub fn test_cross_repo_aggregation_does_not_dilute_unrelated_repos() {
        let strong_repo = RepoSignals {
            name: "strong-ml-project".to_string(),
            language: Some("Python".to_string()),
            stars: 20,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([("MachineLearning".to_string(), 0.8)]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::new(),
            structure_scores: HashMap::new(),
            language_scores: HashMap::new(),
            activity_scores: HashMap::new(),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 50,
        };

        // 10 unrelated repos with zero signals for MachineLearning
        let mut repos = vec![strong_repo.clone()];
        for i in 1..=10 {
            repos.push(RepoSignals {
                name: format!("unrelated-practice-repo-{}", i),
                language: Some("HTML".to_string()),
                stars: 0,
                keyword_signals: Vec::new(),
                dep_scores: HashMap::new(),
                dep_evidence: HashMap::new(),
                filename_scores: HashMap::new(),
                structure_scores: HashMap::new(),
                language_scores: HashMap::new(),
                activity_scores: HashMap::new(),
                negative_signal_penalty: 0.0,
                age_decay: 1.0,
                commit_count: 1,
            });
        }

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let single_repo_caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![strong_repo],
            100,
            &weights,
            &cap_ids,
            0.0,
        );

        let multi_repo_caps = aggregate_all_signals(
            "test_user".to_string(),
            repos,
            100,
            &weights,
            &cap_ids,
            0.0,
        );

        let single_ml = single_repo_caps.first().expect("ML cap in single repo");
        let multi_ml = multi_repo_caps.first().expect("ML cap in multi repo");

        assert!(
            (single_ml.signal_breakdown.dependency_score - multi_ml.signal_breakdown.dependency_score).abs() < 1e-4,
            "10 unrelated repos should not dilute single strong repo dependency score (single {}, multi {})",
            single_ml.signal_breakdown.dependency_score,
            multi_ml.signal_breakdown.dependency_score
        );
        assert_eq!(multi_ml.evidence_repos, vec!["strong-ml-project".to_string()]);
    }

    #[test]
    pub fn test_top_n_repo_capping() {
        // 5 repos with signals for MachineLearning
        let mut repos = Vec::new();
        for i in 1..=5 {
            let score = 0.9 - (i as f32 * 0.1); // 0.8, 0.7, 0.6, 0.5, 0.4
            repos.push(RepoSignals {
                name: format!("ml-repo-{}", i),
                language: Some("Python".to_string()),
                stars: 5,
                keyword_signals: Vec::new(),
                dep_scores: HashMap::from([("MachineLearning".to_string(), score)]),
                dep_evidence: HashMap::new(),
                filename_scores: HashMap::new(),
                structure_scores: HashMap::new(),
                language_scores: HashMap::new(),
                activity_scores: HashMap::new(),
                negative_signal_penalty: 0.0,
                age_decay: 1.0,
                commit_count: 10,
            });
        }

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            repos,
            50,
            &weights,
            &cap_ids,
            0.0,
        );

        let ml_cap = caps.first().expect("ML capability should be extracted");

        // Top 3 capped scores (0.35 max_repo_contribution limit applies to 0.8, 0.7, 0.6):
        // 0.35 + 0.35 + 0.35 = 1.05 / 3 = 0.35
        assert!(
            (ml_cap.signal_breakdown.dependency_score - 0.35).abs() < 1e-4,
            "Top-N dependency score should be 0.35, got {}",
            ml_cap.signal_breakdown.dependency_score
        );
        assert_eq!(ml_cap.evidence_repos.len(), 3, "evidence_repos should contain exactly top 3 repos");
    }

    #[test]
    pub fn test_top_n_divergent_channels_option_a() {
        // Repo A: Strong dependency score, zero keyword score
        let repo_a = RepoSignals {
            name: "repo-a-deps".to_string(),
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

        // Repo B: Strong keyword score, zero dependency score
        let mut repo_b = RepoSignals {
            name: "repo-b-kw".to_string(),
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
        repo_b.keyword_signals.push(Signal {
            capability_type: CapabilityType::new("MachineLearning"),
            score: 0.8,
            keywords: vec!["pytorch".to_string()],
            source: SignalSource::RepoName("repo-b-kw".to_string()),
            tier: super::super::models::SignalTier::Tier1,
            timestamp: 0,
        });

        let weights = ScoringWeights::default();
        let cap_ids = vec!["MachineLearning"];

        let caps = aggregate_all_signals(
            "test_user".to_string(),
            vec![repo_a, repo_b],
            10,
            &weights,
            &cap_ids,
            0.0,
        );

        let ml_cap = caps.first().expect("ML capability should be extracted");

        // Both repos have signal for ML, so effective_repo_count = 2.
        // Capped kw from repo_b = 0.35, capped dep from repo_a = 0.35.
        // Option A holistic averaging over 2 repos: kw = 0.35 / 2 = 0.175, dep = 0.35 / 2 = 0.175.
        assert!(
            (ml_cap.signal_breakdown.keyword_score - 0.175).abs() < 1e-4,
            "Option A keyword score should be 0.175, got {}",
            ml_cap.signal_breakdown.keyword_score
        );
        assert!(
            (ml_cap.signal_breakdown.dependency_score - 0.175).abs() < 1e-4,
            "Option A dependency score should be 0.175, got {}",
            ml_cap.signal_breakdown.dependency_score
        );
    }

    #[test]
    pub fn test_raw_score_zero_lands_in_weak_tier() {
        let weights = ScoringWeights::default();
        // At raw score 0.0, sigmoid confidence is 1 / (1 + e^(4.0 * 0.25)) = 0.2689 < 0.300 (Weak)
        let conf_zero = apply_sigmoid(0.0, weights.alpha, weights.beta);
        assert!(conf_zero < 0.300, "Raw score 0.0 confidence must be < 0.300, got {}", conf_zero);
        assert_eq!(
            CapabilityTier::from_confidence(conf_zero),
            CapabilityTier::Weak,
            "Raw score 0.0 floor must land in Weak tier"
        );
    }

    #[test]
    pub fn test_sigmoid_tier_crossovers() {
        let weights = ScoringWeights::default();
        // Crossovers under alpha=4.0, beta=0.25:
        // C=0.300 (Emerging threshold): raw = 0.039
        // C=0.400 (Strong threshold): raw = 0.149
        // C=0.500 (Proven threshold): raw = 0.250

        let conf_emerging = apply_sigmoid(0.039, weights.alpha, weights.beta);
        let conf_strong = apply_sigmoid(0.149, weights.alpha, weights.beta);
        let conf_proven = apply_sigmoid(0.250, weights.alpha, weights.beta);

        assert!(conf_emerging >= 0.300, "0.039 raw score should cross Emerging threshold (0.300), got {}", conf_emerging);
        assert_eq!(CapabilityTier::from_confidence(conf_emerging), CapabilityTier::Emerging);

        assert!(conf_strong >= 0.400, "0.149 raw score should cross Strong threshold (0.400), got {}", conf_strong);
        assert_eq!(CapabilityTier::from_confidence(conf_strong), CapabilityTier::Strong);

        assert!(conf_proven >= 0.500, "0.250 raw score should cross Proven threshold (0.500), got {}", conf_proven);
        assert_eq!(CapabilityTier::from_confidence(conf_proven), CapabilityTier::Proven);
    }

    #[test]
    pub fn test_entry_level_raw_score_smooth_progression() {
        let weights = ScoringWeights::default();
        // Entry-level raw score range: 0.10 to 0.20
        let conf_10 = apply_sigmoid(0.10, weights.alpha, weights.beta);
        let conf_20 = apply_sigmoid(0.20, weights.alpha, weights.beta);

        // Smooth progression without cliff edge: ~0.354 to ~0.450
        assert!((conf_10 - 0.3543).abs() < 1e-3, "0.10 raw score confidence expected ~0.354, got {}", conf_10);
        assert!((conf_20 - 0.4502).abs() < 1e-3, "0.20 raw score confidence expected ~0.450, got {}", conf_20);

        // Smooth tier progression (Emerging to Strong) without spurious Proven inflation
        assert_eq!(CapabilityTier::from_confidence(conf_10), CapabilityTier::Emerging);
        assert_eq!(CapabilityTier::from_confidence(conf_20), CapabilityTier::Strong);
    }

    #[test]
    pub fn test_flagship_senior_repo_reaches_proven() {
        // High-quality flagship project with multi-channel signals, language boost, and correlation boost
        let mut flagship1 = RepoSignals {
            name: "flagship-distributed-db".to_string(),
            language: Some("Rust".to_string()),
            stars: 500,
            keyword_signals: Vec::new(),
            dep_scores: HashMap::from([
                ("DistributedSystems".to_string(), 0.8),
                ("DatabaseStorageEngines".to_string(), 0.8),
            ]),
            dep_evidence: HashMap::new(),
            filename_scores: HashMap::from([
                ("DistributedSystems".to_string(), 0.8),
                ("DatabaseStorageEngines".to_string(), 0.8),
            ]),
            structure_scores: HashMap::from([
                ("DistributedSystems".to_string(), 0.8),
                ("DatabaseStorageEngines".to_string(), 0.8),
            ]),
            language_scores: HashMap::from([
                ("DistributedSystems".to_string(), 1.0),
                ("DatabaseStorageEngines".to_string(), 1.0),
            ]),
            activity_scores: HashMap::from([
                ("DistributedSystems".to_string(), 0.5),
                ("DatabaseStorageEngines".to_string(), 0.5),
            ]),
            negative_signal_penalty: 0.0,
            age_decay: 1.0,
            commit_count: 500,
        };
        flagship1.keyword_signals.push(Signal {
            capability_type: CapabilityType::new("DistributedSystems"),
            score: 0.8,
            keywords: vec!["raft".to_string(), "consensus".to_string()],
            source: SignalSource::RepoName("flagship-distributed-db".to_string()),
            tier: super::super::models::SignalTier::Tier1,
            timestamp: 0,
        });

        let mut flagship2 = flagship1.clone();
        flagship2.name = "flagship-kv-engine".to_string();

        let weights = ScoringWeights::default();
        let cap_ids = vec!["DistributedSystems", "DatabaseStorageEngines"];

        let caps = aggregate_all_signals(
            "senior_dev".to_string(),
            vec![flagship1, flagship2],
            1000,
            &weights,
            &cap_ids,
            0.0,
        );

        let dist_cap = caps
            .iter()
            .find(|c| c.capability_type.as_str() == "DistributedSystems")
            .expect("DistributedSystems should be extracted");

        assert!(
            dist_cap.signal_breakdown.raw_score >= 0.250,
            "Flagship senior raw score ({}) should cross 0.250 to reach Proven",
            dist_cap.signal_breakdown.raw_score
        );
        assert_eq!(
            dist_cap.tier,
            CapabilityTier::Proven,
            "Flagship senior project should achieve Proven tier"
        );
    }
}




