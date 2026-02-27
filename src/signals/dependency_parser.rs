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
pub struct DepCapabilityScores(pub HashMap<String, f32>);

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
            // Core dependencies get 100% of the weight, ecosystem gets 40%
            let base_multiplier = if *is_core { 1.0_f32 } else { 0.4_f32 };
            let raw_score = 0.8_f32 * idf_normalized * base_multiplier;
            let signal_score = raw_score.max(0.05); // minimum signal floor

            let entry = scores.entry(cap_id.clone()).or_insert(0.0);
            *entry = entry.max(signal_score);
        }
    }

    DepCapabilityScores(scores)
}

// ─── Parsers ──────────────────────────────────────────────────────────────────

fn parse_cargo_toml(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]"
                || trimmed == "[dev-dependencies]"
                || trimmed == "[build-dependencies]";
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
            let key = trimmed[..eq_pos].trim().trim_matches('"');
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

fn parse_pyproject_toml(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[project.dependencies]"
            || trimmed == "dependencies = ["
            || trimmed.starts_with("dependencies = [")
        {
            in_deps = true;
            continue;
        }

        if in_deps {
            if trimmed == "]" {
                in_deps = false;
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
}
