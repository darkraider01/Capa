/// Mathematical operations for similarity search

/// Calculate the mean-centered cosine similarity (Pearson correlation) between two f32 vectors,
/// mapped from the [-1.0, 1.0] range to [0.0, 1.0].
///
/// Returns 0.0 if either vector has a variance of 0 (e.g. uniform/constant vectors).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }

    let mean_a = a.iter().take(len).sum::<f32>() / (len as f32);
    let mean_b = b.iter().take(len).sum::<f32>() / (len as f32);

    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for i in 0..len {
        let val_a = a[i] - mean_a;
        let val_b = b[i] - mean_b;
        dot += val_a * val_b;
        norm_a += val_a * val_a;
        norm_b += val_b * val_b;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        // Fallback to absolute cosine if variance is zero but vectors are identical non-zero
        // e.g., representing single capability profiles
        let mut abs_dot = 0.0;
        let mut abs_norm_a = 0.0;
        let mut abs_norm_b = 0.0;
        for i in 0..len {
            abs_dot += a[i] * b[i];
            abs_norm_a += a[i] * a[i];
            abs_norm_b += b[i] * b[i];
        }
        if abs_norm_a == 0.0 || abs_norm_b == 0.0 {
            return 0.0;
        }
        return abs_dot / (abs_norm_a.sqrt() * abs_norm_b.sqrt());
    }

    let correlation = dot / (norm_a.sqrt() * norm_b.sqrt());
    
    // Pearson falls between -1.0 and 1.0. 
    // We map this linearly to [0.0, 1.0] so it plays nicely with existing logic
    (correlation + 1.0) / 2.0
}

/// Represents a shared capability between two vectors
#[derive(Debug, PartialEq)]
pub struct SharedCapability {
    pub name: String,
    pub overlap_strength: f32,
}

/// Calculate the top shared capabilities between two capability vectors
pub fn calculate_shared_capabilities(
    target_vector: &[f32],
    other_vector: &[f32],
    capability_names: &[&str],
) -> Vec<SharedCapability> {
    let mut overlaps = Vec::new();
    for i in 0..target_vector
        .len()
        .min(other_vector.len())
        .min(capability_names.len())
    {
        let overlap_val =
            target_vector[i].min(other_vector[i]) * (target_vector[i] + other_vector[i]);
        if overlap_val > 0.0 {
            overlaps.push(SharedCapability {
                name: capability_names[i].to_string(),
                overlap_strength: overlap_val,
            });
        }
    }

    overlaps.sort_by(|a, b| {
        b.overlap_strength
            .partial_cmp(&a.overlap_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    overlaps.truncate(3);
    overlaps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = [1.0, 0.0, 1.0, 0.0, 0.0];
        let b = [1.0, 0.0, 1.0, 0.0, 0.0];

        // Identical vectors should have similarity of 1.0
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = [1.0, 0.0, 0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0, 0.0, 0.0];

        // Orthogonal vectors with Pearson mapped to [0,1].
        // Means are 0.2 for both.
        // val_a: [0.8, -0.2, -0.2, -0.2, -0.2]
        // val_b: [-0.2, 0.8, -0.2, -0.2, -0.2]
        // dot = -0.16 + -0.16 + 0.04 + 0.04 + 0.04 = -0.20
        // norm_a = 0.64 + 0.04*4 = 0.80
        // norm_b = 0.80
        // correlation = -0.20 / 0.80 = -0.25
        // Mapped to [0, 1]: (-0.25 + 1) / 2 = 0.375
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.375).abs() < 1e-4);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = [0.0, 0.0, 0.0, 0.0, 0.0];
        let b = [1.0, 1.0, 1.0, 1.0, 1.0];

        // A zero vector should result in 0.0 similarity (avoid NaN)
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_partial_match() {
        let a = [1.0, 1.0, 0.0, 0.0, 0.0]; 
        let b = [1.0, 0.0, 0.0, 0.0, 0.0]; 

        // Pearson correlation between [1,1,0,0,0] and [1,0,0,0,0]
        // mean_a = 0.4. vals: [0.6, 0.6, -0.4, -0.4, -0.4] => norm_a: 1.2
        // mean_b = 0.2. vals: [0.8, -0.2, -0.2, -0.2, -0.2] => norm_b: 0.8
        // dot = (0.6*0.8) + (0.6*-0.2) + 3*(-0.4*-0.2) = 0.48 - 0.12 + 0.24 = 0.6
        // cor = 0.6 / sqrt(1.2 * 0.8) = 0.6 / sqrt(0.96) = 0.6 / 0.97979 = 0.61237
        // mapped: (0.61237 + 1) / 2 = 0.806
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.806186).abs() < 1e-4);
    }

    #[test]
    fn test_low_similarity() {
        // Orthogonal vectors
        let a = [0.9, 0.8, 0.0, 0.0, 0.0];
        let b = [0.0, 0.0, 0.7, 0.6, 0.0];

        // Should mathematically penalize these to <0.5 via Pearson
        let sim = cosine_similarity(&a, &b);
        assert!(sim < 0.5);
    }

    #[test]
    fn test_overlap_stability() {
        let a = [0.9, 0.8, 0.1, 0.0, 0.0];
        let b = [0.85, 0.75, 0.05, 0.0, 0.0];
        let names = ["DS", "CE", "PO", "DB", "API"];

        let shared = calculate_shared_capabilities(&a, &b, &names);

        assert_eq!(shared.len(), 3);
        assert_eq!(shared[0].name, "DS");
        assert_eq!(shared[1].name, "CE");
        assert_eq!(shared[2].name, "PO");

        // Ensure DS overlap is greater than CE overlap
        assert!(shared[0].overlap_strength > shared[1].overlap_strength);
    }
}
