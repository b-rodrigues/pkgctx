//! pkgctx — compile software packages into LLM-ready context
//!
//! Extracts structured, compact API specifications from R or Python packages
//! for use in LLMs, minimizing tokens while maximizing context.

mod compact;
mod fetch;
mod hoist;
mod python_source_extractor;
mod r_source_extractor;
mod schema;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use fetch::PackageInfo;
use std::io::{self, Write};

/// Compile software packages into LLM-ready context
#[derive(Parser)]
#[command(name = "pkgctx")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract context from an R package (CRAN, GitHub, or local path)
    R {
        /// Package specifier: name (CRAN), `github:owner/repo[@ref]`, or local path (., ./path, /path)
        package: String,

        #[command(flatten)]
        options: ExtractOptions,
    },

    /// Extract context from a Python package (`PyPI`, GitHub, or local path)
    Python {
        /// Package specifier: name (`PyPI`), `github:owner/repo[@ref]`, or local path (., ./path, /path)
        package: String,

        #[command(flatten)]
        options: ExtractOptions,
    },
}

#[derive(Parser, Clone)]
pub struct ExtractOptions {
    /// Output format
    #[arg(long, default_value = "yaml", value_enum)]
    format: OutputFormat,

    /// Aggressively minimize token count
    #[arg(long)]
    pub compact: bool,

    /// Include non-exported/internal functions
    #[arg(long)]
    pub include_internal: bool,

    /// Include class specifications
    #[arg(long)]
    pub emit_classes: bool,

    /// Include canonical workflows
    #[arg(long)]
    pub emit_workflows: bool,

    /// Extract frequently used arguments to package-level common_args
    #[arg(long)]
    pub hoist_common_args: bool,

    /// Omit the LLM instructions header from output
    #[arg(long)]
    pub no_header: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Yaml,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::R { package, options } => process_r_package(&package, &options),
        Commands::Python { package, options } => process_python_package(&package, &options),
    }
}

/// Process an R package from any source
fn process_r_package(package: &str, options: &ExtractOptions) -> Result<()> {
    eprintln!("Fetching R package: {package}");

    let source = fetch::PackageSource::parse(package, "r")?;

    match source {
        fetch::PackageSource::Cran(name) => {
            eprintln!("  → Downloading from CRAN...");
            let pkg = fetch::fetch_cran_package(&name)?;
            process_package(&pkg, options, "R")
        }
        fetch::PackageSource::Bioconductor(name) => {
            eprintln!("  → Downloading from Bioconductor...");
            let pkg = fetch::fetch_bioconductor_package(&name)?;
            process_package(&pkg, options, "R")
        }
        fetch::PackageSource::GitHub { owner, repo, ref_ } => {
            eprintln!("  → Downloading from GitHub: {owner}/{repo}...");
            let pkg = fetch::fetch_github_r_package(&owner, &repo, ref_.as_deref())?;
            process_package(&pkg, options, "R")
        }
        fetch::PackageSource::Local(path) => {
            eprintln!("  → Using local path: {}...", path.display());
            let pkg = fetch::fetch_local_r_package(&path)?;
            process_package(&pkg, options, "R")
        }
        fetch::PackageSource::PyPI(_) => {
            anyhow::bail!("PyPI source is not valid for R packages")
        }
    }
}

/// Process a Python package from any source
fn process_python_package(package: &str, options: &ExtractOptions) -> Result<()> {
    eprintln!("Fetching Python package: {package}");

    let source = fetch::PackageSource::parse(package, "python")?;

    match source {
        fetch::PackageSource::PyPI(name) => {
            eprintln!("  → Downloading from PyPI...");
            let pkg = fetch::fetch_pypi_package(&name)?;
            process_package(&pkg, options, "Python")
        }
        fetch::PackageSource::GitHub { owner, repo, ref_ } => {
            eprintln!("  → Downloading from GitHub: {owner}/{repo}...");
            let pkg = fetch::fetch_github_python_package(&owner, &repo, ref_.as_deref())?;
            process_package(&pkg, options, "Python")
        }
        fetch::PackageSource::Local(path) => {
            eprintln!("  → Using local path: {}...", path.display());
            let pkg = fetch::fetch_local_python_package(&path)?;
            process_package(&pkg, options, "Python")
        }
        fetch::PackageSource::Cran(_) | fetch::PackageSource::Bioconductor(_) => {
            anyhow::bail!("CRAN/Bioconductor source is not valid for Python packages")
        }
    }
}

/// Common processing logic for any package type
fn process_package(pkg: &dyn PackageInfo, options: &ExtractOptions, language: &str) -> Result<()> {
    eprintln!("  → Version: {}", pkg.version().unwrap_or("unknown"));
    eprintln!("  → Parsing source...");

    let records = match language {
        "R" => r_source_extractor::extract_from_source(pkg, options)?,
        "Python" => python_source_extractor::extract_from_source(pkg, options)?,
        _ => anyhow::bail!("Unknown language: {language}"),
    };

    let records = apply_transformations(records, options);
    output_records(
        &records,
        options.format,
        pkg.name(),
        language,
        options.no_header,
    )
}

/// Apply post-extraction transformations based on options.
fn apply_transformations(
    records: Vec<schema::Record>,
    options: &ExtractOptions,
) -> Vec<schema::Record> {
    let records = if options.hoist_common_args {
        hoist::hoist_common_args(records)
    } else {
        records
    };

    if options.compact {
        compact::compact_records(records)
    } else {
        records
    }
}

/// Output records to stdout in the specified format
fn output_records(
    records: &[schema::Record],
    format: OutputFormat,
    pkg_name: &str,
    language: &str,
    no_header: bool,
) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    // Create context header if not disabled
    let header = if no_header {
        None
    } else {
        Some(schema::Record::ContextHeader(schema::ContextHeaderRecord {
            llm_instructions: generate_llm_instructions(pkg_name, language),
        }))
    };

    match format {
        OutputFormat::Yaml => write_yaml(&mut handle, header.as_ref(), records)?,
        OutputFormat::Json => write_json(&mut handle, header.as_ref(), records)?,
    }

    Ok(())
}

/// Write records as YAML
fn write_yaml(
    handle: &mut impl Write,
    header: Option<&schema::Record>,
    records: &[schema::Record],
) -> Result<()> {
    if let Some(h) = header {
        writeln!(handle, "---")?;
        write!(handle, "{}", serde_yaml::to_string(h)?)?;
    }
    for record in records {
        writeln!(handle, "---")?;
        write!(handle, "{}", serde_yaml::to_string(record)?)?;
    }
    Ok(())
}

/// Write records as JSON
fn write_json(
    handle: &mut impl Write,
    header: Option<&schema::Record>,
    records: &[schema::Record],
) -> Result<()> {
    if let Some(h) = header {
        writeln!(handle, "{}", serde_json::to_string_pretty(h)?)?;
    }
    for record in records {
        writeln!(handle, "{}", serde_json::to_string_pretty(record)?)?;
    }
    Ok(())
}

/// Generate LLM instructions for the context header
fn generate_llm_instructions(pkg_name: &str, language: &str) -> String {
    format!(
        "This is an LLM-optimized API specification for the {language} package '{pkg_name}'. \
Use this context to write correct code using {pkg_name} functions. \
Each 'function' record describes a public function with its signature, arguments, and purpose. \
The 'package' record contains metadata. \
All listed functions are part of the public API."
    )
}
