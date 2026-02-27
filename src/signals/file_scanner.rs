use crate::signals::capability_registry::CapabilityRegistry;
use std::collections::HashMap;

/// Paths that should be ignored entirely (vendored / generated code)
const VENDOR_PREFIXES: &[&str] = &[
    "vendor",
    "third_party",
    "external",
    "deps",
    "submodules",
    "node_modules",
    ".git",
];

/// Paths that reduce signal strength by 70% (test/demo/example code)
const WEAK_SIGNAL_TOKENS: &[&str] = &[
    "test", "tests", "testing", "example", "examples", "sample", "samples", "demo", "demos",
    "tutorial", "tutorials",
];

/// Score per capability from filename token scanning of a repo
#[derive(Debug, Default)]
pub struct FilenameScores(pub HashMap<String, f32>);

/// Scan a list of file paths and return per-capability scores.
///
/// Rules:
/// - Paths starting with a vendor prefix are skipped entirely
/// - Paths containing test/demo/example tokens get 0.30× signal
/// - Token matching done on filename (not extension) split on `_`, `-`, `.`
pub fn scan_filenames(file_paths: &[String], registry: &CapabilityRegistry) -> FilenameScores {
    let mut scores: HashMap<String, f32> = HashMap::new();

    for path in file_paths {
        // Normalise path separators
        let path_norm = path.replace('\\', "/");
        let segments: Vec<&str> = path_norm.split('/').collect();

        // Skip vendored / generated paths
        if is_vendored(&segments) {
            continue;
        }

        // Determine signal multiplier (weakened for test/demo code)
        let multiplier = if has_weak_signal_segment(&segments) {
            0.30
        } else {
            1.0
        };

        // Tokenise the filename (last segment), strip extension
        let filename = *segments.last().unwrap_or(&"");
        let tokens = tokenise_filename(filename);

        for token in &tokens {
            let cap_ids = registry.caps_for_token(token);
            for cap_id in cap_ids {
                let entry = scores.entry(cap_id.clone()).or_insert(0.0);
                // Accumulate but cap at 1.0
                *entry = (*entry + 0.15 * multiplier).min(1.0);
            }
        }
    }

    FilenameScores(scores)
}

/// Splits a filename into lowercase tokens by `_`, `-`, `.` separators.
/// Extension is dropped.
fn tokenise_filename(filename: &str) -> Vec<String> {
    // Remove extension
    let base = filename
        .rfind('.')
        .map(|i| &filename[..i])
        .unwrap_or(filename);

    base.split(|c: char| c == '_' || c == '-' || c == '.')
        .filter(|t| t.len() >= 3) // ignore single/double char fragments
        .map(|t| t.to_lowercase())
        .collect()
}

fn is_vendored(segments: &[&str]) -> bool {
    if let Some(first) = segments.first() {
        let first_lower = first.to_lowercase();
        VENDOR_PREFIXES.iter().any(|p| first_lower == *p)
    } else {
        false
    }
}

fn has_weak_signal_segment(segments: &[&str]) -> bool {
    for seg in segments {
        let seg_lower = seg.to_lowercase();
        if WEAK_SIGNAL_TOKENS.iter().any(|t| seg_lower == *t) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> CapabilityRegistry {
        CapabilityRegistry::load().unwrap()
    }

    #[test]
    fn test_tokenise_filename() {
        let tokens = tokenise_filename("raft_node.rs");
        assert!(tokens.contains(&"raft".to_string()));
        assert!(tokens.contains(&"node".to_string()));
    }

    #[test]
    fn test_vendored_paths_ignored() {
        let reg = registry();
        let paths = vec!["vendor/lexer/parser.rs".to_string()];
        let scores = scan_filenames(&paths, &reg);
        assert!(
            scores.0.is_empty(),
            "vendor paths should produce no signals"
        );
    }

    #[test]
    fn test_test_path_weakened() {
        let reg = registry();
        // Same token in test vs non-test path
        let test_paths = vec!["tests/lexer_test.rs".to_string()];
        let real_paths = vec!["src/lexer.rs".to_string()];

        let test_scores = scan_filenames(&test_paths, &reg);
        let real_scores = scan_filenames(&real_paths, &reg);

        let test_score = test_scores
            .0
            .get("CompilersLanguageTooling")
            .copied()
            .unwrap_or(0.0);
        let real_score = real_scores
            .0
            .get("CompilersLanguageTooling")
            .copied()
            .unwrap_or(0.0);

        if test_score > 0.0 && real_score > 0.0 {
            assert!(
                real_score > test_score,
                "test-path score should be lower than real-path score"
            );
        }
    }

    #[test]
    fn test_lexer_file_maps_to_compilers() {
        let reg = registry();
        let paths = vec!["src/lexer.rs".to_string()];
        let scores = scan_filenames(&paths, &reg);
        assert!(
            scores.0.contains_key("CompilersLanguageTooling"),
            "lexer.rs should signal CompilersLanguageTooling"
        );
    }
}
