# pkgctx

> **pkgctx — compile software packages into LLM-ready context.**
> Extracts structured, compact API specifications from R or Python packages for use in LLMs, minimizing tokens while maximizing context.

---

## 1. Motivation

Modern LLMs can’t reliably use code libraries unless they are given **structured, minimal, and precise descriptions** of:

* public functions and classes
* argument semantics
* return shapes
* usage patterns and constraints

Human-oriented documentation (Rd, docstrings, Markdown) is **verbose, redundant, and expensive in tokens**.

`pkgctx` solves this by **distilling packages into a compact, LLM-optimized context**, ready for ingestion.

---

## 2. Key Features

* **Language support**: R, Python
* **Formats**: YAML (default, token-efficient), JSON (optional)
* **Output**: One file per package, streamable (YAML list of records)
* **Content**: Functions, classes, arguments, return types, examples, constraints, workflows
* **Deterministic**: Byte-for-byte reproducible given same package version

**Not a doc generator**: Only data relevant to *LLM usage* is included.

---

## 3. Determinism and Nix

`pkgctx` is designed to **produce deterministic outputs**:

* The same package + tool version → identical YAML file
* Ensures reproducibility in CI, caching, and model pipelines

To guarantee determinism, `pkgctx` uses a **Nix flake**:

* Pins Rust compiler (`rustc` / `cargo`)
* Pins R / Python versions + package dependencies
* Builds a statically reproducible binary

**flake.nix skeleton**:

```nix
{
  description = "Nix flake for building pkgctx deterministically";

  outputs = { self, nixpkgs }: {
    packages.x86_64-linux.pkgctx = nixpkgs.lib.mkDerivation {
      pname = "pkgctx";
      version = "0.1.0";
      src = ./.;
      buildInputs = [ nixpkgs.rPackages.r nixpkgs.python39Packages.python ];
      buildPhase = ''
        cargo build --release
      '';
      installPhase = ''
        mkdir -p $out/bin
        cp target/release/pkgctx $out/bin/
      '';
    };
    devShells.default = nixpkgs.mkShell {
      buildInputs = [ nixpkgs.rPackages.r nixpkgs.python39Packages.python nixpkgs.cargo nixpkgs.rustc ];
    };
  };
}
```

---

## 4. CLI Design

### Basic usage

```bash
pkgctx <language> <package> [options]
```

**Examples:**

```bash
pkgctx r dplyr > dplyr.ctx.yaml
pkgctx python numpy --compact > numpy.ctx.yaml
```

### Options

| Option                | Description                                                    |                               |
| --------------------- | -------------------------------------------------------------- | ----------------------------- |
| `--format yaml        | json`                                                          | Output format (default: yaml) |
| `--compact`           | Aggressively minimize token count                              |                               |
| `--include-internal`  | Include non-exported/internal functions                        |                               |
| `--emit-classes`      | Include class specifications                                   |                               |
| `--emit-workflows`    | Include canonical workflows                                    |                               |
| `--hoist-common-args` | Extract frequently used arguments to package-level common_args |                               |

---

## 5. YAML Output Schema

`pkgctx` produces a **stream of records** in YAML. Each record is **self-describing** via a `kind` field.

### 5.1 Record Types

1. **package**: Package metadata + common arguments
2. **function**: Public functions / methods
3. **class**: Public classes
4. **workflow**: Canonical sequences of functions

---

### 5.2 Example: Package Record

```yaml
kind: package
schema_version: "1.1"
name: dplyr
version: 1.1.0
language: R
description: Minimal LLM-ready API spec for dplyr
llm_hints:
  - Prefer functional style
  - Objects are immutable
common_arguments:
  ...: Passed to lower-level functions
  data: data.frame or tibble
```

---

### 5.3 Example: Function Record

```yaml
kind: function
name: filter
exported: true
signature: filter(.data, ..., .preserve = FALSE)
purpose: Keep rows matching logical expressions
role: transformer
arguments:
  .data: data.frame or tibble
  ...: logical expressions
  .preserve: boolean
arg_types:
  .data: table
  ...: logical_vector
  .preserve: scalar_boolean
returns: object of same type as .data
return_type: table
constraints:
  - expressions must evaluate to logical
examples:
  - code: filter(df, x > 1)
    shows: ["default_usage"]
related:
  - arrange
  - select
```

---

### 5.4 Example: Class Record

```yaml
kind: class
name: mypkg_model
constructed_by:
  - fit_model
methods:
  predict: Return numeric predictions
  coef: Return coefficient matrix
```

---

### 5.5 Example: Workflow Record

```yaml
kind: workflow
name: basic_modeling
steps:
  - m <- fit_model(x, y)
  - predict(m, x)
purpose: Fit a model and generate predictions
```

---

## 6. Design Principles

1. **Explicit**: No implicit assumptions, all fields defined
2. **Token-efficient**: YAML plain style, minimal quotes, compact examples
3. **Deterministic**: Stable ordering, reproducible builds via Nix
4. **LLM-focused**: Examples, constraints, workflows for model reasoning
5. **Language-agnostic**: Core in Rust; language-specific extractors isolated

---

## 7. Integration

* **CI / Testing**: Detect API drift

```bash
pkgctx r mypkg > mypkg.ctx.yaml
git diff --exit-code mypkg.ctx.yaml
```

* **LLM Prompting**: Feed `pkgctx` output directly into context

```python
with open("mypkg.ctx.yaml") as f:
    context = f.read()
llm.run(prompt="Use mypkg to transform this dataset", context=context)
```

* **Caching / Artifact**: Hash YAML output, reuse across sessions

---

## 8. v1.1 Schema Extensions

* `arg_types` and `return_type` for light symbolic typing
* `role` for function behavior classification
* `workflow` record type for canonical usage sequences
* `examples` annotations (`shows`)

v1.1 is fully backward compatible with v1.0 readers.

---

## 9. Summary

**pkgctx** is:

* A **compiler**, not a doc generator
* Deterministic via **Nix flake builds**
* Optimized for **LLM token efficiency**
* Outputs **YAML** (default) or JSON for CI / validation
* Designed to scale across **R and Python**

> Tagline: **"pkgctx — compile software packages into LLM-ready context."**

