pub mod math;
pub mod search;
pub mod vector_builder;

// Re-export common types and functions for easier use externally
pub use search::{SimilarityResult, find_similar_entities};
pub use vector_builder::{CapabilityVector, store_vector};
