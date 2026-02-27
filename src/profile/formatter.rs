use super::builder::{CapabilityProfile, CapabilitySummary};

pub fn print_profile(profile: &CapabilityProfile) {
    println!("\nEntity: {}", profile.entity_id);

    if !profile.tech_stack.is_empty() {
        println!("\nTech Stack");
        println!("  {}", profile.tech_stack.join(" • "));
    }

    println!("\nPrimary Strengths");
    print_summaries(&profile.primary);

    if !profile.secondary.is_empty() {
        println!("\nSecondary");
        print_summaries(&profile.secondary);
    }

    if !profile.emerging.is_empty() {
        println!("\nEmerging");
        print_summaries(&profile.emerging);
    }

    println!("\nEvidence");
    print_evidence(&profile.primary);
    if !profile.secondary.is_empty() {
        print_evidence(&profile.secondary);
    }
}

fn print_summaries(summaries: &[CapabilitySummary]) {
    if summaries.is_empty() {
        println!("  (None)");
        return;
    }

    for sym in summaries {
        println!(
            "  {}: {} ({:.2})",
            sym.capability_type,
            sym.tier.to_uppercase(),
            sym.normalized_score
        );
    }
}

fn print_evidence(summaries: &[CapabilitySummary]) {
    for sym in summaries {
        // Take top 3 repos and top 5 keywords to avoid spam
        let repos: Vec<String> = sym.evidence_repos.iter().take(3).cloned().collect();
        let keywords: Vec<String> = sym
            .evidence_keywords
            .iter()
            .take(5)
            .map(|k| k.to_string())
            .collect();

        if !repos.is_empty() || !keywords.is_empty() {
            let repo_str = if repos.is_empty() {
                "General Structural Patterns".to_string()
            } else {
                repos.join(", ")
            };

            let kw_str = if keywords.is_empty() {
                "structural code patterns".to_string()
            } else {
                keywords.join(", ")
            };

            println!("  {} -> {}", repo_str, kw_str);
        }
    }
}
