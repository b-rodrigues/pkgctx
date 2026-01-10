//! Package source fetching
//!
//! Downloads R packages from CRAN/GitHub and Python packages from `PyPI`/GitHub.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Represents a package source location
#[derive(Debug, Clone)]
pub enum PackageSource {
    /// CRAN package (e.g., "dplyr")
    Cran(String),
    /// Bioconductor package (e.g., "GenomicRanges")
    Bioconductor(String),
    /// `PyPI` package (e.g., "numpy")
    PyPI(String),
    /// GitHub repository (e.g., "tidyverse/dplyr" or "ropensci/rix")
    GitHub {
        owner: String,
        repo: String,
        ref_: Option<String>,
    },
    /// Local path (e.g., "." or "./mypackage" or "/path/to/package")
    Local(PathBuf),
}

impl PackageSource {
    /// Parse a package specifier into a source.
    ///
    /// Formats:
    /// - `dplyr` (CRAN for R, `PyPI` for Python)
    /// - `github:owner/repo` or `github:owner/repo@ref`
    /// - `.` or `./path` or `/path` or `~/path` (local path)
    pub fn parse(spec: &str, language: &str) -> Result<Self> {
        // Check for local path indicators
        if spec.starts_with('.')
            || spec.starts_with('/')
            || spec.starts_with('~')
            || spec.starts_with("local:")
        {
            let path_str = spec.strip_prefix("local:").unwrap_or(spec);
            let path = if path_str.starts_with('~') {
                // Expand home directory
                let home = std::env::var("HOME").context("HOME environment variable not set")?;
                PathBuf::from(path_str.replacen('~', &home, 1))
            } else {
                PathBuf::from(path_str)
            };

            let canonical = path
                .canonicalize()
                .with_context(|| format!("Local path does not exist: {}", path.display()))?;

            return Ok(Self::Local(canonical));
        }

        if let Some(rest) = spec.strip_prefix("github:") {
            let (repo_part, ref_) = rest.find('@').map_or((rest, None), |at_pos| {
                (&rest[..at_pos], Some(rest[at_pos + 1..].to_string()))
            });

            let parts: Vec<&str> = repo_part.split('/').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid GitHub spec: expected 'github:owner/repo', got '{spec}'");
            }

            Ok(Self::GitHub {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                ref_,
            })
        } else {
            if let Some(name) = spec.strip_prefix("bioc:") {
                return Ok(Self::Bioconductor(name.to_string()));
            }

            match language {
                "r" | "R" => Ok(Self::Cran(spec.to_string())),
                "python" | "Python" => Ok(Self::PyPI(spec.to_string())),
                _ => anyhow::bail!("Unknown language: {language}"),
            }
        }
    }
}

/// Common trait for package information (local or fetched)
pub trait PackageInfo {
    /// Get the path to the package source directory
    fn source_path(&self) -> &Path;
    /// Get the package name
    fn name(&self) -> &str;
    /// Get the package version, if available
    fn version(&self) -> Option<&str>;
}

/// Downloaded package with extracted source
pub struct FetchedPackage {
    /// Temporary directory containing extracted source (kept alive by RAII)
    #[allow(dead_code)]
    temp_dir: TempDir,
    /// Path to the package source root
    pub source_path: PathBuf,
    /// Package name
    pub name: String,
    /// Package version (if available)
    pub version: Option<String>,
}

