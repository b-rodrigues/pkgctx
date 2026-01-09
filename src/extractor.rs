//! Language-agnostic extractor trait

use crate::schema::Record;
use crate::ExtractOptions;
use anyhow::Result;

/// Trait for language-specific package extractors
pub trait Extractor {
    /// Extract records from a package
    fn extract(&self, package_name: &str, options: &ExtractOptions) -> Result<Vec<Record>>;
}
