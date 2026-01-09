//! Package source fetching
//!
//! Downloads R packages from CRAN/GitHub and Python packages from PyPI/GitHub.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Represents a package source location
#[derive(Debug, Clone)]
pub enum PackageSource {
    /// CRAN package (e.g., "dplyr")
    Cran(String),
    /// PyPI package (e.g., "numpy")
    PyPI(String),
    /// GitHub repository (e.g., "tidyverse/dplyr" or "ropensci/rix")
    GitHub { owner: String, repo: String, ref_: Option<String> },
}

impl PackageSource {
    /// Parse a package specifier into a source.
    /// 
    /// Formats:
    /// - `dplyr` (CRAN for R, PyPI for Python)
    /// - `github:owner/repo` or `github:owner/repo@ref`
    pub fn parse(spec: &str, language: &str) -> Result<Self> {
        if spec.starts_with("github:") {
            let rest = &spec[7..];
            let (repo_part, ref_) = if let Some(at_pos) = rest.find('@') {
                (&rest[..at_pos], Some(rest[at_pos + 1..].to_string()))
            } else {
                (rest, None)
            };
            
            let parts: Vec<&str> = repo_part.split('/').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid GitHub spec: expected 'github:owner/repo', got '{}'", spec);
            }
            
            Ok(PackageSource::GitHub {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                ref_,
            })
        } else {
            match language {
                "r" | "R" => Ok(PackageSource::Cran(spec.to_string())),
                "python" | "Python" => Ok(PackageSource::PyPI(spec.to_string())),
                _ => anyhow::bail!("Unknown language: {}", language),
            }
        }
    }
}

/// Downloaded package with extracted source
pub struct FetchedPackage {
    /// Temporary directory containing extracted source (kept alive by RAII)
    #[allow(dead_code)]
    pub temp_dir: TempDir,
    /// Path to the package source root
    pub source_path: PathBuf,
    /// Package name
    pub name: String,
    /// Package version (if available)
    pub version: Option<String>,
}

/// Fetch an R package from CRAN
pub fn fetch_cran_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    
    // Use R to download the package
    let r_script = format!(r#"
        pkg <- "{name}"
        destdir <- "{destdir}"
        
        # Get package info from CRAN
        available <- available.packages(repos = "https://cloud.r-project.org")
        if (!(pkg %in% rownames(available))) {{
            stop(paste("Package", pkg, "not found on CRAN"))
        }}
        
        version <- available[pkg, "Version"]
        
        # Download source tarball
        url <- paste0("https://cloud.r-project.org/src/contrib/", pkg, "_", version, ".tar.gz")
        destfile <- file.path(destdir, paste0(pkg, "_", version, ".tar.gz"))
        download.file(url, destfile, quiet = TRUE)
        
        # Extract
        untar(destfile, exdir = destdir)
        
        # Output info
        cat("VERSION:", version, "\n")
        cat("PATH:", file.path(destdir, pkg), "\n")
    "#,
        name = name,
        destdir = temp_dir.path().display()
    );
    
    let output = Command::new("Rscript")
        .args(["--vanilla", "-e", &r_script])
        .output()
        .context("Failed to run Rscript for CRAN download")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to download CRAN package '{}': {}", name, stderr);
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut version = None;
    let mut source_path = temp_dir.path().join(name);
    
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("VERSION: ") {
            version = Some(v.trim().to_string());
        }
        if let Some(p) = line.strip_prefix("PATH: ") {
            source_path = PathBuf::from(p.trim());
        }
    }
    
    Ok(FetchedPackage {
        temp_dir,
        source_path,
        name: name.to_string(),
        version,
    })
}

/// Fetch an R package from GitHub
pub fn fetch_github_r_package(owner: &str, repo: &str, ref_: Option<&str>) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let ref_name = ref_.unwrap_or("HEAD");
    
    // Download tarball from GitHub
    let url = format!(
        "https://github.com/{}/{}/archive/{}.tar.gz",
        owner, repo, ref_name
    );
    
    let tarball_path = temp_dir.path().join("source.tar.gz");
    
    // Use curl to download
    let output = Command::new("curl")
        .args(["-sL", "-o", tarball_path.to_str().unwrap(), &url])
        .output()
        .context("Failed to download from GitHub")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to download from GitHub: {}", url);
    }
    
    // Extract tarball
    let output = Command::new("tar")
        .args(["-xzf", tarball_path.to_str().unwrap(), "-C", temp_dir.path().to_str().unwrap()])
        .output()
        .context("Failed to extract tarball")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to extract tarball");
    }
    
    // Find the extracted directory (usually repo-ref/)
    let entries: Vec<_> = std::fs::read_dir(temp_dir.path())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    
    let source_path = if entries.len() == 1 {
        entries[0].path()
    } else {
        temp_dir.path().to_path_buf()
    };
    
    // Try to get version from DESCRIPTION
    let version = parse_description_version(&source_path);
    
    Ok(FetchedPackage {
        temp_dir,
        source_path,
        name: repo.to_string(),
        version,
    })
}

/// Fetch a Python package from PyPI
pub fn fetch_pypi_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    
    // Use pip to download source
    let output = Command::new("python3")
        .args([
            "-m", "pip", "download",
            "--no-binary", ":all:",
            "--no-deps",
            "-d", temp_dir.path().to_str().unwrap(),
            name,
        ])
        .output()
        .context("Failed to download from PyPI")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to download PyPI package '{}': {}", name, stderr);
    }
    
    // Find the downloaded file and extract it
    let entries: Vec<_> = std::fs::read_dir(temp_dir.path())?
        .filter_map(|e| e.ok())
        .collect();
    
    if entries.is_empty() {
        anyhow::bail!("No files downloaded for package '{}'", name);
    }
    
    let archive_path = entries[0].path();
    let archive_name = archive_path.file_name().unwrap().to_str().unwrap();
    
    // Extract based on file type
    if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        Command::new("tar")
            .args(["-xzf", archive_path.to_str().unwrap(), "-C", temp_dir.path().to_str().unwrap()])
            .output()?;
    } else if archive_name.ends_with(".zip") {
        Command::new("unzip")
            .args(["-q", archive_path.to_str().unwrap(), "-d", temp_dir.path().to_str().unwrap()])
            .output()?;
    }
    
    // Find the extracted directory
    let entries: Vec<_> = std::fs::read_dir(temp_dir.path())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    
    let source_path = if entries.len() == 1 {
        entries[0].path()
    } else {
        temp_dir.path().to_path_buf()
    };
    
    // Parse version from directory name (usually package-version/)
    let version = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(&format!("{}-", name)))
        .map(|v| v.to_string());
    
    Ok(FetchedPackage {
        temp_dir,
        source_path,
        name: name.to_string(),
        version,
    })
}

/// Fetch a Python package from GitHub
pub fn fetch_github_python_package(owner: &str, repo: &str, ref_: Option<&str>) -> Result<FetchedPackage> {
    // Same as R GitHub fetch
    fetch_github_r_package(owner, repo, ref_)
}

/// Parse version from R DESCRIPTION file
fn parse_description_version(path: &PathBuf) -> Option<String> {
    let desc_path = path.join("DESCRIPTION");
    if let Ok(content) = std::fs::read_to_string(&desc_path) {
        for line in content.lines() {
            if let Some(version) = line.strip_prefix("Version:") {
                return Some(version.trim().to_string());
            }
        }
    }
    None
}
