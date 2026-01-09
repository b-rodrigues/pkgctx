//! R package extractor
//!
//! Extracts API information from R packages by spawning an R subprocess
//! and using introspection functions.

use crate::extractor::Extractor;
use crate::schema::{
    Example, FunctionRecord, PackageRecord, Record, SCHEMA_VERSION,
};
use crate::ExtractOptions;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Command;

/// Extractor for R packages
pub struct RExtractor {
    r_executable: String,
}

/// Raw function info from R introspection script
#[derive(Debug, Deserialize)]
struct RFunctionInfo {
    name: String,
    exported: bool,
    signature: String,
    #[serde(default)]
    arguments: Vec<RArgument>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    returns: Option<String>,
    #[serde(default)]
    examples: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RArgument {
    name: String,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Raw package info from R introspection script
#[derive(Debug, Deserialize)]
struct RPackageInfo {
    name: String,
    version: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    functions: Vec<RFunctionInfo>,
}

impl RExtractor {
    /// Create a new R extractor, locating the R executable.
    pub fn new() -> Result<Self> {
        // Try to find R in PATH
        let r_executable = which::which("R")
            .context("R executable not found in PATH. Make sure R is available.")?
            .to_string_lossy()
            .to_string();

        Ok(Self { r_executable })
    }

    /// Run the R introspection script and parse its JSON output.
    fn run_introspection(&self, package_name: &str, include_internal: bool) -> Result<RPackageInfo> {
        let r_script = self.generate_introspection_script(package_name, include_internal);

        // Use Rscript for cleaner output (no prompts)
        let output = Command::new("Rscript")
            .args(["--vanilla", "-e", &r_script])
            .output()
            .context("Failed to execute Rscript")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("R introspection failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Find the JSON output (between markers)
        let json_start = stdout
            .find("<<<PKGCTX_JSON_START>>>")
            .context("Could not find JSON start marker in R output")?;
        let json_end = stdout
            .find("<<<PKGCTX_JSON_END>>>")
            .context("Could not find JSON end marker in R output")?;

        let json_str = &stdout[json_start + 23..json_end];

        serde_json::from_str(json_str)
            .with_context(|| format!("Failed to parse R introspection JSON. Raw output length: {} bytes", json_str.len()))
    }

    /// Generate the R script that introspects a package.
    fn generate_introspection_script(&self, package_name: &str, include_internal: bool) -> String {
        format!(r#"
suppressPackageStartupMessages({{
  library(jsonlite)
}})

# Helper to sanitize strings for JSON output
sanitize_string <- function(x) {{
  if (is.null(x) || length(x) == 0) return(NULL)
  x <- as.character(x)
  # Remove control characters (except newline, tab, carriage return)
  x <- gsub("[[:cntrl:]]", " ", x)
  # Collapse multiple whitespace
  x <- gsub("\\\\s+", " ", x)
  trimws(x)
}}

introspect_package <- function(pkg_name, include_internal = FALSE) {{
  # Load the package namespace
  if (!requireNamespace(pkg_name, quietly = TRUE)) {{
    stop(paste("Package", pkg_name, "not found"))
  }}
  
  ns <- asNamespace(pkg_name)
  
  # Get package description
  pkg_desc <- packageDescription(pkg_name)
  
  # Get exported functions
  exports <- getNamespaceExports(pkg_name)
  
  # Get all functions
  if (include_internal) {{
    all_names <- ls(ns, all.names = TRUE)
  }} else {{
    all_names <- exports
  }}
  
  # Filter to only functions
  is_func <- sapply(all_names, function(n) {{
    tryCatch({{
      obj <- get(n, envir = ns)
      is.function(obj)
    }}, error = function(e) FALSE)
  }})
  func_names <- all_names[is_func]
  
  # Get Rd database for documentation
  rd_db <- tryCatch({{
    tools::Rd_db(pkg_name)
  }}, error = function(e) NULL)
  
  # Parse Rd to extract info
  parse_rd <- function(rd) {{
    if (is.null(rd)) return(list())
    
    result <- list(
      title = NULL,
      description = NULL,
      arguments = list(),
      value = NULL,
      examples = NULL
    )
    
    for (section in rd) {{
      tag <- attr(section, "Rd_tag")
      if (is.null(tag)) next
      
      content <- sanitize_string(paste(unlist(section), collapse = ""))
      
      if (tag == "\\title") {{
        result$title <- content
      }} else if (tag == "\\description") {{
        result$description <- content
      }} else if (tag == "\\value") {{
        result$value <- content
      }} else if (tag == "\\arguments") {{
        # Parse arguments
        for (item in section) {{
          item_tag <- attr(item, "Rd_tag")
          if (!is.null(item_tag) && item_tag == "\\item") {{
            if (length(item) >= 2) {{
              arg_name <- paste(unlist(item[[1]]), collapse = "")
              arg_desc <- sanitize_string(paste(unlist(item[[2]]), collapse = ""))
              result$arguments[[arg_name]] <- arg_desc
            }}
          }}
        }}
      }} else if (tag == "\\examples") {{
        result$examples <- content
      }}
    }}
    
    result
  }}
  
  # Find Rd for a function
  find_rd <- function(func_name) {{
    if (is.null(rd_db)) return(NULL)
    
    # Try exact match first
    if (func_name %in% names(rd_db)) {{
      return(rd_db[[func_name]])
    }}
    
    # Try with .Rd extension
    rd_name <- paste0(func_name, ".Rd")
    if (rd_name %in% names(rd_db)) {{
      return(rd_db[[rd_name]])
    }}
    
    NULL
  }}
  
  # Build function info
  functions <- lapply(func_names, function(fn) {{
    func <- get(fn, envir = ns)
    formals_list <- formals(func)
    
    # Build signature
    if (length(formals_list) == 0) {{
      sig <- paste0(fn, "()")
    }} else {{
      args <- sapply(names(formals_list), function(arg_name) {{
        default_val <- formals_list[[arg_name]]
        if (missing(default_val) || identical(default_val, quote(expr = ))) {{
          arg_name
        }} else {{
          paste0(arg_name, " = ", deparse(default_val, width.cutoff = 500L))
        }}
      }})
      sig <- paste0(fn, "(", paste(args, collapse = ", "), ")")
    }}
    
    # Get documentation
    rd <- find_rd(fn)
    rd_info <- parse_rd(rd)
    
    # Build arguments list
    arguments <- lapply(names(formals_list), function(arg_name) {{
      default_val <- formals_list[[arg_name]]
      default_str <- if (missing(default_val) || identical(default_val, quote(expr = ))) {{
        NULL
      }} else {{
        deparse(default_val, width.cutoff = 500L)
      }}
      
      list(
        name = arg_name,
        default = default_str,
        description = rd_info$arguments[[arg_name]]
      )
    }})
    
    # Parse examples
    examples <- if (!is.null(rd_info$examples) && nchar(rd_info$examples) > 0) {{
      # Split by newline, filter empty, take first few
      ex_lines <- strsplit(rd_info$examples, "\n")[[1]]
      ex_lines <- vapply(ex_lines, sanitize_string, character(1), USE.NAMES = FALSE)
      ex_lines <- ex_lines[!is.na(ex_lines) & nchar(ex_lines) > 0]
      ex_lines <- ex_lines[!grepl("^#", ex_lines)]  # Remove comments
      as.list(head(ex_lines, 3))  # Return as list for JSON array
    }} else {{
      list()  # Empty list ensures JSON array []
    }}
    
    list(
      name = fn,
      exported = fn %in% exports,
      signature = sig,
      arguments = arguments,
      title = rd_info$title,
      description = rd_info$description,
      returns = rd_info$value,
      examples = examples
    )
  }})
  
  list(
    name = pkg_name,
    version = as.character(pkg_desc$Version),
    title = sanitize_string(pkg_desc$Title),
    description = sanitize_string(pkg_desc$Description),
    functions = functions
  )
}}

result <- introspect_package("{pkg_name}", {include_internal})
cat("<<<PKGCTX_JSON_START>>>")
cat(toJSON(result, auto_unbox = TRUE, null = "null"))
cat("<<<PKGCTX_JSON_END>>>")
"#,
        pkg_name = package_name,
        include_internal = if include_internal { "TRUE" } else { "FALSE" }
        )
    }
}

impl Extractor for RExtractor {
    fn extract(&self, package_name: &str, options: &ExtractOptions) -> Result<Vec<Record>> {
        let pkg_info = self.run_introspection(package_name, options.include_internal)?;

        let mut records = Vec::new();

        // Create package record
        let pkg_record = PackageRecord {
            schema_version: SCHEMA_VERSION.to_string(),
            name: pkg_info.name.clone(),
            version: pkg_info.version,
            language: "R".to_string(),
            description: pkg_info.title.or(pkg_info.description),
            llm_hints: Vec::new(),
            common_arguments: BTreeMap::new(),
        };
        records.push(Record::Package(pkg_record));

        // Create function records
        for func in pkg_info.functions {
            let mut arguments = BTreeMap::new();
            for arg in &func.arguments {
                let desc = arg.description.clone().unwrap_or_else(|| {
                    arg.default
                        .as_ref()
                        .map(|d| format!("default: {}", d))
                        .unwrap_or_default()
                });
                if !desc.is_empty() {
                    arguments.insert(arg.name.clone(), desc);
                }
            }

            let examples: Vec<Example> = func
                .examples
                .into_iter()
                .map(|code| Example {
                    code,
                    shows: Vec::new(),
                })
                .collect();

            let func_record = FunctionRecord {
                name: func.name,
                exported: func.exported,
                signature: func.signature,
                purpose: func.title,
                role: None,
                arguments,
                arg_types: BTreeMap::new(),
                returns: func.returns,
                return_type: None,
                constraints: Vec::new(),
                examples,
                related: Vec::new(),
            };
            records.push(Record::Function(func_record));
        }

        Ok(records)
    }
}
