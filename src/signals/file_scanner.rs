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

/// Paths that reduce signal strength by default, dynamically attenuated based on evidence quality
const WEAK_SIGNAL_TOKENS: &[&str] = &[
    "test", "tests", "testing", "example", "examples", "sample", "samples", "demo", "demos",
    "tutorial", "tutorials",
];

/// Extensions for primary executable / domain source code files
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "go", "cpp", "c", "h", "hpp", "java", "ts", "js", "jsx", "tsx", "rb", "swift",
    "kt", "cs", "hs", "zig", "scala", "ex", "exs", "sql", "sh", "r", "jl", "m", "mm", "clj",
    "erl", "elm", "v", "nim", "lua",
];

/// Generic / boilerplate filenames commonly found in tutorials or practice repos
const GENERIC_FILENAME_TOKENS: &[&str] = &[
    "main", "index", "hello", "world", "test", "demo", "sample", "example", "app", "temp",
    "foo", "bar", "baz", "basic", "simple", "buffer_demo", "schema_test", "test_file",
];

/// Score per capability from filename token scanning of a repo
#[derive(Debug, Default)]
pub struct FilenameScores(pub HashMap<String, f32>);

/// Scan a list of file paths and return per-capability scores.
///
/// Rules:
/// - Paths starting with a vendor prefix are skipped entirely
/// - Paths containing test/demo/example/tutorial tokens receive dynamic attenuation (0.40x - 0.80x)
///   based on evidence quality (source code extension bonus + domain capability bonus with generic filename veto)
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

        let filename = *segments.last().unwrap_or(&"");
        let tokens = tokenise_filename(filename);

        // 1. Gather all capability matches first
        let mut matched_caps: Vec<String> = Vec::new();
        for token in &tokens {
            let cap_ids = registry.caps_for_token(token);
            for cap_id in cap_ids {
                matched_caps.push(cap_id.clone());
            }
        }

        // 2. Determine signal multiplier dynamically based on evidence quality
        let multiplier = compute_path_multiplier(&segments, filename, matched_caps.len());

        // 3. Accumulate scores
        for cap_id in matched_caps {
            let entry = scores.entry(cap_id).or_insert(0.0);
            *entry = (*entry + 0.15 * multiplier).min(1.0);
        }
    }

    FilenameScores(scores)
}

/// Determine dynamic path signal multiplier.
/// - Unflagged paths get 1.0x.
/// - Flagged paths (test/demo/example/tutorial) start with a base 0.40x multiplier.
/// - Source code extension bonus: +0.20x for real source files.
/// - Domain bonus (+0.20x): ONLY applies if `matched_cap_count > 0 && !is_generic_filename`.
/// - Capped at 0.80x for flagged paths.
fn compute_path_multiplier(
    segments: &[&str],
    filename: &str,
    matched_cap_count: usize,
) -> f32 {
    if !has_weak_signal_segment(segments) {
        return 1.0;
    }

    let mut multiplier = 0.40_f32;

    if is_source_code_file(filename) {
        multiplier += 0.20;
    }

    // Domain bonus: strictly gated by (matched capabilities AND NOT generic filename)
    let is_generic = is_generic_filename(filename);
    if matched_cap_count > 0 && !is_generic {
        multiplier += 0.20;
    }

    multiplier.min(0.80)
}

fn is_source_code_file(filename: &str) -> bool {
    if let Some(dot_idx) = filename.rfind('.') {
        let ext = filename[dot_idx + 1..].to_lowercase();
        SOURCE_EXTENSIONS.iter().any(|&e| e == ext)
    } else {
        false
    }
}

fn is_generic_filename(filename: &str) -> bool {
    let base = filename
        .rfind('.')
        .map(|i| &filename[..i])
        .unwrap_or(filename)
        .to_lowercase();

    GENERIC_FILENAME_TOKENS.iter().any(|&g| {
        base == g
            || base.contains("hello")
            || base.contains("demo")
            || base.contains("sample")
            || base.contains("test")
            || base.contains("basic")
            || base.contains("simple")
    })
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

        assert!(
            real_score > test_score,
            "real path score ({}) should be higher than test path score ({})",
            real_score,
            test_score
        );
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

    #[test]
    fn test_generic_filename_veto_on_borderline_cases() {
        let reg = registry();
        // buffer_demo.rs matches "buffer" (DatabaseInternals file_tokens), but is a generic demo name.
        // It gets 0.40 (base) + 0.20 (source ext) = 0.60x multiplier, domain bonus is vetoed by generic filename.
        let generic_demo_paths = vec!["examples/buffer_demo.rs".to_string()];
        let rich_domain_paths = vec!["examples/lexer_parser.rs".to_string()];

        let generic_scores = scan_filenames(&generic_demo_paths, &reg);
        let rich_scores = scan_filenames(&rich_domain_paths, &reg);

        let generic_score = generic_scores.0.get("DatabaseInternals").copied().unwrap_or(0.0);
        let rich_score = rich_scores.0.get("CompilersLanguageTooling").copied().unwrap_or(0.0);

        // Expected: 0.15 * 0.60 = 0.09 for generic_score (1 token)
        // Expected: 2 * (0.15 * 0.80) = 0.24 for rich_score (2 matching tokens: lexer + parser)
        assert!(
            (generic_score - 0.09).abs() < 1e-4,
            "Generic filename veto should produce 0.60x multiplier (score 0.09), got {}",
            generic_score
        );
        assert!(
            (rich_score - 0.24).abs() < 1e-4,
            "Rich domain implementation with 2 matching tokens in practice folder should produce 0.80x multiplier per token (score 0.24), got {}",
            rich_score
        );
    }

    #[test]
    fn test_tutorial_non_code_file() {
        let reg = registry();
        // tutorials/lexer.txt: non-code extension (.txt) gets base 0.40x + domain bonus 0.20x (non-generic lexer) = 0.60x
        let text_paths = vec!["tutorials/lexer.txt".to_string()];
        let text_scores = scan_filenames(&text_paths, &reg);
        let text_score = text_scores.0.get("CompilersLanguageTooling").copied().unwrap_or(0.0);

        // Expected: 0.15 * 0.60 = 0.09
        assert!(
            (text_score - 0.09).abs() < 1e-4,
            "Non-code domain tutorial file should receive 0.60x multiplier (score 0.09), got {}",
            text_score
        );
    }
}
