//! R package extractor from source
//!
//! Parses R package source directly from downloaded tarballs without requiring installation.

use crate::fetch::PackageInfo;
use crate::schema::{Example, FunctionRecord, PackageRecord, Record, SCHEMA_VERSION};
use crate::ExtractOptions;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Extract records from an R package source directory
pub fn extract_from_source(pkg: &dyn PackageInfo, options: &ExtractOptions) -> Result<Vec<Record>> {
    let mut records = Vec::new();

    // Parse DESCRIPTION for package metadata
    let (title, description) = parse_description(pkg.source_path())?;

    let pkg_record = PackageRecord {
        schema_version: SCHEMA_VERSION.to_string(),
        name: pkg.name().to_string(),
        version: pkg.version().unwrap_or("unknown").to_string(),
        language: "R".to_string(),
        description: title.or(description),
        llm_hints: Vec::new(),
        common_arguments: BTreeMap::new(),
    };
    records.push(Record::Package(pkg_record));

    // Parse NAMESPACE for exported functions
    let exports = parse_namespace(pkg.source_path())?;

    // Parse Rd files for documentation
    let rd_docs = parse_rd_files(pkg.source_path())?;

    // Parse R files for function signatures
    let functions = parse_r_files(
        pkg.source_path(),
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
    // NAMESPACE might not exist for some packages
    let content = fs::read_to_string(&ns_path).unwrap_or_default();

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
                    .map(ToString::to_string)
                    .unwrap_or_default();

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

    // Collect raw section content first, then parse
    let sections = extract_rd_sections(content);

    // Process each section
    if let Some(title) = sections.get("title") {
        doc.title = Some(sanitize(title));
    }
    if let Some(desc) = sections.get("description") {
        doc.description = Some(sanitize(desc));
    }
    if let Some(value) = sections.get("value") {
        doc.value = Some(sanitize(value));
    }
    if let Some(args) = sections.get("arguments") {
        doc.arguments = parse_arguments_section(args);
    }
    if let Some(examples) = sections.get("examples") {
        doc.examples = parse_example_blocks(examples);
    }

    Ok(doc)
}

/// Extract all sections from Rd content, handling nested braces properly
fn extract_rd_sections(content: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current_section = String::new();
    let mut section_content = String::new();
    let mut brace_depth: isize = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for new section start (only when not already in a section)
        if brace_depth == 0 {
            for section_name in &[
                "title",
                "description",
                "value",
                "arguments",
                "examples",
                "usage",
                "details",
            ] {
                let prefix = format!("\\{section_name}{{");
                if trimmed.starts_with(&prefix) {
                    current_section = (*section_name).to_string();
                    // Get content after the opening brace
                    let after_prefix = trimmed.strip_prefix(&prefix).unwrap_or("");
                    section_content = after_prefix.to_string();
                    brace_depth = 1 + count_braces(after_prefix);
                    break;
                }
            }
        } else {
            // We're inside a section - accumulate content
            if !section_content.is_empty() {
                section_content.push('\n');
            }
            section_content.push_str(trimmed);
            brace_depth += count_braces(trimmed);

            // Section ended
            if brace_depth <= 0 {
                // Remove trailing brace(s)
                let content = section_content.trim_end_matches('}').to_string();
                sections.insert(current_section.clone(), content);
                current_section.clear();
                section_content.clear();
                brace_depth = 0;
            }
        }
    }

    sections
}

/// Parse the \arguments{} section to extract individual argument descriptions
fn parse_arguments_section(content: &str) -> BTreeMap<String, String> {
    let mut arguments = BTreeMap::new();
    let mut current_name = String::new();
    let mut current_desc = String::new();
    let mut in_item = false;
    let mut brace_depth: isize = 0;
    let mut chars = content.chars().peekable();
    let mut buffer = String::new();

    while let Some(c) = chars.next() {
        buffer.push(c);

        // Look for \item{ pattern
        if buffer.ends_with("\\item{") && !in_item {
            buffer.clear();
            // Extract argument name (until closing brace)
            current_name.clear();
            let mut name_brace_depth = 1;
            for nc in chars.by_ref() {
                if nc == '{' {
                    name_brace_depth += 1;
                    current_name.push(nc);
                } else if nc == '}' {
                    name_brace_depth -= 1;
                    if name_brace_depth == 0 {
                        break;
                    }
                    current_name.push(nc);
                } else {
                    current_name.push(nc);
                }
            }

            // Skip opening brace of description
            while let Some(&nc) = chars.peek() {
                if nc == '{' {
                    chars.next();
                    break;
                } else if nc.is_whitespace() || nc == '\n' {
                    chars.next();
                } else {
                    break;
                }
            }

            current_desc.clear();
            brace_depth = 1;
            in_item = true;
            buffer.clear();
        } else if in_item {
            // We're collecting description content
            if c == '{' {
                brace_depth += 1;
                current_desc.push(c);
            } else if c == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // End of this item's description
                    let name = current_name.trim().to_string();
                    let desc = sanitize(current_desc.trim());
                    if !name.is_empty() {
                        arguments.insert(name, desc);
                    }
                    in_item = false;
                    current_name.clear();
                    current_desc.clear();
                    buffer.clear();
                } else {
                    current_desc.push(c);
                }
            } else {
                current_desc.push(c);
            }
        }
    }

    arguments
}

