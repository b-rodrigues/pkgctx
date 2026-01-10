# pkgctx

> **pkgctx — compile software packages into LLM-ready context.**

Extracts structured, compact API specifications from R or Python packages for use in LLMs, minimizing tokens while maximizing context.

## Features

- **Language support**: R (CRAN, GitHub, local) and Python (PyPI, GitHub, local)
- **Source-based**: Downloads and parses source code on demand (no installation required)
- **Local path support**: Use `.` or `./path` to extract from local directories (great for CI)
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

# Extract from local directory (great for CI!)
pkgctx r . > mypackage.ctx.yaml
pkgctx python ./mypackage > mypackage.ctx.yaml
```

### Options

| Option | Description |
|--------|-------------|
| `--format yaml\|json` | Output format (default: yaml) |
| `--compact` | Aggressively minimize token count (~67% reduction) |
| `--include-internal` | Include non-exported/internal functions |
| `--emit-classes` | Include class specifications (Python) |
| `--hoist-common-args` | Extract common arguments to package level |
| `--no-header` | Omit the LLM instructions header from output |

### Examples

```bash
# Compact output for LLM context window (from CRAN)
pkgctx r dplyr --compact > dplyr.ctx.yaml

# Full extraction with classes (from PyPI)
pkgctx python numpy --emit-classes > numpy.ctx.yaml

# Maximum compression (from GitHub)
pkgctx r github:ropensci/rix --compact --hoist-common-args > rix.ctx.yaml

# CI: Extract context from checked-out repo
cd my-r-package
pkgctx r . > package.ctx.yaml

# Local path with different notations
pkgctx r .                    # Current directory
pkgctx r ./src/mypackage       # Relative path
pkgctx r /absolute/path/to/pkg # Absolute path
pkgctx r ~/repos/mypackage     # Home directory expansion
```

## Output Schema (v1.1)

pkgctx produces a stream of YAML records. Each record has a `kind` field:

### Context Header Record

The first record (unless `--no-header` is used) provides instructions for LLMs:

```yaml
kind: context_header
llm_instructions: >-
  This is an LLM-optimized API specification for the R package 'dplyr'.
  Use this context to write correct code using dplyr functions.
  Each 'function' record describes a public function with its signature,
  arguments, and purpose. All listed functions are part of the public API.
```

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

## CI Usage

Use `pkgctx` in GitHub Actions to extract LLM-ready context from your package on every push. This is useful for:
- Generating up-to-date API documentation for LLMs
- Detecting API drift by diffing the output
- Providing context to AI-powered code review tools

### GitHub Actions Example

```yaml
name: Generate Package Context

on:
  push:
    branches: [main]

jobs:
  generate-context:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Nix
        uses: DeterminateSystems/nix-installer-action@main
      
      - name: Generate package context (R)
        run: |
          nix run github:b-rodrigues/pkgctx -- r . > package.ctx.yaml
      
      # Or for Python packages:
      # - name: Generate package context (Python)
      #   run: |
      #     nix run github:b-rodrigues/pkgctx -- python . > package.ctx.yaml
      
      - name: Commit and push context
        run: |
          git config --local user.email "github-actions[bot]@users.noreply.github.com"
          git config --local user.name "github-actions[bot]"
          git add package.ctx.yaml
          git diff --staged --quiet || git commit -m "Update package context [skip ci]"
          git push
```

### Detecting API Drift

You can use `pkgctx` to detect breaking changes in your API:

```yaml
- name: Check for API drift
  run: |
    nix run github:b-rodrigues/pkgctx -- r . > current.ctx.yaml
    git diff --exit-code current.ctx.yaml || echo "API has changed!"
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
