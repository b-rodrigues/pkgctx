//! Hoist common arguments to package level
//!
//! When --hoist-common-args is enabled, this module identifies arguments
//! that appear in many functions and moves their descriptions to the
//! package record's common_arguments field.

use crate::schema::Record;
use std::collections::{BTreeMap, HashMap};

/// Minimum number of occurrences for an argument to be considered "common"
const MIN_OCCURRENCES: usize = 3;

/// Hoist common arguments from function records to the package record.
pub fn hoist_common_args(mut records: Vec<Record>) -> Vec<Record> {
    // Find all arguments and their descriptions across functions
    let mut arg_counts: HashMap<String, usize> = HashMap::new();
    let mut arg_descriptions: HashMap<String, String> = HashMap::new();

    for record in &records {
        if let Record::Function(func) = record {
            for (arg_name, arg_desc) in &func.arguments {
                *arg_counts.entry(arg_name.clone()).or_insert(0) += 1;
                // Keep the first non-empty description
                if !arg_desc.is_empty() {
                    arg_descriptions
                        .entry(arg_name.clone())
                        .or_insert_with(|| arg_desc.clone());
                }
            }
        }
    }

    // Find common arguments (appear >= MIN_OCCURRENCES times)
    let common_args: BTreeMap<String, String> = arg_counts
        .into_iter()
        .filter(|(_, count)| *count >= MIN_OCCURRENCES)
        .filter_map(|(name, _)| arg_descriptions.get(&name).map(|desc| (name, desc.clone())))
        .collect();

    if common_args.is_empty() {
        return records;
    }

    // Update package record with common arguments
    for record in &mut records {
        if let Record::Package(pkg) = record {
            pkg.common_arguments = common_args.clone();
            break;
        }
    }

    // Remove common arguments from individual function records
    for record in &mut records {
        if let Record::Function(func) = record {
            for arg_name in common_args.keys() {
                // Replace detailed description with reference to common_args
                if func.arguments.contains_key(arg_name) {
                    func.arguments
                        .insert(arg_name.clone(), "(see common_arguments)".to_string());
                }
            }
        }
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FunctionRecord, PackageRecord, SCHEMA_VERSION};

    #[test]
    fn test_hoist_common_args() {
        let records = vec![
            Record::Package(PackageRecord {
                schema_version: SCHEMA_VERSION.to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                language: "R".to_string(),
                description: None,
                llm_hints: vec![],
                common_arguments: BTreeMap::new(),
            }),
            Record::Function(FunctionRecord {
                name: "func1".to_string(),
                exported: true,
                signature: "func1(data, x)".to_string(),
                purpose: None,
                role: None,
                arguments: [
                    ("data".to_string(), "A data frame".to_string()),
                    ("x".to_string(), "Column name".to_string()),
                ]
                .into_iter()
                .collect(),
                arg_types: BTreeMap::new(),
                returns: None,
                return_type: None,
                constraints: vec![],
                examples: vec![],
                related: vec![],
            }),
            Record::Function(FunctionRecord {
                name: "func2".to_string(),
                exported: true,
                signature: "func2(data, y)".to_string(),
                purpose: None,
                role: None,
                arguments: [
                    ("data".to_string(), "A data frame".to_string()),
                    ("y".to_string(), "Another column".to_string()),
                ]
                .into_iter()
                .collect(),
                arg_types: BTreeMap::new(),
                returns: None,
                return_type: None,
                constraints: vec![],
                examples: vec![],
                related: vec![],
            }),
            Record::Function(FunctionRecord {
                name: "func3".to_string(),
                exported: true,
                signature: "func3(data, z)".to_string(),
                purpose: None,
                role: None,
                arguments: [
                    ("data".to_string(), "A data frame".to_string()),
                    ("z".to_string(), "Yet another column".to_string()),
                ]
                .into_iter()
                .collect(),
                arg_types: BTreeMap::new(),
                returns: None,
                return_type: None,
                constraints: vec![],
                examples: vec![],
                related: vec![],
            }),
        ];

        let result = hoist_common_args(records);

        // Check package has common_arguments
        if let Record::Package(pkg) = &result[0] {
            assert!(pkg.common_arguments.contains_key("data"));
            assert_eq!(
                pkg.common_arguments.get("data"),
                Some(&"A data frame".to_string())
            );
        } else {
            panic!("Expected Package record");
        }

        // Check functions reference common_arguments
        if let Record::Function(func) = &result[1] {
            assert_eq!(
                func.arguments.get("data"),
                Some(&"(see common_arguments)".to_string())
            );
        }
    }
}
