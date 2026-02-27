use crate::extraction::models::ExtractedCapability;
use anyhow::Result;
use std::path::Path;
use tantivy::{Index, IndexWriter, doc, schema::Schema};

/// Capability search index
pub struct CapabilityIndex {
    pub index: Index,
    pub schema: Schema,
}

impl CapabilityIndex {
    /// Create a new index at the specified path
    pub fn create<P: AsRef<Path>>(index_path: P) -> Result<Self> {
        let schema = super::build_capability_schema();

        // Try to open first, if fails try to create
        let index = match Index::open_in_dir(&index_path) {
            Ok(idx) => idx,
            Err(_) => {
                // Ensure directory exists
                std::fs::create_dir_all(&index_path)?;
                Index::create_in_dir(index_path, schema.clone())?
            }
        };

        Ok(Self { index, schema })
    }

    /// Open an existing index
    pub fn open<P: AsRef<Path>>(index_path: P) -> Result<Self> {
        let index = Index::open_in_dir(index_path)?;
        let schema = index.schema();

        Ok(Self { index, schema })
    }

    /// Get an index writer with 50MB heap
    pub fn get_writer(&self) -> Result<IndexWriter> {
        Ok(self.index.writer(50_000_000)?)
    }
}

/// Index capabilities into Tantivy
pub fn index_capabilities(
    writer: &mut IndexWriter,
    capabilities: &[ExtractedCapability],
    schema: &Schema,
) -> Result<()> {
    let entity_id = schema.get_field("entity_id")?;
    let capability_type = schema.get_field("capability_type")?;
    let tier = schema.get_field("tier")?;
    let confidence = schema.get_field("confidence")?;
    let normalized_score = schema.get_field("normalized_score")?;
    let timestamp = schema.get_field("timestamp")?;
    let evidence_keywords = schema.get_field("evidence_keywords")?;
    let evidence_repos = schema.get_field("evidence_repos")?;

    for cap in capabilities {
        writer.add_document(doc!(
            entity_id => cap.user_login.clone(),
            capability_type => cap.capability_type.as_str(),
            tier => cap.tier.as_str(),
            confidence => cap.confidence as f64,
            normalized_score => cap.normalized_score as f64,
            timestamp => cap.timestamp,
            evidence_keywords => cap.evidence_keywords.join(" "),
            evidence_repos => cap.evidence_repos.join(" "),
        ))?;
    }

    writer.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_creation() {
        let temp_dir = TempDir::new().unwrap();
        let index = CapabilityIndex::create(temp_dir.path()).unwrap();

        assert!(index.schema.get_field("entity_id").is_ok());
    }
}
