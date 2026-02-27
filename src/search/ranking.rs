/// Calculate final ranking score
///
/// Formula: w1 * score + w2 * recency + w3 * keyword_match
pub fn calculate_final_score(
    score: f32,
    timestamp: i64,
    keyword_match_score: f32,
    weights: (f32, f32, f32),
) -> f32 {
    let (score_w, recency_w, keyword_match_w) = weights;

    // Calculate recency score (0-1, decays over 365 days)
    let now = chrono::Utc::now().timestamp();
    let days_ago = ((now - timestamp) as f32 / (24.0 * 3600.0)).max(0.0);
    let recency_score = (-days_ago / 365.0).exp();

    // Weighted combination
    score_w * score + recency_w * recency_score + keyword_match_w * keyword_match_score
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_WEIGHTS: (f32, f32, f32) = (0.7, 0.2, 0.1);

    #[test]
    fn test_final_score_calculation() {
        let now = chrono::Utc::now().timestamp();

        // Recent, high confidence, good keyword match
        let score1 = calculate_final_score(0.8, now, 1.0, DEFAULT_WEIGHTS);
        assert!(score1 > 0.85);

        // Old, low confidence, no keyword match
        let old_timestamp = now - (365 * 24 * 3600);
        let score2 = calculate_final_score(0.3, old_timestamp, 0.0, DEFAULT_WEIGHTS);
        assert!(score2 < 0.4);
    }

    #[test]
    fn test_recency_decay() {
        let now = chrono::Utc::now().timestamp();

        // Recent should score higher
        let recent = calculate_final_score(0.5, now, 0.0, DEFAULT_WEIGHTS);
        let old = calculate_final_score(0.5, now - (180 * 24 * 3600), 0.0, DEFAULT_WEIGHTS);

        assert!(recent > old);
    }
}
