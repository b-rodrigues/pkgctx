# pkgctx

> **pkgctx — compile software packages into LLM-ready context.**

Extracts structured, compact API specifications from R or Python packages for use in LLMs, minimizing tokens while maximizing context.

## Features

- **Language support**: R (CRAN, GitHub) and Python (PyPI, GitHub)
- **Source-based**: Downloads and parses source code on demand (no installation required)
- **Formats**: YAML (default, token-efficient) or JSON
- **Deterministic**: Reproducible via Nix flake
- **Token-efficient**: Compact mode reduces output by ~67%
- **LLM-focused**: Extracts signatures, arguments, docs, examples

## Installation

### Run directly from GitHub (no installation needed)

```bash
# Run pkgctx directly without cloning
nix run github:b-rodrigues/pkgctx -- r rix > rix.ctx.yaml
```

### Build from source

```bash
# Clone the repository
git clone https://github.com/b-rodrigues/pkgctx.git
cd pkgctx

# Enter the Nix development shell (provides Rust, R, Python)
nix develop

# Build the project
cargo build --release
```

## Usage

### Basic Usage

```bash
# Extract R package from CRAN
pkgctx r dplyr > dplyr.ctx.yaml

# Extract R package from GitHub
pkgctx r github:ropensci/rix > rix.ctx.yaml

# Extract Python package from PyPI
pkgctx python requests > requests.ctx.yaml

# Extract Python package from GitHub
pkgctx python github:psf/requests > requests.ctx.yaml
```

### Options

| Option | Description |
|--------|-------------|
| `--format yaml\|json` | Output format (default: yaml) |
| `--compact` | Aggressively minimize token count (~67% reduction) |
| `--include-internal` | Include non-exported/internal functions |
| `--emit-classes` | Include class specifications (Python) |
| `--hoist-common-args` | Extract common arguments to package level |

### Examples

```bash
# Compact output for LLM context window (from CRAN)
pkgctx r dplyr --compact > dplyr.ctx.yaml

# Full extraction with classes (from PyPI)
pkgctx python numpy --emit-classes > numpy.ctx.yaml

# Maximum compression (from GitHub)
pkgctx r github:ropensci/rix --compact --hoist-common-args > rix.ctx.yaml
```

## Output Schema (v1.1)

pkgctx produces a stream of YAML records. Each record has a `kind` field:

### Package Record

```yaml
kind: package
schema_version: '1.1'
name: dplyr
version: 1.1.0
language: R
description: A Grammar of Data Manipulation
common_arguments:
  .data: A data frame or tibble
```

### Function Record

```yaml
kind: function
name: filter
exported: true
signature: filter(.data, ..., .preserve = FALSE)
purpose: Keep rows matching logical expressions
arguments:
  .data: A data frame or tibble
  ...: Expressions that return logical vectors
returns: An object of the same type as .data
```

### Class Record

```yaml
kind: class
name: JSONDecoder
methods:
  decode: Return the Python representation of a JSON string
  raw_decode: Decode a JSON document from a string
```

## Development

```bash
# Enter dev shell
nix develop

# Run tests
cargo test

# Build release binary
cargo build --release
```

## License

GPL-3.0-or-later
