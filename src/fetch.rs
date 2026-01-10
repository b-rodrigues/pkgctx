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
    /// - `dplyr` (CRAN for R, PyPI for Python)
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

            return Ok(PackageSource::Local(canonical));
        }

        if let Some(rest) = spec.strip_prefix("github:") {
            let (repo_part, ref_) = if let Some(at_pos) = rest.find('@') {
                (&rest[..at_pos], Some(rest[at_pos + 1..].to_string()))
            } else {
                (rest, None)
            };

            let parts: Vec<&str> = repo_part.split('/').collect();
            if parts.len() != 2 {
                anyhow::bail!(
                    "Invalid GitHub spec: expected 'github:owner/repo', got '{}'",
                    spec
                );
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

/// Common trait for package information (local or fetched)
pub trait PackageInfo {
    fn source_path(&self) -> &std::path::Path;
    fn name(&self) -> &str;
    fn version(&self) -> Option<&str>;
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

impl PackageInfo for FetchedPackage {
    fn source_path(&self) -> &std::path::Path {
        &self.source_path
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

/// Fetch an R package from CRAN
pub fn fetch_cran_package(name: &str) -> Result<FetchedPackage> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;

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
pub fn fetch_github_r_package(
    owner: &str,
    repo: &str,
    ref_: Option<&str>,
) -> Result<FetchedPackage> {
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
        .args([
            "-xzf",
            tarball_path.to_str().unwrap(),
            "-C",
            temp_dir.path().to_str().unwrap(),
        ])
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
            "-m",
            "pip",
            "download",
            "--no-binary",
            ":all:",
            "--no-deps",
            "-d",
            temp_dir.path().to_str().unwrap(),
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
            .args([
                "-xzf",
                archive_path.to_str().unwrap(),
                "-C",
                temp_dir.path().to_str().unwrap(),
            ])
            .output()?;
    } else if archive_name.ends_with(".zip") {
        Command::new("unzip")
            .args([
                "-q",
                archive_path.to_str().unwrap(),
                "-d",
                temp_dir.path().to_str().unwrap(),
            ])
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
pub fn fetch_github_python_package(
    owner: &str,
    repo: &str,
    ref_: Option<&str>,
) -> Result<FetchedPackage> {
    // Same as R GitHub fetch
    fetch_github_r_package(owner, repo, ref_)
}

/// Parse version from R DESCRIPTION file
fn parse_description_version(path: &std::path::Path) -> Option<String> {
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

/// Parse package name from R DESCRIPTION file
fn parse_description_name(path: &std::path::Path) -> Option<String> {
    let desc_path = path.join("DESCRIPTION");
    if let Ok(content) = std::fs::read_to_string(&desc_path) {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("Package:") {
                return Some(name.trim().to_string());
            }
        }
    }
    None
}

/// Parse package name/version from Python pyproject.toml or setup.py
fn parse_python_package_info(path: &std::path::Path) -> (Option<String>, Option<String>) {
    // Try pyproject.toml first
    let pyproject_path = path.join("pyproject.toml");
    if let Ok(content) = std::fs::read_to_string(&pyproject_path) {
        let mut name = None;
        let mut version = None;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("name") {
                if let Some(val) = extract_toml_string_value(line) {
                    name = Some(val);
                }
            } else if line.starts_with("version") && version.is_none() {
                if let Some(val) = extract_toml_string_value(line) {
                    version = Some(val);
                }
            }
        }

        if name.is_some() {
            return (name, version);
        }
    }

    // Fallback to setup.py
    let setup_path = path.join("setup.py");
    if let Ok(content) = std::fs::read_to_string(&setup_path) {
        let mut name = None;
        let mut version = None;

        for line in content.lines() {
            let line = line.trim();
            if line.contains("name=") || line.contains("name =") {
                if let Some(val) = extract_python_string_value(line, "name") {
                    name = Some(val);
                }
            } else if line.contains("version=") || line.contains("version =") {
                if let Some(val) = extract_python_string_value(line, "version") {
                    version = Some(val);
                }
            }
        }

        return (name, version);
    }

    (None, None)
}

fn extract_toml_string_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.splitn(2, '=').collect();
    if parts.len() == 2 {
        let val = parts[1].trim();
        // Remove quotes
        let val = val.trim_matches('"').trim_matches('\'');
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

fn extract_python_string_value(line: &str, key: &str) -> Option<String> {
    // Match patterns like: name="foo" or name = "foo" or name='foo'
    let pattern = format!("{}\\s*=\\s*[\"']([^\"']+)[\"']", key);
    if let Ok(re) = regex::Regex::new(&pattern) {
        if let Some(caps) = re.captures(line) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}

/// Load a local R package from the filesystem
pub fn fetch_local_r_package(path: &std::path::Path) -> Result<LocalPackage> {
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
pub fn fetch_local_python_package(path: &std::path::Path) -> Result<LocalPackage> {
    // Check for common Python package markers
    let has_pyproject = path.join("pyproject.toml").exists();
    let has_setup_py = path.join("setup.py").exists();
    let has_setup_cfg = path.join("setup.cfg").exists();
    let has_init = path.join("__init__.py").exists()
        || std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
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
    fn source_path(&self) -> &std::path::Path {
        &self.source_path
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}
