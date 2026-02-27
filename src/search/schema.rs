use tantivy::schema::*;

/// Build the Tantivy schema for capability indexing
pub fn build_capability_schema() -> Schema {
    let mut schema_builder = Schema::builder();

    // Entity and capability identification (exact match)
    schema_builder.add_text_field("entity_id", STRING | STORED);
    schema_builder.add_text_field("capability_type", STRING | STORED);
    schema_builder.add_text_field("tier", STRING | STORED);

    // Scoring fields (FAST for sorting/filtering)
    schema_builder.add_f64_field("confidence", FAST | STORED);
    schema_builder.add_f64_field("normalized_score", FAST | STORED);
    schema_builder.add_i64_field("timestamp", FAST | STORED);

    // Full-text searchable fields
    schema_builder.add_text_field("evidence_keywords", TEXT | STORED);
    schema_builder.add_text_field("evidence_repos", TEXT | STORED);

    schema_builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_creation() {
        let schema = build_capability_schema();

        // Verify all fields exist
        assert!(schema.get_field("entity_id").is_ok());
        assert!(schema.get_field("capability_type").is_ok());
        assert!(schema.get_field("tier").is_ok());
        assert!(schema.get_field("confidence").is_ok());
        assert!(schema.get_field("normalized_score").is_ok());
        assert!(schema.get_field("timestamp").is_ok());
        assert!(schema.get_field("evidence_keywords").is_ok());
        assert!(schema.get_field("evidence_repos").is_ok());
    }
}
