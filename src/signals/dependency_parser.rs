use crate::signals::capability_registry::CapabilityRegistry;
use std::collections::HashMap;

/// A dependency-based signal: one package → possibly multiple capabilities
#[derive(Debug, Clone)]
pub struct DependencySignal {
    pub dep_name: String,
    pub capability_id: String,
    /// Base score before IDF weighting (0.0–1.0)
    pub base_score: f32,
}

/// Per-capability aggregate score from dependency signals in one repo
#[derive(Debug, Default)]
pub struct DepCapabilityScores(
    pub HashMap<String, f32>,
    pub HashMap<String, Vec<String>>,
);

/// Supported manifest filenames (checked at top of repo tree)
pub const MANIFEST_FILES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "composer.json",
    "Gemfile",
];

/// Filter and prioritize manifest files from a repository file tree.
/// Matches manifest basenames against MANIFEST_FILES and .csproj files.
/// Sorts by path depth ascending (shallowest root-level files first),
/// using lowercased path as a deterministic tie-breaker, then truncates to max_manifests.
pub fn filter_and_prioritize_manifests(tree: &[String], max_manifests: usize) -> Vec<String> {
    let mut matches: Vec<String> = tree
        .iter()
        .filter(|p| {
            let basename = p.split('/').last().unwrap_or(p);
            MANIFEST_FILES.iter().any(|m| {
                m.eq_ignore_ascii_case(basename) || p.to_lowercase().ends_with(".csproj")
            })
        })
        .cloned()
        .collect();

    matches.sort_by_key(|p| (p.matches('/').count(), p.to_lowercase()));
    matches.truncate(max_manifests);
    matches
}

/// Parse raw package names from a manifest file.
/// Returns lowercase, version-stripped package names.
pub fn parse_dependencies(filename: &str, content: &str) -> Vec<String> {
    let fname = filename.to_lowercase();
    let fname = fname.split('/').last().unwrap_or(&fname);

    match fname {
        "cargo.toml" => parse_cargo_toml(content),
        "package.json" => parse_package_json(content),
        "requirements.txt" => parse_requirements_txt(content),
        "pyproject.toml" => parse_pyproject_toml(content),
        "go.mod" => parse_go_mod(content),
        "pom.xml" => parse_pom_xml(content),
        "build.gradle" | "build.gradle.kts" => parse_gradle(content),
        "composer.json" => parse_composer_json(content),
        "gemfile" => parse_gemfile(content),
        _ if fname.ends_with(".csproj") => parse_csproj(content),
        _ => Vec::new(),
    }
}

/// Convert a list of package names → capability signals using the registry.
/// Applies IDF weighting using dep_frequencies (dep → number of repos that use it).
pub fn dep_signals(
    deps: &[String],
    registry: &CapabilityRegistry,
    dep_frequencies: &HashMap<String, u64>,
    total_repos: u64,
) -> DepCapabilityScores {
    let mut scores: HashMap<String, f32> = HashMap::new();
    let mut evidence: HashMap<String, Vec<String>> = HashMap::new();

    for dep in deps {
        let dep_lower = dep.to_lowercase();
        let caps_info = registry.caps_for_dep(&dep_lower);
        if caps_info.is_empty() {
            continue;
        }

        // IDF weight: rarer libraries are stronger signals
        let freq = dep_frequencies.get(&dep_lower).copied().unwrap_or(1).max(1);
        let idf = if total_repos > 0 {
            (total_repos as f32 / freq as f32).ln().max(0.0)
        } else {
            1.0
        };
        // Normalize IDF to [0, 1] range
        let idf_normalized = (idf / (100.0_f32).ln()).min(1.0);
        
        for (cap_id, is_core) in caps_info {
            // Core dependencies get base 0.34, ecosystem gets base 0.17
            let base_score = if *is_core { 0.34_f32 } else { 0.17_f32 };
            // IDF modifier scales baseline from 0.6x (common) up to 1.0x (rare)
            let idf_modifier = 0.6_f32 + 0.4_f32 * idf_normalized;
            let signal_score = (base_score * idf_modifier).max(0.05);

            let entry = scores.entry(cap_id.clone()).or_insert(0.0);
            *entry = entry.max(signal_score);

            evidence
                .entry(cap_id.clone())
                .or_default()
                .push(dep_lower.clone());
        }
    }

    DepCapabilityScores(scores, evidence)
}

