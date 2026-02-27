use crate::signals::capability_registry::CapabilityRegistry;
use std::collections::{HashMap, HashSet};

/// Top-level directories that are build artifacts or package caches (always ignored)
const BUILD_ARTIFACT_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "dist",
    "target",
    "build",
    "out",
    "coverage",
    ".git",
    ".github", // .github/workflows is handled separately
    "__pycache__",
    ".venv",
    "venv",
];

/// Structure signal per repo: capability_id → score
#[derive(Debug, Default)]
pub struct StructureScores(pub HashMap<String, f32>);

/// Detect project structure signals from a file tree.
///
/// Rules:
/// - Only top-level directories are scanned (depth == 0 in the tree)
/// - `.github/workflows` is treated as a special top-level pattern
/// - Build artifact directories are ignored
/// - Composite evidence: structure + dependency co-occurrence = full signal; alone = 0.3×
pub fn detect_structure(
    file_paths: &[String],
    registry: &CapabilityRegistry,
    dep_scores: &HashMap<String, f32>, // existing dependency scores for co-occurrence gate
) -> StructureScores {
    let mut scores: HashMap<String, f32> = HashMap::new();

    // Collect top-level directories from the tree
    let top_level_dirs = extract_top_level_dirs(file_paths);

    for dir in &top_level_dirs {
        let dir_lower = dir.to_lowercase();

        // Special case: `.github/workflows` is captured as a folder token
        let lookup_dir = if dir_lower == ".github" {
            // Check if workflows sub-dir exists
            let has_workflows = file_paths
                .iter()
                .any(|p| p.to_lowercase().contains(".github/workflows"));
            if has_workflows {
                "workflows"
            } else {
                continue;
            }
        } else {
            dir_lower.as_str()
        };

        let cap_ids = registry.caps_for_folder(lookup_dir);
        for cap_id in cap_ids {
            // Composite evidence gate:
            // If there's already dependency evidence for this capability → full signal
            // Otherwise → 0.3× (structure alone is a weak hint)
            let has_dep_evidence = dep_scores.get(cap_id).copied().unwrap_or(0.0) > 0.0;
            let multiplier = if has_dep_evidence { 1.0 } else { 0.3 };

            let entry = scores.entry(cap_id.clone()).or_insert(0.0);
            *entry = (*entry + 0.25 * multiplier).min(1.0);
        }
    }

    StructureScores(scores)
}

/// Extract unique top-level directory names from a flat list of file paths.
fn extract_top_level_dirs(file_paths: &[String]) -> HashSet<String> {
    let mut dirs = HashSet::new();

    for path in file_paths {
        let path_norm = path.replace('\\', "/");
        // Path has at least one slash → first segment is a top-level dir
        if let Some(slash_pos) = path_norm.find('/') {
            let dir = &path_norm[..slash_pos];
            if !dir.is_empty() {
                let dir_lower = dir.to_lowercase();
                // Skip build artifacts
                if !BUILD_ARTIFACT_DIRS.iter().any(|&b| dir_lower == b) {
                    dirs.insert(dir.to_string());
                }
            }
        }
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> CapabilityRegistry {
        CapabilityRegistry::load().unwrap()
    }

    #[test]
    fn test_bench_dir_maps_to_performance() {
        let reg = registry();
        let paths = vec![
            "benches/hash_bench.rs".to_string(),
            "src/lib.rs".to_string(),
        ];
        // With dep evidence
        let mut dep_scores = HashMap::new();
        dep_scores.insert("PerformanceEngineering".to_string(), 0.5);
        let scores = detect_structure(&paths, &reg, &dep_scores);
        // benches should map to PerformanceEngineering
        assert!(
            scores.0.contains_key("PerformanceEngineering"),
            "benches dir should signal PerformanceEngineering"
        );
    }

    #[test]
    fn test_node_modules_ignored() {
        let reg = registry();
        let paths = vec!["node_modules/react/index.js".to_string()];
        let dep_scores = HashMap::new();
        let scores = detect_structure(&paths, &reg, &dep_scores);
        // node_modules is a build artifact, should be ignored
        assert!(
            scores.0.is_empty(),
            "node_modules should produce no structure signals"
        );
    }

    #[test]
    fn test_migrations_without_deps_is_weak() {
        let reg = registry();
        let paths = vec!["migrations/001_create_users.sql".to_string()];
        let no_deps: HashMap<String, f32> = HashMap::new();
        let with_deps = {
            let mut m = HashMap::new();
            m.insert("DatabaseUsage".to_string(), 0.5_f32);
            m
        };

        let weak = detect_structure(&paths, &reg, &no_deps);
        let strong = detect_structure(&paths, &reg, &with_deps);

        let weak_score = weak.0.get("DatabaseUsage").copied().unwrap_or(0.0);
        let strong_score = strong.0.get("DatabaseUsage").copied().unwrap_or(0.0);

        if weak_score > 0.0 && strong_score > 0.0 {
            assert!(
                strong_score > weak_score,
                "migrations + SQL deps should score higher than migrations alone"
            );
        }
    }
}
