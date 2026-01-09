//! pkgctx — compile software packages into LLM-ready context
//!
//! Extracts structured, compact API specifications from R or Python packages
//! for use in LLMs, minimizing tokens while maximizing context.

mod compact;
mod extractor;
mod python_extractor;
mod r_extractor;
mod schema;

use crate::extractor::Extractor;
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
    /// Extract context from an R package
    R {
        /// Name of the R package to extract
        package: String,

        #[command(flatten)]
        options: ExtractOptions,
    },

    /// Extract context from a Python package
    Python {
        /// Name of the Python package to extract
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
            let extractor = r_extractor::RExtractor::new()?;
            let records = extractor.extract(&package, &options)?;
            let records = if options.compact {
                compact::compact_records(records)
            } else {
                records
            };
            output_records(&records, options.format)?;
        }
        Commands::Python { package, options } => {
            let extractor = python_extractor::PythonExtractor::new()?;
            let records = extractor.extract(&package, &options)?;
            let records = if options.compact {
                compact::compact_records(records)
            } else {
                records
            };
            output_records(&records, options.format)?;
        }
    }

    Ok(())
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
