//! Python package extractor
//!
//! Extracts API information from Python packages by spawning a Python subprocess
//! and using introspection functions.

use crate::extractor::Extractor;
use crate::schema::{
    ClassRecord, Example, FunctionRecord, PackageRecord, Record, SCHEMA_VERSION,
};
use crate::ExtractOptions;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Command;

/// Extractor for Python packages
pub struct PythonExtractor;

/// Raw function info from Python introspection script
#[derive(Debug, Deserialize)]
struct PyFunctionInfo {
    name: String,
    signature: String,
    #[serde(default)]
    docstring: Option<String>,
    #[serde(default)]
    parameters: Vec<PyParameter>,
    #[serde(default)]
    return_annotation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PyParameter {
    name: String,
    #[serde(default)]
    annotation: Option<String>,
    #[serde(default)]
    default: Option<String>,
}

/// Raw class info from Python introspection
#[derive(Debug, Deserialize)]
struct PyClassInfo {
    name: String,
    #[serde(default)]
    docstring: Option<String>,
    #[serde(default)]
    methods: Vec<PyFunctionInfo>,
}

/// Raw package info from Python introspection script
#[derive(Debug, Deserialize)]
struct PyPackageInfo {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    functions: Vec<PyFunctionInfo>,
    #[serde(default)]
    classes: Vec<PyClassInfo>,
}

impl PythonExtractor {
    /// Create a new Python extractor.
    pub fn new() -> Result<Self> {
        // Verify python3 is available
        which::which("python3")
            .context("python3 executable not found in PATH. Make sure Python is available.")?;
        Ok(Self)
    }

    /// Run the Python introspection script and parse its JSON output.
    fn run_introspection(&self, package_name: &str, include_internal: bool) -> Result<PyPackageInfo> {
        let py_script = self.generate_introspection_script(package_name, include_internal);

        let output = Command::new("python3")
            .args(["-c", &py_script])
            .output()
            .context("Failed to execute python3")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Python introspection failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Find the JSON output (between markers)
        let json_start = stdout
            .find("<<<PKGCTX_JSON_START>>>")
            .context("Could not find JSON start marker in Python output")?;
        let json_end = stdout
            .find("<<<PKGCTX_JSON_END>>>")
            .context("Could not find JSON end marker in Python output")?;

        let json_str = &stdout[json_start + 23..json_end];

        serde_json::from_str(json_str)
            .with_context(|| format!("Failed to parse Python introspection JSON. Raw output length: {} bytes", json_str.len()))
    }

