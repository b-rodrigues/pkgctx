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
        /// Package specifier: name (CRAN), github:owner/repo[@ref], or local path (., ./path, /path)
        package: String,

        #[command(flatten)]
        options: ExtractOptions,
    },

    /// Extract context from a Python package (PyPI, GitHub, or local path)
    Python {
        /// Package specifier: name (PyPI), github:owner/repo[@ref], or local path (., ./path, /path)
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
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Yaml,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::R { package, options } => {
            eprintln!("Fetching R package: {}", package);

            let source = fetch::PackageSource::parse(&package, "r")?;

            match source {
                fetch::PackageSource::Cran(name) => {
                    eprintln!("  → Downloading from CRAN...");
                    let pkg = fetch::fetch_cran_package(&name)?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = r_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                fetch::PackageSource::GitHub { owner, repo, ref_ } => {
                    eprintln!("  → Downloading from GitHub: {}/{}...", owner, repo);
                    let pkg = fetch::fetch_github_r_package(&owner, &repo, ref_.as_deref())?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = r_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                fetch::PackageSource::Local(path) => {
                    eprintln!("  → Using local path: {}...", path.display());
                    let pkg = fetch::fetch_local_r_package(&path)?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = r_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                _ => anyhow::bail!("Invalid source for R package"),
            };
        }
        Commands::Python { package, options } => {
            eprintln!("Fetching Python package: {}", package);

            let source = fetch::PackageSource::parse(&package, "python")?;

            match source {
                fetch::PackageSource::PyPI(name) => {
                    eprintln!("  → Downloading from PyPI...");
                    let pkg = fetch::fetch_pypi_package(&name)?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = python_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                fetch::PackageSource::GitHub { owner, repo, ref_ } => {
                    eprintln!("  → Downloading from GitHub: {}/{}...", owner, repo);
                    let pkg = fetch::fetch_github_python_package(&owner, &repo, ref_.as_deref())?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = python_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                fetch::PackageSource::Local(path) => {
                    eprintln!("  → Using local path: {}...", path.display());
                    let pkg = fetch::fetch_local_python_package(&path)?;
                    eprintln!(
                        "  → Version: {}",
                        pkg.version.as_deref().unwrap_or("unknown")
                    );
                    eprintln!("  → Parsing source...");
                    let records = python_source_extractor::extract_from_source(&pkg, &options)?;
                    let records = apply_transformations(records, &options);
                    output_records(&records, options.format)?;
                }
                _ => anyhow::bail!("Invalid source for Python package"),
            };
        }
    }

    Ok(())
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

fn output_records(records: &[schema::Record], format: OutputFormat) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    match format {
        OutputFormat::Yaml => {
            for record in records {
                writeln!(handle, "---")?;
                let yaml = serde_yaml::to_string(record)?;
                write!(handle, "{}", yaml)?;
            }
        }
        OutputFormat::Json => {
            for record in records {
                let json = serde_json::to_string_pretty(record)?;
                writeln!(handle, "{}", json)?;
            }
        }
    }

    Ok(())
}