// ─── Parsers ──────────────────────────────────────────────────────────────────

fn parse_cargo_toml(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers starting with '[' and ending with ']'
        if trimmed.starts_with('[') {
            let header = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim();

            // 1. Sub-table dependency headers: [dependencies.tokio], [dev-dependencies.criterion], [workspace.dependencies.serde]
            let subtable_pkg = if let Some(pos) = header.find("dependencies.") {
                Some(&header[pos + "dependencies.".len()..])
            } else {
                None
            };

            if let Some(raw_pkg) = subtable_pkg {
                let pkg = raw_pkg
                    .trim()
                    .trim_matches(|c: char| c == '"' || c == '\'' || c == ']');
                if !pkg.is_empty() && !pkg.starts_with('#') {
                    deps.push(pkg.to_lowercase());
                }
                in_deps = false;
                continue;
            }

            // 2. Section dependency headers: [dependencies], [dev-dependencies], [build-dependencies], [workspace.dependencies], [target.'cfg(...)'.dependencies]
            if header.ends_with("dependencies")
                || header.ends_with("dev-dependencies")
                || header.ends_with("build-dependencies")
            {
                in_deps = true;
                continue;
            }

            // 3. Any other header (e.g. [package], [profile.release]):
            in_deps = false;
            continue;
        }

        if !in_deps {
            continue;
        }

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse "name = ..." or "name = { ... }"
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'');
            if !key.is_empty() && !key.starts_with('#') {
                deps.push(key.to_lowercase());
            }
        }
    }

    deps
}

fn parse_package_json(content: &str) -> Vec<String> {
    let mut deps = Vec::new();

    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return deps;
    };

    let sections = ["dependencies", "devDependencies", "peerDependencies"];
    for section in &sections {
        if let Some(obj) = v.get(section).and_then(|s| s.as_object()) {
            for key in obj.keys() {
                // Strip namespace scopes like @types/
                let name = if key.starts_with('@') {
                    key.split('/').nth(1).unwrap_or(key)
                } else {
                    key.as_str()
                };
                deps.push(name.to_lowercase());
            }
        }
    }

    deps
}

fn parse_requirements_txt(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Skip comments, options (-r, --index-url etc), empty lines
            if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
                return None;
            }
            // Strip version specifiers: pkg==1.0, pkg>=1.0, pkg[extra]
            let name = line
                .split(|c: char| c == '=' || c == '>' || c == '<' || c == '[' || c == ';')
                .next()
                .unwrap_or(line)
                .trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_lowercase())
            }
        })
        .collect()
}

#[derive(PartialEq)]
enum PyprojectParseMode {
    None,
    Array,
    Table,
}

fn parse_pyproject_toml(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut mode = PyprojectParseMode::None;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Section header check
        if trimmed.starts_with('[') {
            if trimmed == "[project.dependencies]" {
                mode = PyprojectParseMode::Array;
                continue;
            } else if trimmed.starts_with("[tool.poetry.")
                && (trimmed.ends_with("dependencies]") || trimmed.ends_with("dev-dependencies]"))
            {
                mode = PyprojectParseMode::Table;
                continue;
            } else {
                mode = PyprojectParseMode::None;
                continue;
            }
        }

        // Inline array check e.g. "dependencies = ["
        if mode == PyprojectParseMode::None
            && (trimmed == "dependencies = [" || trimmed.starts_with("dependencies = ["))
        {
            mode = PyprojectParseMode::Array;
            continue;
        }

        match mode {
            PyprojectParseMode::Array => {
                if trimmed == "]" || trimmed.starts_with(']') {
                    mode = PyprojectParseMode::None;
                    continue;
                }
                // Lines like: "requests>=2.0", '"pandas"'
                let name = trimmed
                    .trim_matches(|c: char| c == '"' || c == '\'' || c == ',')
                    .split(|c: char| c == '=' || c == '>' || c == '<' || c == '[' || c == ';')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && !name.starts_with('#') {
                    deps.push(name.to_lowercase());
                }
            }
            PyprojectParseMode::Table => {
                if let Some(eq_pos) = trimmed.find('=') {
                    let key = trimmed[..eq_pos]
                        .trim()
                        .trim_matches(|c: char| c == '"' || c == '\'');
                    if !key.is_empty() && !key.starts_with('#') && key != "python" {
                        deps.push(key.to_lowercase());
                    }
                }
            }
            PyprojectParseMode::None => {}
        }
    }

    deps
}