impl PackageInfo for FetchedPackage {
    fn source_path(&self) -> &Path {
        &self.source_path
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

/// A local package reference (no temp directory needed)
pub struct LocalPackage {
    /// Path to the package source root
    pub source_path: PathBuf,
    /// Package name
    pub name: String,
    /// Package version (if available)
    pub version: Option<String>,
}

impl PackageInfo for LocalPackage {
    fn source_path(&self) -> &Path {
        &self.source_path
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

/// Convert a path to a string, returning an error if it contains non-UTF8 characters
fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("Path contains non-UTF8 characters: {}", path.display()))
}

/// Fetch an R package from CRAN
pub fn fetch_cran_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let destdir = path_to_str(temp_dir.path())?;

    // Use R to download the package
    let r_script = format!(
        r#"
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
    "#
    );

    let output = Command::new("Rscript")
        .args(["--vanilla", "-e", &r_script])
        .output()
        .context("Failed to run Rscript for CRAN download")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to download CRAN package '{name}': {stderr}");
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

/// Fetch an R package from Bioconductor
pub fn fetch_bioconductor_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let destdir = path_to_str(temp_dir.path())?;

    // Use R to download the package via BiocManager
    let r_script = format!(
        r#"
        pkg <- "{name}"
        destdir <- "{destdir}"
        
        if (!requireNamespace("BiocManager", quietly = TRUE)) {{
            stop("BiocManager must be installed to fetch Bioconductor packages")
        }}
        
        # Ensure CRAN mirror is set
        options(repos = c(CRAN = "https://cloud.r-project.org"))
        
        # Get Bioconductor repositories
        repos <- BiocManager::repositories()
        
        # Get package info
        available <- available.packages(repos = repos)
        if (!(pkg %in% rownames(available))) {{
            stop(paste("Package", pkg, "not found in Bioconductor repositories"))
        }}
        
        version <- available[pkg, "Version"]
        
        # Download source tarball using download.packages for reliability
        # Pass 'available' to avoid re-fetching metadata
        dl_res <- download.packages(pkg, destdir, available = available, type = "source", quiet = TRUE)
        
        if (nrow(dl_res) < 1) {{
            stop("Failed to download package")
        }}
        
        destfile <- dl_res[1, 2]
        
        # Extract
        untar(destfile, exdir = destdir)
        
        # Output info
        cat("VERSION:", version, "\n")
        # Usually extract to a directory with the package name
        cat("PATH:", file.path(destdir, pkg), "\n")
    "#
    );

    let output = Command::new("Rscript")
        .args(["--vanilla", "-e", &r_script])
        .output()
        .context("Failed to run Rscript for Bioconductor download")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to download Bioconductor package '{name}': {stderr}");
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
pub fn fetch_github_r_package(
    owner: &str,
    repo: &str,
    ref_: Option<&str>,
) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let ref_name = ref_.unwrap_or("HEAD");

    // Download tarball from GitHub
    let url = format!("https://github.com/{owner}/{repo}/archive/{ref_name}.tar.gz");

    let tarball_path = temp_dir.path().join("source.tar.gz");
    let tarball_str = path_to_str(&tarball_path)?;
    let temp_dir_str = path_to_str(temp_dir.path())?;

    // Use curl to download
    let output = Command::new("curl")
        .args(["-sL", "-o", tarball_str, &url])
        .output()
        .context("Failed to download from GitHub")?;

    if !output.status.success() {
        anyhow::bail!("Failed to download from GitHub: {url}");
    }

    // Extract tarball
    let output = Command::new("tar")
        .args(["-xzf", tarball_str, "-C", temp_dir_str])
        .output()
        .context("Failed to extract tarball")?;

    if !output.status.success() {
        anyhow::bail!("Failed to extract tarball");
    }

    // Find the extracted directory (usually repo-ref/)
    let source_path =
        find_single_directory(temp_dir.path())?.unwrap_or_else(|| temp_dir.path().to_path_buf());

    // Try to get version from DESCRIPTION
    let version = parse_description_version(&source_path);

    Ok(FetchedPackage {
        temp_dir,
        source_path,
        name: repo.to_string(),
        version,
    })
}

/// Fetch a Python package from `PyPI`
pub fn fetch_pypi_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let temp_dir_str = path_to_str(temp_dir.path())?;

    // Use pip to download source
    let output = Command::new("python3")
        .args([
            "-m",
            "pip",
            "download",
            "--no-binary",
            ":all:",
            "--no-deps",
            "-d",
            temp_dir_str,
            name,
        ])
        .output()
        .context("Failed to download from PyPI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to download PyPI package '{name}': {stderr}");
    }

    // Find the downloaded file and extract it
    let entries: Vec<_> = std::fs::read_dir(temp_dir.path())?
        .filter_map(Result::ok)
        .collect();

    if entries.is_empty() {
        anyhow::bail!("No files downloaded for package '{name}'");
    }

    let archive_path = entries[0].path();
    extract_archive(&archive_path, temp_dir.path())?;

    // Find the extracted directory
    let source_path =
        find_single_directory(temp_dir.path())?.unwrap_or_else(|| temp_dir.path().to_path_buf());

    // Parse version from directory name (usually package-version/)
    let version = source_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(&format!("{name}-")))
        .map(ToString::to_string);

    Ok(FetchedPackage {
        temp_dir,
        source_path,
        name: name.to_string(),
        version,
    })
}

/// Fetch a Python package from GitHub
pub fn fetch_github_python_package(
    owner: &str,
    repo: &str,
    ref_: Option<&str>,
) -> Result<FetchedPackage> {
    // Same as R GitHub fetch
    fetch_github_r_package(owner, repo, ref_)
}

