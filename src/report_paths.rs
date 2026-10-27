// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Safe, shared source exclusions for declared public report files.
use std::collections::BTreeSet;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;

use crate::config;
use crate::coverage_error::CoverageError;
use crate::session::array;
use crate::session::invalid;
use crate::session::read_json;
use crate::session::string;
use crate::session::utf8;

fn normalized(path: &Path) -> Result<(), CoverageError> {
    let value = utf8(path.as_os_str(), "non UTF-8 report path")?;
    if value.is_empty()
        || value.contains('\\')
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid(
            "report path must be normalized without traversal or backslash aliases",
        ));
    }
    Ok(())
}
fn physical(path: &Path) -> Result<PathBuf, CoverageError> {
    let mut current = path;
    let mut suffix = Vec::new();
    while fs::symlink_metadata(current).is_err() {
        let Some(name) = current.file_name() else {
            return Err(invalid("report path has no existing ancestor"));
        };
        suffix.push(name.to_owned());
        let Some(parent) = current.parent() else {
            return Err(invalid("report path has no parent"));
        };
        current = parent;
    }
    let mut result = current.canonicalize()?;
    for item in suffix.into_iter().rev() {
        result.push(item);
    }
    Ok(result)
}
fn artifact(rel: &Path) -> bool {
    rel.components()
        .next()
        .is_some_and(|c| c.as_os_str() == "build" || c.as_os_str() == "target")
}
fn validate_relative(project: &Path, relative: &str) -> Result<(), CoverageError> {
    let rel = Path::new(relative);
    let canonical = rel
        .components()
        .map(|component| match component {
            Component::Normal(value) => utf8(value, "non UTF-8 source exclusion"),
            _ => Err(invalid("source exclusion must contain only normal relative components")),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    if relative != canonical || relative.contains(':') {
        return Err(invalid("source exclusion must equal its normalized original spelling"));
    }
    normalized(rel)?;
    if rel.is_absolute() {
        return Err(invalid("source exclusion must be project-relative"));
    }
    if artifact(rel) {
        return Err(invalid("build outputs do not need explicit source exclusions"));
    }
    if rel.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err(invalid("source exclusions must name JSON report files"));
    }
    let Some(component) = rel.components().next() else {
        return Err(invalid("empty source exclusion"));
    };
    let first = component.as_os_str();
    let filename = rel.file_name().and_then(|v| v.to_str()).unwrap_or("");
    if first == ".infra"
        || first == ".git"
        || first == "conan"
        || filename.to_ascii_lowercase().starts_with("readme")
        || matches!(
            filename,
            "CMakeLists.txt"
                | "CMakePresets.json"
                | "CMakeUserPresets.json"
                | "conanfile.py"
                | "conanfile.txt"
                | "LICENSE"
        )
    {
        return Err(invalid("report output is a reserved configuration input"));
    }
    let scope = config::project_scope(project)?;
    let path = project.join(rel);
    let actual = physical(&path)?;
    if actual != path {
        return Err(invalid("source exclusion cannot use a symlink alias"));
    }
    for root in scope
        .source_roots
        .iter()
        .chain(&scope.test_roots)
        .chain(&scope.generated_roots)
        .chain(&scope.third_party_roots)
    {
        let root = config::owned(project, root)?;
        if path.starts_with(root) {
            return Err(invalid("report output overlaps a protected project input root"));
        }
    }
    if path.exists() {
        if !path.is_file() {
            return Err(invalid("report output must be a file"));
        }
        let report = read_json(&path).map_err(|e| invalid(format!("existing output is not a public report: {e}")))?;
        let name = report["tool"]["name"].as_str().unwrap_or("");
        if report["schema"] != 1
            || !matches!(
                name,
                "cpp-infra-tools"
                    | "cpp-infra-verify"
                    | "cpp-infra-dependency"
                    | "cpp-infra-style"
                    | "cpp-infra-analyze"
                    | "cpp-infra-coverage"
                    | "cpp-infra-ci"
                    | "cpp-infra-pages"
            )
            || !report["tool"]["version"].is_string()
            || !matches!(
                report["status"].as_str(),
                Some("passed" | "failed" | "error" | "cancelled" | "skipped")
            )
            || report["project"]
                .as_str()
                .and_then(|p| Path::new(p).canonicalize().ok())
                .as_deref()
                != Some(project)
        {
            return Err(invalid(
                "existing output is not a matching public infrastructure report",
            ));
        }
    }
    Ok(())
}
/// Validates the explicit output including ancestors resolving back into
/// inputs.
pub(crate) fn output(project: &Path, path: &Path) -> Result<PathBuf, CoverageError> {
    normalized(path)?;
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        project.join(path)
    };
    let actual = physical(&absolute)?;
    if let Ok(rel) = actual.strip_prefix(project)
        && !artifact(rel)
    {
        validate_relative(project, utf8(rel.as_os_str(), "non UTF-8 report path")?)?;
    }
    Ok(actual)
}
/// Merges metadata exclusions with this call's safe explicit report
/// destination.
pub(crate) fn merge(project: &Path, metadata: &[Value], report: Option<&Path>) -> Result<Vec<String>, CoverageError> {
    let mut paths = BTreeSet::new();
    for value in metadata {
        if let Some(list) = value.get("source_excluded_paths") {
            for item in array(list, "source_excluded_paths must be an array")? {
                let rel = string(item, "source exclusion must be a string")?;
                validate_relative(project, rel)?;
                paths.insert(rel.to_owned());
            }
        }
    }
    if let Some(report) = report {
        let path = output(project, report)?;
        if let Ok(rel) = path.strip_prefix(project)
            && !artifact(rel)
        {
            let rel = utf8(rel.as_os_str(), "non UTF-8 report path")?;
            validate_relative(project, rel)?;
            paths.insert(rel.into());
        }
    }
    Ok(paths.into_iter().collect())
}