fn parse_go_mod(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "require (" {
            in_require = true;
            continue;
        }
        if trimmed == ")" {
            in_require = false;
            continue;
        }

        // Inline: require github.com/foo/bar v1.2.3
        let target = if trimmed.starts_with("require ") {
            trimmed.strip_prefix("require ").unwrap_or("").trim()
        } else if in_require {
            trimmed
        } else {
            continue;
        };

        // Extract just the repo basename: github.com/gorilla/mux → mux
        if let Some(module_path) = target.split_whitespace().next() {
            let name = module_path.split('/').last().unwrap_or(module_path);
            if !name.is_empty() {
                deps.push(name.to_lowercase());
            }
        }
    }

    deps
}

fn parse_pom_xml(content: &str) -> Vec<String> {
    // Simple regex-free extract of <artifactId> values
    let mut deps = Vec::new();
    for part in content.split("<artifactId>") {
        if let Some(end) = part.find("</artifactId>") {
            let name = part[..end].trim();
            if !name.is_empty() {
                deps.push(name.to_lowercase());
            }
        }
    }
    deps
}

fn parse_gradle(content: &str) -> Vec<String> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        // Patterns: implementation("group:artifact:version") or implementation 'g:a:v'
        for kw in &["implementation", "api", "testImplementation", "compile"] {
            if trimmed.starts_with(kw) {
                // Extract quoted string
                for quote in &['"', '\''] {
                    if let Some(start) = trimmed.find(*quote) {
                        if let Some(end) = trimmed[start + 1..].find(*quote) {
                            let coord = &trimmed[start + 1..start + 1 + end];
                            // coord = "group:artifact:version" → take artifact (index 1)
                            if let Some(artifact) = coord.split(':').nth(1) {
                                deps.push(artifact.to_lowercase());
                            }
                        }
                    }
                }
            }
        }
    }

    deps
}

fn parse_composer_json(content: &str) -> Vec<String> {
    let mut deps = Vec::new();

    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return deps;
    };

    for section in &["require", "require-dev"] {
        if let Some(obj) = v.get(section).and_then(|s| s.as_object()) {
            for key in obj.keys() {
                // vendor/package → take package part
                let name = key.split('/').last().unwrap_or(key.as_str());
                deps.push(name.to_lowercase());
            }
        }
    }

    deps
}

fn parse_gemfile(content: &str) -> Vec<String> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("gem ") {
            // gem 'rails', '~> 7.0'  or  gem "sidekiq"
            for quote in &['"', '\''] {
                if let Some(start) = trimmed.find(*quote) {
                    if let Some(end) = trimmed[start + 1..].find(*quote) {
                        let name = &trimmed[start + 1..start + 1 + end];
                        if !name.is_empty() {
                            deps.push(name.to_lowercase());
                        }
                        break;
                    }
                }
            }
        }
    }

    deps
}

