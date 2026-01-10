//! Python package extractor from source
//!
//! Parses Python package source directly from downloaded tarballs without requiring installation.

use crate::fetch::PackageInfo;
use crate::schema::{ClassRecord, FunctionRecord, PackageRecord, Record, SCHEMA_VERSION};
use crate::ExtractOptions;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Command;

/// Extract records from a Python package source directory
pub fn extract_from_source(pkg: &dyn PackageInfo, options: &ExtractOptions) -> Result<Vec<Record>> {
    // Use Python AST to parse the source
    let py_script =
        generate_source_parser(&pkg.source_path().to_string_lossy(), options.include_internal);

    let output = Command::new("python3")
        .args(["-c", &py_script])
        .output()
        .context("Failed to execute python3 for source parsing")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Python source parsing failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let json_start = stdout
        .find("<<<PKGCTX_JSON_START>>>")
        .context("Could not find JSON start marker")?;
    let json_end = stdout
        .find("<<<PKGCTX_JSON_END>>>")
        .context("Could not find JSON end marker")?;

    let json_str = &stdout[json_start + 23..json_end];

    let parsed: PySourceInfo = serde_json::from_str(json_str)
        .with_context(|| "Failed to parse Python AST output".to_string())?;

    let mut records = Vec::new();

    // Package record
    let pkg_record = PackageRecord {
        schema_version: SCHEMA_VERSION.to_string(),
        name: pkg.name().to_string(),
        version: pkg
            .version()
            .map(|v| v.to_string())
            .unwrap_or_else(|| parsed.version.unwrap_or_else(|| "unknown".to_string())),
        language: "Python".to_string(),
        description: parsed.description,
        llm_hints: Vec::new(),
        common_arguments: BTreeMap::new(),
    };
    records.push(Record::Package(pkg_record));

    // Function records
    for func in parsed.functions {
        let mut arguments = BTreeMap::new();
        for param in func.parameters {
            let desc = param
                .annotation
                .or_else(|| param.default.map(|d| format!("default: {}", d)))
                .unwrap_or_default();
            if !desc.is_empty() {
                arguments.insert(param.name, desc);
            }
        }

        let func_record = FunctionRecord {
            name: func.name,
            exported: true,
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

    // Class records
    if options.emit_classes {
        for cls in parsed.classes {
            let mut methods = BTreeMap::new();
            for method in cls.methods {
                let desc = method.docstring.unwrap_or(method.signature);
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

#[derive(Debug, Deserialize)]
struct PySourceInfo {
    version: Option<String>,
    description: Option<String>,
    functions: Vec<PyFuncInfo>,
    #[serde(default)]
    classes: Vec<PyClassInfo>,
}

#[derive(Debug, Deserialize)]
struct PyFuncInfo {
    name: String,
    signature: String,
    #[serde(default)]
    docstring: Option<String>,
    #[serde(default)]
    parameters: Vec<PyParamInfo>,
    #[serde(default)]
    return_annotation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PyParamInfo {
    name: String,
    #[serde(default)]
    annotation: Option<String>,
    #[serde(default)]
    default: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PyClassInfo {
    name: String,
    #[serde(default)]
    methods: Vec<PyFuncInfo>,
}

fn generate_source_parser(source_path: &str, include_internal: bool) -> String {
    format!(
        r#"
import ast
import os
import json
import re

def parse_package_source(source_path, include_internal=False):
    """Parse Python source files using AST."""
    
    functions = []
    classes = []
    version = None
    description = None
    
    # Find Python files
    py_files = []
    for root, dirs, files in os.walk(source_path):
        # Skip test directories
        dirs[:] = [d for d in dirs if d not in ['tests', 'test', '__pycache__', '.git']]
        for f in files:
            if f.endswith('.py'):
                py_files.append(os.path.join(root, f))
    
    # Try to get version from setup.py or pyproject.toml
    setup_py = os.path.join(source_path, 'setup.py')
    if os.path.exists(setup_py):
        with open(setup_py) as f:
            content = f.read()
            m = re.search(r"version\s*=\s*['\"]([^'\"]+)['\"]", content)
            if m:
                version = m.group(1)
    
    pyproject = os.path.join(source_path, 'pyproject.toml')
    if os.path.exists(pyproject):
        with open(pyproject) as f:
            content = f.read()
            m = re.search(r'version\s*=\s*["\']([^"\']+)["\']', content)
            if m:
                version = m.group(1)
    
    # Parse each Python file
    for py_file in py_files:
        try:
            with open(py_file) as f:
                source = f.read()
            tree = ast.parse(source)
            
            # Get module docstring
            if description is None and ast.get_docstring(tree):
                description = sanitize(ast.get_docstring(tree).split('\n')[0])
            
            for node in ast.walk(tree):
                if isinstance(node, ast.FunctionDef) or isinstance(node, ast.AsyncFunctionDef):
                    # Skip private functions unless requested
                    if not include_internal and node.name.startswith('_') and node.name != '__init__':
                        continue
                    
                    func_info = extract_function(node)
                    functions.append(func_info)
                
                elif isinstance(node, ast.ClassDef):
                    # Skip private classes unless requested
                    if not include_internal and node.name.startswith('_'):
                        continue
                    
                    class_info = extract_class(node, include_internal)
                    classes.append(class_info)
        except Exception as e:
            pass  # Skip files that can't be parsed
    
    return {{
        'version': version,
        'description': description,
        'functions': functions,
        'classes': classes,
    }}

def extract_function(node):
    """Extract function info from AST node."""
    name = node.name
    
    # Build signature
    params = []
    args = node.args
    
    # Regular args
    defaults_offset = len(args.args) - len(args.defaults)
    for i, arg in enumerate(args.args):
        param = {{'name': arg.arg}}
        if arg.annotation:
            param['annotation'] = sanitize(ast.unparse(arg.annotation))
        if i >= defaults_offset:
            default_idx = i - defaults_offset
            param['default'] = sanitize(ast.unparse(args.defaults[default_idx]))
        params.append(param)
    
    # *args
    if args.vararg:
        params.append({{'name': '*' + args.vararg.arg}})
    
    # Keyword-only args
    for i, arg in enumerate(args.kwonlyargs):
        param = {{'name': arg.arg}}
        if arg.annotation:
            param['annotation'] = sanitize(ast.unparse(arg.annotation))
        if args.kw_defaults[i]:
            param['default'] = sanitize(ast.unparse(args.kw_defaults[i]))
        params.append(param)
    
    # **kwargs
    if args.kwarg:
        params.append({{'name': '**' + args.kwarg.arg}})
    
    # Build signature string
    param_strs = []
    for p in params:
        s = p['name']
        if 'annotation' in p:
            s += ': ' + p['annotation']
        if 'default' in p:
            s += ' = ' + p['default']
        param_strs.append(s)
    
    signature = f"{{name}}({{', '.join(param_strs)}})"
    
    # Return annotation
    return_annotation = None
    if node.returns:
        return_annotation = sanitize(ast.unparse(node.returns))
    
    # Docstring
    docstring = None
    ds = ast.get_docstring(node)
    if ds:
        docstring = sanitize(ds.split('\n')[0])
    
    return {{
        'name': name,
        'signature': signature,
        'docstring': docstring,
        'parameters': params,
        'return_annotation': return_annotation,
    }}

def extract_class(node, include_internal):
    """Extract class info from AST node."""
    methods = []
    for item in node.body:
        if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if not include_internal and item.name.startswith('_') and item.name != '__init__':
                continue
            methods.append(extract_function(item))
    
    return {{
        'name': node.name,
        'methods': methods,
    }}

def sanitize(s):
    """Remove control characters and normalize whitespace."""
    if s is None:
        return None
    s = re.sub(r'[\x00-\x1f\x7f-\x9f]', ' ', str(s))
    return ' '.join(s.split()).strip()

result = parse_package_source("{source_path}", {include_internal})
print("<<<PKGCTX_JSON_START>>>", end="")
print(json.dumps(result), end="")
print("<<<PKGCTX_JSON_END>>>")
"#,
        source_path = source_path.replace('\\', "\\\\").replace('"', "\\\""),
        include_internal = if include_internal { "True" } else { "False" }
    )
}
