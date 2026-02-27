use anyhow::Result;
use std::collections::HashMap;
use crate::config::SearchConfig;
use crate::extraction::config::ScoringWeights;
use crate::extraction::models::{CapabilityType, Signal, SignalSource, SignalTier};
use crate::extraction::scoring::{aggregate_all_signals, RepoSignals};
use crate::signals::capability_registry::CapabilityRegistry;

fn empty_repo_signals(name: &str) -> RepoSignals {
    RepoSignals {
        name: name.to_string(),
        language: None,
        stars: 0,
        keyword_signals: Vec::new(),
        dep_scores: HashMap::new(),
        filename_scores: HashMap::new(),
        structure_scores: HashMap::new(),
        language_scores: HashMap::new(),
        activity_scores: HashMap::new(),
        negative_signal_penalty: 0.0,
        age_decay: 1.0,
        commit_count: 10,
    }
}

pub async fn run_synthetic_tests(registry: &CapabilityRegistry) -> Result<()> {
    println!("\n🧪 Running Synthetic Adversarial Tests:");
    
    let config = SearchConfig::load()?;
    let mut passed = 0;
    let mut total = 0;

    // Test 1: LeetCode repository (should completely suppress signals due to penalty)
    total += 1;
    let mut leetcode_repo = empty_repo_signals("my-leetcode-solutions");
    leetcode_repo.negative_signal_penalty = 0.25; // Simulated match from pipeline.rs
    // Give it a strong algorithm signal to see if it gets squashed
    leetcode_repo.keyword_signals.push(Signal {
        capability_type: CapabilityType::new("DistributedAlgorithms"),
        score: 0.6,
        keywords: vec!["algorithm".to_string()],
        source: SignalSource::RepoDescription("leetcode".to_string()),
        tier: SignalTier::Tier1,
        timestamp: 0,
    });
    
    let weights = ScoringWeights::default();
    
    let caps = aggregate_all_signals("synth_1".to_string(), vec![leetcode_repo], 10, &weights, &registry.ids(), 0.0);
    // Let's just check if confidence is low.
    let is_suppressed = caps.is_empty() || caps.iter().all(|c| c.confidence < 0.5);
    if is_suppressed {
        println!("  ✅ PASS: LeetCode repository successfully suppressed.");
        passed += 1;
    } else {
        println!("  🚨 FAIL: LeetCode repository bypassed negative filter! Max conf: {}", caps.iter().map(|c| c.confidence).fold(0.0f32, f32::max));
    }

    // Test 2: Only numpy dependency (Weak ML)
    total += 1;
    let mut numpy_repo = empty_repo_signals("data-script");
    numpy_repo.dep_scores.insert("MachineLearning".to_string(), 0.08); // typical ecosystem dep score
    let caps = aggregate_all_signals("synth_2".to_string(), vec![numpy_repo], 10, &weights, &registry.ids(), 0.0);
    let ml_conf = caps.iter().find(|c| c.capability_type.as_str() == "MachineLearning").map(|c| c.confidence).unwrap_or(0.0);
    
    if ml_conf < 0.5 {
        println!("  ✅ PASS: Ecosystem dependency (numpy only) yielded weak signal ({:.2}).", ml_conf);
        passed += 1;
    } else {
        println!("  🚨 FAIL: NumPy single dependency over-classified ML! Conf: {}", ml_conf);
    }

    // Test 3: Only Dockerfile (Weak DevOps)
    total += 1;
    let mut docker_repo = empty_repo_signals("simple-web");
    docker_repo.filename_scores.insert("DevOpsAutomation".to_string(), 0.2); // Just the Dockerfile
    let caps = aggregate_all_signals("synth_3".to_string(), vec![docker_repo], 10, &weights, &registry.ids(), 0.0);
    let devops_conf = caps.iter().find(|c| c.capability_type.as_str() == "DevOpsAutomation").map(|c| c.confidence).unwrap_or(0.0);
    
    if devops_conf < 0.5 {
        println!("  ✅ PASS: Dockerfile alone yielded weak signal ({:.2}).", devops_conf);
        passed += 1;
    } else {
        println!("  🚨 FAIL: Dockerfile single file over-classified DevOps! Conf: {}", devops_conf);
    }

    println!("===================================");
    println!("Synthetic Tests Score: {}/{}", passed, total);
    println!("===================================\n");

    Ok(())
}
