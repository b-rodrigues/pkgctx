//! Compact mode transformations for token-efficient output
//!
//! When --compact is enabled, this module transforms records to minimize
//! token count while preserving essential information for LLM usage.

use crate::schema::{ClassRecord, FunctionRecord, PackageRecord, Record};

/// Apply compact transformations to a list of records.
pub fn compact_records(records: Vec<Record>) -> Vec<Record> {
    records.into_iter().map(compact_record).collect()
}

fn compact_record(record: Record) -> Record {
    match record {
        Record::Package(pkg) => Record::Package(compact_package(pkg)),
        Record::Function(func) => Record::Function(compact_function(func)),
        Record::Class(cls) => Record::Class(compact_class(cls)),
        Record::Workflow(wf) => Record::Workflow(wf), // Keep workflows as-is
    }
}

fn compact_package(mut pkg: PackageRecord) -> PackageRecord {
    // Truncate description to first sentence
    pkg.description = pkg.description.map(truncate_to_sentence);
    pkg
}

fn compact_function(mut func: FunctionRecord) -> FunctionRecord {
    // Truncate purpose to first sentence
    func.purpose = func.purpose.map(truncate_to_sentence);
    
    // Truncate argument descriptions
    func.arguments = func.arguments
        .into_iter()
        .map(|(k, v)| (k, truncate_to_sentence(v)))
        .collect();
    
    // Truncate returns description
    func.returns = func.returns.map(truncate_to_sentence);
    
    // Remove examples in compact mode (signature is usually enough)
    func.examples.clear();
    
    // Remove constraints in compact mode
    func.constraints.clear();
    
    // Remove related functions (can be inferred from names)
    func.related.clear();
    
    func
}

fn compact_class(mut cls: ClassRecord) -> ClassRecord {
    // Truncate method descriptions
    cls.methods = cls.methods
        .into_iter()
        .map(|(k, v)| (k, truncate_to_sentence(v)))
        .collect();
    
    cls
}

/// Truncate a string to the first sentence (ends with . ! or ?).
/// Also limits to ~100 characters if no sentence boundary found.
fn truncate_to_sentence(s: String) -> String {
    // Find first sentence terminator
    let terminators = ['.', '!', '?'];
    
    for (i, c) in s.char_indices() {
        if terminators.contains(&c) {
            // Check if this looks like end of sentence (followed by space or end)
            let next_idx = i + c.len_utf8();
            if next_idx >= s.len() {
                return s[..=i].to_string();
            }
            if let Some(next_char) = s[next_idx..].chars().next() {
                if next_char.is_whitespace() || next_char.is_uppercase() {
                    return s[..=i].to_string();
                }
            }
        }
    }
    
    // No sentence boundary found, truncate at word boundary around 100 chars
    if s.len() > 100 {
        // Find last space before 100 chars
        if let Some(pos) = s[..100].rfind(' ') {
            return format!("{}...", &s[..pos]);
        }
        return format!("{}...", &s[..100]);
    }
    
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_to_sentence() {
        assert_eq!(
            truncate_to_sentence("First sentence. Second sentence.".to_string()),
            "First sentence."
        );
        
        assert_eq!(
            truncate_to_sentence("Short text".to_string()),
            "Short text"
        );
        
        let long = "This is a very long description that goes on and on without any sentence boundaries and just keeps going forever";
        let result = truncate_to_sentence(long.to_string());
        assert!(result.len() < long.len());
        assert!(result.ends_with("..."));
    }
}
