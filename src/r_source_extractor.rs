//! R package extractor from source
//!
//! Parses R package source directly from downloaded tarballs without requiring installation.

use crate::fetch::FetchedPackage;
use crate::schema::{Example, FunctionRecord, PackageRecord, Record, SCHEMA_VERSION};
use crate::ExtractOptions;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Extract records from an R package source directory
pub fn extract_from_source(pkg: &FetchedPackage, options: &ExtractOptions) -> Result<Vec<Record>> {
    let mut records = Vec::new();

    // Parse DESCRIPTION for package metadata
    let (title, description) = parse_description(&pkg.source_path)?;

    let pkg_record = PackageRecord {
        schema_version: SCHEMA_VERSION.to_string(),
        name: pkg.name.clone(),
        version: pkg.version.clone().unwrap_or_else(|| "unknown".to_string()),
        language: "R".to_string(),
        description: title.or(description),
        llm_hints: Vec::new(),
        common_arguments: BTreeMap::new(),
    };
    records.push(Record::Package(pkg_record));

    // Parse NAMESPACE for exported functions
    let exports = parse_namespace(&pkg.source_path)?;

    // Parse Rd files for documentation
    let rd_docs = parse_rd_files(&pkg.source_path)?;

    // Parse R files for function signatures
    let functions = parse_r_files(
        &pkg.source_path,
        &exports,
        &rd_docs,
        options.include_internal,
    )?;

    for func in functions {
        records.push(Record::Function(func));
    }

    Ok(records)
}

/// Parse DESCRIPTION file for title and description
fn parse_description(path: &Path) -> Result<(Option<String>, Option<String>)> {
    let desc_path = path.join("DESCRIPTION");
    let content = fs::read_to_string(&desc_path).context("Failed to read DESCRIPTION file")?;

    let mut title = None;
    let mut description = None;
    let mut in_description = false;
    let mut desc_lines = Vec::new();

    for line in content.lines() {
        if let Some(t) = line.strip_prefix("Title:") {
            title = Some(sanitize(t.trim()));
            in_description = false;
        } else if let Some(d) = line.strip_prefix("Description:") {
            desc_lines.push(d.trim().to_string());
            in_description = true;
        } else if in_description {
            if line.starts_with(' ') || line.starts_with('\t') {
                desc_lines.push(line.trim().to_string());
            } else {
                in_description = false;
            }
        }
    }

    if !desc_lines.is_empty() {
        description = Some(sanitize(&desc_lines.join(" ")));
    }

    Ok((title, description))
}

/// Parse NAMESPACE file for exported functions
fn parse_namespace(path: &Path) -> Result<Vec<String>> {
    let ns_path = path.join("NAMESPACE");
    let content = fs::read_to_string(&ns_path).unwrap_or_default(); // NAMESPACE might not exist

    let mut exports = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        // export(func1, func2, ...)
        if let Some(rest) = line.strip_prefix("export(") {
            if let Some(inner) = rest.strip_suffix(')') {
                for name in inner.split(',') {
                    exports.push(name.trim().to_string());
                }
            }
        }
        // exportPattern("^[^.]") - export all non-dot functions
        // S3method(generic, class) - export S3 method
    }

    Ok(exports)
}

/// Parsed Rd documentation
struct RdDoc {
    title: Option<String>,
    description: Option<String>,
    arguments: BTreeMap<String, String>,
    value: Option<String>,
    examples: Vec<String>,
}

/// Parse all Rd files in man/ directory
fn parse_rd_files(path: &Path) -> Result<BTreeMap<String, RdDoc>> {
    let mut docs = BTreeMap::new();

    let man_path = path.join("man");
    if !man_path.exists() {
        return Ok(docs);
    }

    for entry in fs::read_dir(&man_path)? {
        let entry = entry?;
        let file_path = entry.path();

        if file_path.extension().is_some_and(|e| e == "Rd") {
            if let Ok(content) = fs::read_to_string(&file_path) {
                let name = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                if let Ok(doc) = parse_rd_content(&content) {
                    docs.insert(name, doc);
                }
            }
        }
    }

    Ok(docs)
}

/// Parse Rd file content
fn parse_rd_content(content: &str) -> Result<RdDoc> {
    let mut doc = RdDoc {
        title: None,
        description: None,
        arguments: BTreeMap::new(),
        value: None,
        examples: Vec::new(),
    };

    // Simple Rd parser - extract sections
    let mut current_section = String::new();
    let mut section_content = String::new();
    let mut brace_depth = 0;

    for line in content.lines() {
        let line = line.trim();

        // Check for section start
        if line.starts_with("\\title{") {
            current_section = "title".to_string();
            section_content = extract_brace_content(line, "\\title{");
            brace_depth = count_braces(line);
        } else if line.starts_with("\\description{") {
            current_section = "description".to_string();
            section_content = extract_brace_content(line, "\\description{");
            brace_depth = count_braces(line);
        } else if line.starts_with("\\value{") {
            current_section = "value".to_string();
            section_content = extract_brace_content(line, "\\value{");
            brace_depth = count_braces(line);
        } else if line.starts_with("\\arguments{") {
            current_section = "arguments".to_string();
            brace_depth = 1;
        } else if line.starts_with("\\examples{") {
            current_section = "examples".to_string();
            brace_depth = 1;
        } else if current_section == "arguments" && line.starts_with("\\item{") {
            // Parse argument: \item{name}{description}
            if let Some((name, desc)) = parse_item(line) {
                doc.arguments.insert(name, sanitize(&desc));
            }
        } else if brace_depth > 0 {
            brace_depth += line.chars().filter(|c| *c == '{').count() as i32;
            brace_depth -= line.chars().filter(|c| *c == '}').count() as i32;

            if current_section == "examples" && !line.starts_with('%') {
                let ex_line = line.trim_start_matches("\\dontrun{").trim_end_matches('}');
                if !ex_line.is_empty() {
                    doc.examples.push(sanitize(ex_line));
                }
            } else if !current_section.is_empty() && current_section != "arguments" {
                section_content.push(' ');
                section_content.push_str(line);
            }

            if brace_depth == 0 {
                match current_section.as_str() {
                    "title" => doc.title = Some(sanitize(&section_content)),
                    "description" => doc.description = Some(sanitize(&section_content)),
                    "value" => doc.value = Some(sanitize(&section_content)),
                    _ => {}
                }
                current_section.clear();
                section_content.clear();
            }
        }
    }

    Ok(doc)
}