fn count_braces(line: &str) -> isize {
    let open = line.chars().filter(|&c| c == '{').count();
    let close = line.chars().filter(|&c| c == '}').count();
    open as isize - close as isize
}

/// Parse example content into separate code blocks.
/// Strips Rd directives and splits on balanced code blocks.
fn parse_example_blocks(content: &str) -> Vec<String> {
    // First, strip all Rd directives from the content
    let cleaned = strip_rd_directives(content);

    let mut examples = Vec::new();
    let mut current_block = String::new();
    let mut paren_depth: usize = 0;
    let mut in_string = false;
    let mut prev_char = ' ';

    for line in cleaned.lines() {
        let trimmed = line.trim();

        // Skip empty lines between examples, but only if we're not in a function call
        if trimmed.is_empty() {
            if !current_block.is_empty() && paren_depth == 0 {
                let block = current_block.trim().to_string();
                if is_valid_example(&block) {
                    examples.push(block);
                }
                current_block.clear();
            }
            continue;
        }

        // Skip pure comment lines as example separators (but include them in blocks)
        if trimmed.starts_with('#') && current_block.is_empty() {
            continue;
        }

        // Add line to current block
        if !current_block.is_empty() {
            current_block.push('\n');
        }
        current_block.push_str(trimmed);

        // Track parenthesis depth to know when a function call is complete
        for c in trimmed.chars() {
            if c == '"' && prev_char != '\\' {
                in_string = !in_string;
            }
            if !in_string {
                if c == '(' {
                    paren_depth += 1;
                } else if c == ')' {
                    paren_depth = paren_depth.saturating_sub(1);
                }
            }
            prev_char = c;
        }

        // If parentheses are balanced and line doesn't end with continuation,
        // consider the example complete
        if paren_depth == 0 && !trimmed.ends_with(',') && !trimmed.ends_with('(') {
            // Check if next meaningful operation or if this looks complete
            let last_char = trimmed.chars().last().unwrap_or(' ');
            if last_char == ')' || !trimmed.contains('(') {
                let block = current_block.trim().to_string();
                if is_valid_example(&block) {
                    examples.push(block);
                }
                current_block.clear();
            }
        }
    }

    // Don't forget the last block
    if !current_block.is_empty() {
        let block = current_block.trim().to_string();
        if is_valid_example(&block) {
            examples.push(block);
        }
    }

    examples
}