fn parse_csproj(content: &str) -> Vec<String> {
    let mut deps = Vec::new();

    for part in content.split("<PackageReference") {
        // <PackageReference Include="Microsoft.Extensions.Logging" Version="..." />
        if let Some(include_pos) = part.to_lowercase().find("include=") {
            let after = &part[include_pos + 8..];
            for quote in &['"', '\''] {
                if let Some(start) = after.find(*quote) {
                    if let Some(end) = after[start + 1..].find(*quote) {
                        let name = &after[start + 1..start + 1 + end];
                        // Take last segment of dotted name
                        let short = name.split('.').last().unwrap_or(name);
                        deps.push(short.to_lowercase());
                        break;
                    }
                }
            }
        }
    }

    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_toml_parsing() {
        let content = r#"
[package]
name = "my-app"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = "1"
axum = "0.7"

[dev-dependencies]
criterion = "0.5"
"#;
        let deps = parse_dependencies("Cargo.toml", content);
        assert!(deps.contains(&"tokio".to_string()));
        assert!(deps.contains(&"serde".to_string()));
        assert!(deps.contains(&"axum".to_string()));
        assert!(deps.contains(&"criterion".to_string()));
    }

    #[test]
    fn test_cargo_toml_subtable_and_target_parsing() {
        let content = r#"
[package]
name = "subtable-app"

[dependencies.tokio]
version = "1.0"
features = ["full"]

[dev-dependencies.criterion]
version = "0.5"

[workspace.dependencies]
serde = "1.0"

[target.'cfg(unix)'.dependencies]
openssl = "0.10"

[target.'cfg(windows)'.dependencies.winapi]
version = "0.3"
"#;
        let deps = parse_dependencies("Cargo.toml", content);
        assert!(deps.contains(&"tokio".to_string()), "tokio should be parsed from subtable header");
        assert!(deps.contains(&"criterion".to_string()), "criterion should be parsed from dev subtable header");
        assert!(deps.contains(&"serde".to_string()), "serde should be parsed from workspace.dependencies");
        assert!(deps.contains(&"openssl".to_string()), "openssl should be parsed from target dependencies");
        assert!(deps.contains(&"winapi".to_string()), "winapi should be parsed from target subtable header");

        // Regression guard: ensure sub-table body properties are not parsed as dependency names
        assert!(!deps.contains(&"version".to_string()), "sub-table 'version' key must not be parsed as dep");
        assert!(!deps.contains(&"features".to_string()), "sub-table 'features' key must not be parsed as dep");
        assert!(!deps.contains(&"full".to_string()), "sub-table feature string must not be parsed as dep");
    }


    #[test]
    fn test_package_json_parsing() {
        let content = r#"{
  "dependencies": { "react": "^18.0.0", "lodash": "4.0.0" },
  "devDependencies": { "typescript": "^5.0.0" }
}"#;
        let deps = parse_dependencies("package.json", content);
        assert!(deps.contains(&"react".to_string()));
        assert!(deps.contains(&"lodash".to_string()));
        assert!(deps.contains(&"typescript".to_string()));
    }

    #[test]
    fn test_requirements_txt_parsing() {
        let content = "requests==2.28.0\nnumpy>=1.24\n# comment\ntorch\n";
        let deps = parse_dependencies("requirements.txt", content);
        assert!(deps.contains(&"requests".to_string()));
        assert!(deps.contains(&"numpy".to_string()));
        assert!(deps.contains(&"torch".to_string()));
    }

    #[test]
    fn test_go_mod_parsing() {
        let content = "module myapp\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.0\n\tgolang.org/x/net v0.12.0\n)\n";
        let deps = parse_dependencies("go.mod", content);
        assert!(deps.contains(&"gin".to_string()));
    }

    #[test]
    fn test_idf_weighting() {
        let registry = crate::signals::capability_registry::CapabilityRegistry::load().unwrap();
        let mut freqs = HashMap::new();
        freqs.insert("raft-rs".to_string(), 1u64); // very rare
        freqs.insert("serde".to_string(), 50u64); // very common FIXME: use real freq

        let rare_dep = vec!["raft-rs".to_string()];
        let common_dep = vec!["serde".to_string()];

        let rare_signals = dep_signals(&rare_dep, &registry, &freqs, 100);
        let common_signals = dep_signals(&common_dep, &registry, &freqs, 100);

        // raft-rs should score higher than serde for the same capability
        let raft_score = rare_signals.0.values().cloned().fold(0.0_f32, f32::max);
        let serde_score = common_signals.0.values().cloned().fold(0.0_f32, f32::max);

        if raft_score > 0.0 && serde_score > 0.0 {
            assert!(
                raft_score > serde_score,
                "rare dep should score higher than common dep"
            );
        }
    }

    #[test]
    fn test_pyproject_toml_parsing() {
        let poetry_content = r#"
[tool.poetry]
name = "my-poetry-app"

[tool.poetry.dependencies]
python = "^3.9"
requests = "^2.28.1"
torch = { version = "^2.0.0" }

[tool.poetry.dev-dependencies]
pytest = "^7.0"

[tool.poetry.group.formatting.dependencies]
black = "^22.0"

[build-system]
requires = ["poetry-core"]
"#;
        let deps = parse_dependencies("pyproject.toml", poetry_content);
        assert!(deps.contains(&"requests".to_string()));
        assert!(deps.contains(&"torch".to_string()));
        assert!(deps.contains(&"pytest".to_string()));
        assert!(deps.contains(&"black".to_string()));
        assert!(!deps.contains(&"python".to_string()));
        assert!(!deps.contains(&"poetry-core".to_string()));

        let pep621_content = r#"
[project]
name = "my-pep621-app"
dependencies = [
    "flask>=2.0",
    "pandas",
]
"#;
        let pep_deps = parse_dependencies("pyproject.toml", pep621_content);
        assert!(pep_deps.contains(&"flask".to_string()));
        assert!(pep_deps.contains(&"pandas".to_string()));
    }

    #[test]
    fn test_filter_and_prioritize_manifests() {
        let tree = vec![
            "packages/service-c/package.json".to_string(),
            "packages/service-a/package.json".to_string(),
            "packages/service-b/package.json".to_string(),
            "package.json".to_string(),
            "src/App.csproj".to_string(),
            "README.md".to_string(),
        ];

        let selected = filter_and_prioritize_manifests(&tree, 3);
        assert_eq!(
            selected,
            vec![
                "package.json".to_string(),
                "src/App.csproj".to_string(),
                "packages/service-a/package.json".to_string(),
            ]
        );
    }

    #[test]
    fn test_dep_signals_idf_and_baseline_score() {
        let registry = CapabilityRegistry::load().unwrap();

        let mut dep_freqs_common = HashMap::new();
        dep_freqs_common.insert("tokio".to_string(), 100);

        let mut dep_freqs_rare = HashMap::new();
        dep_freqs_rare.insert("tokio".to_string(), 1);

        // Core dependency: tokio -> ConcurrentProgramming
        let DepCapabilityScores(scores_common, _) =
            dep_signals(&["tokio".to_string()], &registry, &dep_freqs_common, 100);

        let DepCapabilityScores(scores_rare, _) =
            dep_signals(&["tokio".to_string()], &registry, &dep_freqs_rare, 100);

        let common_score = *scores_common.get("ConcurrentProgramming").unwrap();
        let rare_score = *scores_rare.get("ConcurrentProgramming").unwrap();

        // Common core dependency gets healthy baseline (>= 0.20), not crushed to 0.05
        assert!(common_score >= 0.20, "Common score should be >= 0.20, got {}", common_score);
        // Rare core dependency scores higher than common core dependency
        assert!(rare_score > common_score, "Rare ({}) should exceed common ({})", rare_score, common_score);
    }

    #[test]
    fn test_dep_score_fits_under_max_repo_contribution_cap() {
        let registry = CapabilityRegistry::load().unwrap();
        let cap = crate::extraction::config::ScoringWeights::default().max_repo_contribution;

        let mut dep_freqs_common = HashMap::new();
        dep_freqs_common.insert("tokio".to_string(), 100);
        let mut dep_freqs_rare = HashMap::new();
        dep_freqs_rare.insert("tokio".to_string(), 1);

        let DepCapabilityScores(scores_common, _) =
            dep_signals(&["tokio".to_string()], &registry, &dep_freqs_common, 100);
        let DepCapabilityScores(scores_rare, _) =
            dep_signals(&["tokio".to_string()], &registry, &dep_freqs_rare, 100);

        let raw_common = *scores_common.get("ConcurrentProgramming").unwrap();
        let raw_rare = *scores_rare.get("ConcurrentProgramming").unwrap();

        // Raw scores must sit strictly under the 0.35 max_repo_contribution cap
        assert!(raw_common <= cap, "raw_common ({}) exceeds cap ({})", raw_common, cap);
        assert!(raw_rare <= cap, "raw_rare ({}) exceeds cap ({})", raw_rare, cap);

        // After applying the cap min(cap), rarity differentiation MUST be preserved!
        let post_cap_common = raw_common.min(cap);
        let post_cap_rare = raw_rare.min(cap);

        assert!(
            post_cap_rare > post_cap_common,
            "Post-cap rare ({}) must be strictly greater than post-cap common ({})",
            post_cap_rare,
            post_cap_common
        );
    }
}


