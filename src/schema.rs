//! YAML output schema types for pkgctx
//!
//! Defines the record types that match the v1.1 schema specification.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A record in the pkgctx output stream.
/// Each record is self-describing via its `kind` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Record {
    Package(PackageRecord),
    Function(FunctionRecord),
    Class(ClassRecord),
    Workflow(WorkflowRecord),
}

/// Package metadata record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRecord {
    /// Schema version for forward compatibility
    pub schema_version: String,

    /// Package name
    pub name: String,

    /// Package version
    pub version: String,

    /// Source language (R or Python)
    pub language: String,

    /// Brief description optimized for LLM context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Hints for LLM on how to use this package
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub llm_hints: Vec<String>,

    /// Common arguments shared across many functions
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub common_arguments: BTreeMap<String, String>,
}

/// Function/method record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRecord {
    /// Function name
    pub name: String,

    /// Whether this function is exported (public API)
    pub exported: bool,

    /// Full function signature
    pub signature: String,

    /// One-line description of what the function does
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,

    /// Function role: transformer, constructor, predicate, accessor, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Argument descriptions (name -> description)
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub arguments: BTreeMap<String, String>,

    /// Argument types for light symbolic typing (name -> type)
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub arg_types: BTreeMap<String, String>,

    /// Description of return value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,

    /// Return type for light symbolic typing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,

    /// Constraints or requirements for using this function
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub constraints: Vec<String>,

    /// Usage examples
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub examples: Vec<Example>,

    /// Related functions
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub related: Vec<String>,
}

/// A code example with optional annotations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Example {
    /// The example code
    pub code: String,

    /// What this example demonstrates
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub shows: Vec<String>,
}

/// Class record for OOP constructs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassRecord {
    /// Class name
    pub name: String,

    /// Functions that construct instances of this class
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub constructed_by: Vec<String>,

    /// Methods available on instances (name -> description)
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub methods: BTreeMap<String, String>,
}

/// Workflow record showing canonical usage patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRecord {
    /// Workflow name
    pub name: String,

    /// Steps in the workflow (code snippets)
    pub steps: Vec<String>,

    /// What this workflow accomplishes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Current schema version
pub const SCHEMA_VERSION: &str = "1.1";