    /// Generate the Python script that introspects a package.
    fn generate_introspection_script(&self, package_name: &str, include_internal: bool) -> String {
        format!(r#"
import json
import inspect
import importlib
import re

def sanitize_string(s):
    """Remove control characters and normalize whitespace."""
    if s is None:
        return None
    # Remove control characters
    s = re.sub(r'[\x00-\x1f\x7f-\x9f]', ' ', str(s))
    # Normalize whitespace
    s = ' '.join(s.split())
    return s.strip() if s.strip() else None

def get_first_line(docstring):
    """Get first non-empty line of docstring."""
    if not docstring:
        return None
    lines = docstring.strip().split('\n')
    for line in lines:
        line = line.strip()
        if line:
            return sanitize_string(line)
    return None

def introspect_package(pkg_name, include_internal=False):
    try:
        pkg = importlib.import_module(pkg_name)
    except ImportError as e:
        raise ImportError(f"Package {{pkg_name}} not found: {{e}}")
    
    # Get version
    version = getattr(pkg, '__version__', 'unknown')
    
    # Get description from docstring
    description = get_first_line(pkg.__doc__)
    
    functions = []
    classes = []
    
    # Get all members
    for name, obj in inspect.getmembers(pkg):
        # Skip private/internal unless requested
        if not include_internal and name.startswith('_'):
            continue
        
        # Skip imported items (only include items defined in this package)
        if hasattr(obj, '__module__') and obj.__module__ != pkg_name:
            # Allow submodules
            if not (hasattr(obj, '__module__') and obj.__module__ and obj.__module__.startswith(pkg_name + '.')):
                continue
        
        if inspect.isfunction(obj) or inspect.isbuiltin(obj):
            func_info = extract_function_info(name, obj)
            if func_info:
                functions.append(func_info)
        elif inspect.isclass(obj):
            class_info = extract_class_info(name, obj, include_internal)
            if class_info:
                classes.append(class_info)
    
    return {{
        'name': pkg_name,
        'version': str(version),
        'description': description,
        'functions': functions,
        'classes': classes,
    }}

def extract_function_info(name, func):
    """Extract function information."""
    try:
        sig = inspect.signature(func)
        sig_str = f"{{name}}{{sig}}"
    except (ValueError, TypeError):
        sig_str = f"{{name}}(...)"
        sig = None
    
    docstring = get_first_line(func.__doc__)
    
    parameters = []
    return_annotation = None
    
    if sig:
        for param_name, param in sig.parameters.items():
            param_info = {{'name': param_name}}
            
            if param.annotation != inspect.Parameter.empty:
                param_info['annotation'] = sanitize_string(str(param.annotation))
            
            if param.default != inspect.Parameter.empty:
                try:
                    param_info['default'] = sanitize_string(repr(param.default))
                except:
                    param_info['default'] = '...'
            
            parameters.append(param_info)
        
        if sig.return_annotation != inspect.Signature.empty:
            return_annotation = sanitize_string(str(sig.return_annotation))
    
    return {{
        'name': name,
        'signature': sig_str,
        'docstring': docstring,
        'parameters': parameters,
        'return_annotation': return_annotation,
    }}

def extract_class_info(name, cls, include_internal):
    """Extract class information."""
    docstring = get_first_line(cls.__doc__)
    
    methods = []
    for method_name, method in inspect.getmembers(cls, predicate=inspect.isfunction):
        if not include_internal and method_name.startswith('_'):
            # Allow __init__
            if method_name != '__init__':
                continue
        
        method_info = extract_function_info(method_name, method)
        if method_info:
            methods.append(method_info)
    
    return {{
        'name': name,
        'docstring': docstring,
        'methods': methods,
    }}

result = introspect_package("{pkg_name}", {include_internal})
print("<<<PKGCTX_JSON_START>>>", end="")
print(json.dumps(result), end="")
print("<<<PKGCTX_JSON_END>>>")
"#,
        pkg_name = package_name,
        include_internal = if include_internal { "True" } else { "False" }
        )
    }
}

impl Extractor for PythonExtractor {
    fn extract(&self, package_name: &str, options: &ExtractOptions) -> Result<Vec<Record>> {
        let pkg_info = self.run_introspection(package_name, options.include_internal)?;

        let mut records = Vec::new();

        // Create package record
        let pkg_record = PackageRecord {
            schema_version: SCHEMA_VERSION.to_string(),
            name: pkg_info.name.clone(),
            version: pkg_info.version,
            language: "Python".to_string(),
            description: pkg_info.description,
            llm_hints: Vec::new(),
            common_arguments: BTreeMap::new(),
        };
        records.push(Record::Package(pkg_record));

        // Create function records
        for func in pkg_info.functions {
            let mut arguments = BTreeMap::new();
            for param in &func.parameters {
                let desc = param.annotation.clone()
                    .or_else(|| param.default.as_ref().map(|d| format!("default: {}", d)))
                    .unwrap_or_default();
                if !desc.is_empty() {
                    arguments.insert(param.name.clone(), desc);
                }
            }

            let func_record = FunctionRecord {
                name: func.name,
                exported: true, // Python doesn't have explicit exports
                signature: func.signature,
                purpose: func.docstring,
                role: None,
                arguments,
                arg_types: BTreeMap::new(),
                returns: None,
                return_type: func.return_annotation,
                constraints: Vec::new(),
                examples: Vec::new(),
                related: Vec::new(),
            };
            records.push(Record::Function(func_record));
        }

        // Create class records if requested
        if options.emit_classes {
            for cls in pkg_info.classes {
                let mut methods = BTreeMap::new();
                for method in cls.methods {
                    let desc = method.docstring.unwrap_or_else(|| method.signature.clone());
                    methods.insert(method.name, desc);
                }

                let class_record = ClassRecord {
                    name: cls.name,
                    constructed_by: Vec::new(),
                    methods,
                };
                records.push(Record::Class(class_record));
            }
        }

        Ok(records)
    }
}