fn extract_brace_content(line: &str, prefix: &str) -> String {
    line.strip_prefix(prefix)
        .unwrap_or("")
        .trim_end_matches('}')
        .to_string()
}

fn count_braces(line: &str) -> i32 {
    line.chars().filter(|c| *c == '{').count() as i32
        - line.chars().filter(|c| *c == '}').count() as i32
}

fn parse_item(line: &str) -> Option<(String, String)> {
    // \item{name}{description}
    let rest = line.strip_prefix("\\item{")?;
    let close_pos = rest.find('}')?;
    let name = rest[..close_pos].to_string();
    let rest = &rest[close_pos + 1..];
    let desc = rest
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_string();
    Some((name, desc))
}

/// Parse R files for function definitions
fn parse_r_files(
    path: &Path,
    exports: &[String],
    rd_docs: &BTreeMap<String, RdDoc>,
    include_internal: bool,
) -> Result<Vec<FunctionRecord>> {
    let mut functions = Vec::new();

    let r_path = path.join("R");
    if !r_path.exists() {
        return Ok(functions);
    }

    for entry in fs::read_dir(&r_path)? {
        let entry = entry?;
        let file_path = entry.path();

        if file_path.extension().is_some_and(|e| e == "R" || e == "r") {
            if let Ok(content) = fs::read_to_string(&file_path) {
                let file_funcs =
                    extract_functions_from_r(&content, exports, rd_docs, include_internal);
                functions.extend(file_funcs);
            }
        }
    }

    Ok(functions)
}

/// Extract function definitions from R source code
fn extract_functions_from_r(
    content: &str,
    exports: &[String],
    rd_docs: &BTreeMap<String, RdDoc>,
    include_internal: bool,
) -> Vec<FunctionRecord> {
    let mut functions = Vec::new();

    // Simple regex-like parsing for: name <- function(args) or name = function(args)
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let line = line.trim();

        // Skip comments
        if line.starts_with('#') {
            continue;
        }

        // Look for function assignment patterns
        let patterns = ["<- function(", "= function(", "<-function(", "=function("];

        for pattern in patterns {
            if let Some(pos) = line.find(pattern) {
                let name = line[..pos].trim().to_string();
                if name.is_empty() || name.contains(' ') {
                    continue;
                }

                let exported = exports.contains(&name) || exports.is_empty();

                // Skip internal functions unless requested
                if !include_internal && name.starts_with('.') {
                    continue;
                }
                if !include_internal && !exported {
                    continue;
                }

                // Extract signature (may span multiple lines)
                let mut sig_content = line[pos + pattern.len()..].to_string();
                let mut paren_depth = 1;
                let mut j = i;

                while paren_depth > 0 && j < lines.len() {
                    for c in sig_content.chars() {
                        if c == '(' {
                            paren_depth += 1;
                        }
                        if c == ')' {
                            paren_depth -= 1;
                        }
                    }
                    if paren_depth > 0 {
                        j += 1;
                        if j < lines.len() {
                            sig_content.push_str(lines[j].trim());
                        }
                    }
                }

                // Build signature
                let args_end = sig_content.rfind(')').unwrap_or(sig_content.len());
                let args = &sig_content[..args_end];
                let signature = format!("{}({})", name, args.trim());

                // Get documentation
                let doc = rd_docs.get(&name);

                let mut arguments = BTreeMap::new();
                if let Some(d) = doc {
                    arguments = d.arguments.clone();
                }

                let examples: Vec<Example> = doc
                    .map(|d| {
                        d.examples
                            .iter()
                            .take(3)
                            .map(|e| Example {
                                code: e.clone(),
                                shows: Vec::new(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let func = FunctionRecord {
                    name: name.clone(),
                    exported,
                    signature,
                    purpose: doc.and_then(|d| d.title.clone()),
                    role: None,
                    arguments,
                    arg_types: BTreeMap::new(),
                    returns: doc.and_then(|d| d.value.clone()),
                    return_type: None,
                    constraints: Vec::new(),
                    examples,
                    related: Vec::new(),
                };

                functions.push(func);
                break;
            }
        }
    }

    functions
}

fn sanitize(s: &str) -> String {
    // Remove Rd markup and normalize whitespace
    let s = s
        .replace("\\code{", "")
        .replace("\\link{", "")
        .replace("\\pkg{", "")
        .replace("\\emph{", "")
        .replace("\\strong{", "")
        .replace("}", "")
        .replace("\\n", " ")
        .replace('\n', " ");

    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.trim().to_string()
}