/// Strip Rd directives like \dontrun{}, \donttest{}, \dontshow{} from example content
fn strip_rd_directives(content: &str) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();
    let mut brace_depth_to_skip: usize = 0;

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Check for Rd directives
            let mut directive = String::from(c);
            while let Some(&nc) = chars.peek() {
                if nc.is_alphabetic() {
                    directive.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Check if this is a directive we want to skip
            if matches!(
                directive.as_str(),
                "\\dontrun" | "\\donttest" | "\\dontshow" | "\\donteval"
            ) {
                // Skip the opening brace
                while let Some(&nc) = chars.peek() {
                    if nc == '{' {
                        chars.next();
                        brace_depth_to_skip = 1;
                        break;
                    } else if nc.is_whitespace() {
                        chars.next();
                    } else {
                        break;
                    }
                }
            } else {
                // Not a directive we care about, keep it
                result.push_str(&directive);
            }
        } else if brace_depth_to_skip > 0 {
            // We're inside a directive we're stripping
            if c == '{' {
                brace_depth_to_skip += 1;
            } else if c == '}' {
                brace_depth_to_skip -= 1;
                // When we exit the directive, don't add the closing brace
                if brace_depth_to_skip == 0 {
                    continue;
                }
            }
            // Keep the content inside the directive (just not the directive itself)
            if brace_depth_to_skip > 0 {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if an example block is valid (not just whitespace or stray braces)
fn is_valid_example(block: &str) -> bool {
    let trimmed = block.trim();
    !trimmed.is_empty() && trimmed != "}" && trimmed != "{" && !trimmed.starts_with("\\")
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

/// Remove Rd markup and normalize whitespace
fn sanitize(s: &str) -> String {
    let stripped = strip_rd_markup(s);
    // Normalize whitespace
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip all Rd markup commands, keeping their content where appropriate
fn strip_rd_markup(content: &str) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Collect the command name
            let mut cmd = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_alphabetic() || nc == '_' {
                    cmd.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Determine how to handle this command
            match cmd.as_str() {
                // Commands whose content we want to keep
                "code" | "link" | "pkg" | "emph" | "strong" | "bold" | "sQuote" | "dQuote"
                | "file" | "option" | "var" | "env" | "command" | "dfn" | "cite" | "acronym"
                | "samp" | "kbd" => {
                    // Skip opening brace if present, extract content
                    skip_whitespace(&mut chars);
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        result.push_str(&extract_brace_content_nested(&mut chars));
                    }
                }
                // Commands with [optional]{required} that we want the required part from
                "href" | "url" => {
                    skip_whitespace(&mut chars);
                    // For \href{url}{text}, we want the text; for \url{url}, we want the url
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let first = extract_brace_content_nested(&mut chars);
                        skip_whitespace(&mut chars);
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let second = extract_brace_content_nested(&mut chars);
                            // For \href, second is display text
                            result.push_str(&strip_rd_markup(&second));
                        } else {
                            // For \url, just the URL
                            result.push_str(&first);
                        }
                    }
                }
                // linkS4class, linkS3class - just get the class name
                "linkS4class" | "linkS3class" => {
                    skip_whitespace(&mut chars);
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        result.push_str(&extract_brace_content_nested(&mut chars));
                    }
                }
                // Commands to completely skip (including their content)
                "section" | "subsection" | "seealso" | "author" | "references" | "source"
                | "format" | "note" | "keyword" | "concept" | "alias" | "name" | "docType"
                | "title" | "encoding" | "Rdversion" => {
                    skip_whitespace(&mut chars);
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let _ = extract_brace_content_nested(&mut chars);
                    }
                }
                // \item in arguments - skip the name, keep description
                "item" => {
                    skip_whitespace(&mut chars);
                    // Skip the name part
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let _ = extract_brace_content_nested(&mut chars);
                    }
                    skip_whitespace(&mut chars);
                    // Get the description part
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let desc = extract_brace_content_nested(&mut chars);
                        result.push_str(&strip_rd_markup(&desc));
                        result.push(' ');
                    }
                }
                // \describe, \itemize, \enumerate - process their content
                "describe" | "itemize" | "enumerate" => {
                    skip_whitespace(&mut chars);
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let inner = extract_brace_content_nested(&mut chars);
                        result.push_str(&strip_rd_markup(&inner));
                    }
                }
                // \verb - just get the content (handles \verb{} or \verb|| forms)
                "verb" => {
                    skip_whitespace(&mut chars);
                    if let Some(&delim) = chars.peek() {
                        if delim == '{' {
                            chars.next();
                            result.push_str(&extract_brace_content_nested(&mut chars));
                        } else {
                            // Handle \verb|...|
                            chars.next();
                            for nc in chars.by_ref() {
                                if nc == delim {
                                    break;
                                }
                                result.push(nc);
                            }
                        }
                    }
                }
                // \dots, \ldots -> ...
                "dots" | "ldots" => {
                    result.push_str("...");
                }
                // \R -> R
                "R" => {
                    result.push('R');
                }
                // \cr, \tab -> space
                "cr" | "tab" => {
                    result.push(' ');
                }
                // Unknown command - try to extract content if followed by brace
                _ => {
                    skip_whitespace(&mut chars);
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let inner = extract_brace_content_nested(&mut chars);
                        result.push_str(&strip_rd_markup(&inner));
                    }
                }
            }
        } else if c == '{' || c == '}' {
            // Stray braces - skip them
        } else {
            result.push(c);
        }
    }

    result
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn extract_brace_content_nested(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut content = String::new();
    let mut depth = 1;

    for c in chars.by_ref() {
        if c == '{' {
            depth += 1;
            content.push(c);
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            content.push(c);
        } else {
            content.push(c);
        }
    }

    content
}