/// Load a local R package from the filesystem
pub fn fetch_local_r_package(path: &Path) -> Result<LocalPackage> {
    // Verify it looks like an R package (has DESCRIPTION file)
    let desc_path = path.join("DESCRIPTION");
    if !desc_path.exists() {
        anyhow::bail!(
            "Not a valid R package: missing DESCRIPTION file at {}",
            path.display()
        );
    }

    let name = parse_description_name(path).unwrap_or_else(|| {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    let version = parse_description_version(path);

    Ok(LocalPackage {
        source_path: path.to_path_buf(),
        name,
        version,
    })
}

/// Load a local Python package from the filesystem
pub fn fetch_local_python_package(path: &Path) -> Result<LocalPackage> {
    // Check for common Python package markers
    let has_pyproject = path.join("pyproject.toml").exists();
    let has_setup_py = path.join("setup.py").exists();
    let has_setup_cfg = path.join("setup.cfg").exists();
    let has_init = path.join("__init__.py").exists()
        || std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .any(|e| e.path().join("__init__.py").exists())
            })
            .unwrap_or(false);

    if !has_pyproject && !has_setup_py && !has_setup_cfg && !has_init {
        anyhow::bail!(
            "Not a valid Python package: no pyproject.toml, setup.py, setup.cfg, or __init__.py found at {}",
            path.display()
        );
    }

    let (name, version) = parse_python_package_info(path);

    let name = name.unwrap_or_else(|| {
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    Ok(LocalPackage {
        source_path: path.to_path_buf(),
        name,
        version,
    })
}

// ============================================================================
// Helper functions
// ============================================================================

/// Find a single directory in the given path, used for finding extracted archives
fn find_single_directory(path: &Path) -> Result<Option<PathBuf>> {
    let directories: Vec<_> = std::fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();

    Ok(if directories.len() == 1 {
        Some(directories[0].path())
    } else {
        None
    })
}

/// Extract an archive (tar.gz, tgz, or zip) to the given directory
fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let archive_str = path_to_str(archive_path)?;
    let dest_str = path_to_str(dest_dir)?;

    let extension = archive_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let file_name = archive_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if extension == "gz" || extension == "tgz" || file_name.ends_with(".tar.gz") {
        Command::new("tar")
            .args(["-xzf", archive_str, "-C", dest_str])
            .output()
            .context("Failed to extract tar.gz archive")?;
    } else if extension == "zip" {
        Command::new("unzip")
            .args(["-q", archive_str, "-d", dest_str])
            .output()
            .context("Failed to extract zip archive")?;
    }

    Ok(())
}

/// Parse version from R DESCRIPTION file
fn parse_description_version(path: &Path) -> Option<String> {
    let desc_path = path.join("DESCRIPTION");
    let content = std::fs::read_to_string(&desc_path).ok()?;

    content
        .lines()
        .find_map(|line| line.strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
}

/// Parse package name from R DESCRIPTION file
fn parse_description_name(path: &Path) -> Option<String> {
    let desc_path = path.join("DESCRIPTION");
    let content = std::fs::read_to_string(&desc_path).ok()?;

    content
        .lines()
        .find_map(|line| line.strip_prefix("Package:"))
        .map(|n| n.trim().to_string())
}

/// Parse package name/version from Python pyproject.toml or setup.py
fn parse_python_package_info(path: &Path) -> (Option<String>, Option<String>) {
    // Try pyproject.toml first
    if let Some(result) = parse_pyproject_toml(path) {
        return result;
    }

    // Fallback to setup.py
    parse_setup_py(path).unwrap_or((None, None))
}

fn parse_pyproject_toml(path: &Path) -> Option<(Option<String>, Option<String>)> {
    let content = std::fs::read_to_string(path.join("pyproject.toml")).ok()?;

    let mut name = None;
    let mut version = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("name") {
            name = name.or_else(|| extract_toml_string_value(line));
        } else if line.starts_with("version") {
            version = version.or_else(|| extract_toml_string_value(line));
        }
    }

    if name.is_some() {
        Some((name, version))
    } else {
        None
    }
}

fn parse_setup_py(path: &Path) -> Option<(Option<String>, Option<String>)> {
    let content = std::fs::read_to_string(path.join("setup.py")).ok()?;

    let mut name = None;
    let mut version = None;

    for line in content.lines() {
        let line = line.trim();
        if (line.contains("name=") || line.contains("name =")) && name.is_none() {
            name = extract_python_string_value(line, "name");
        }
        if (line.contains("version=") || line.contains("version =")) && version.is_none() {
            version = extract_python_string_value(line, "version");
        }
    }

    Some((name, version))
}

fn extract_toml_string_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.splitn(2, '=').collect();
    if parts.len() == 2 {
        let val = parts[1].trim().trim_matches('"').trim_matches('\'');
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

fn extract_python_string_value(line: &str, key: &str) -> Option<String> {
    // Match patterns like: name="foo" or name = "foo" or name='foo'
    let pattern = format!(r#"{key}\s*=\s*["']([^"']+)["']"#);
    regex::Regex::new(&pattern)
        .ok()?
        .captures(line)?
        .get(1)
        .map(|m| m.as_str().to_string())
}
