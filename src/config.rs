// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Strict project scope and coverage configuration.
use std::collections::BTreeMap;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::coverage_error::CoverageError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub schema: u32,
    pub profile: String,
    pub binary_targets: Vec<String>,
    pub required_source_roots: Vec<String>,
    #[serde(default)]
    pub threshold_exempt_files: Vec<Exemption>,
    pub jobs: usize,
    pub thresholds: BTreeMap<String, f64>,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
}
fn timeout() -> u64 {
    300
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Exemption {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Project {
    pub schema: u32,
    pub kind: String,
    pub source_roots: Vec<String>,
    #[serde(default)]
    pub test_roots: Vec<String>,
    #[serde(default)]
    pub generated_roots: Vec<String>,
    #[serde(default)]
    pub third_party_roots: Vec<String>,
    pub profiles: BTreeMap<String, Profile>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub configure: String,
    pub build: String,
    pub test: Option<String>,
    pub conan_context: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: u32,
    thresholds: BTreeMap<String, f64>,
}
fn invalid(s: impl Into<String>) -> CoverageError {
    CoverageError::Invalid(s.into())
}
/// Resolves a project-owned existing relative path, rejecting traversal and
/// symlink escapes.
pub(crate) fn owned(project: &Path, rel: &str) -> Result<PathBuf, CoverageError> {
    let p = Path::new(rel);
    if rel.contains('\\') || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(invalid(format!("invalid project path: {rel}")));
    }
    let full = project
        .join(p)
        .canonicalize()
        .map_err(|e| invalid(format!("project path {rel}: {e}")))?;
    if !full.starts_with(project) {
        return Err(invalid(format!("path escapes project: {rel}")));
    }
    Ok(full)
}
/// Rejects policy inputs outside the shared no-follow digest's tracked area.
pub(crate) fn validate_inputs(project: &Path) -> Result<(), CoverageError> {
    for rel in [
        ".infra/project.toml",
        ".infra/coverage/config.toml",
        ".infra/coverage/policy/thresholds.toml",
        "CMakeLists.txt",
        "CMakePresets.json",
        "conanfile.py",
        "conanfile.txt",
    ] {
        let path = project.join(rel);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.file_type().is_symlink() {
            return Err(invalid(format!("configuration input must not be a symlink: {rel}")));
        }
        let canonical = path.canonicalize()?;
        let actual = canonical
            .strip_prefix(project)
            .map_err(|_| invalid(format!("configuration input escapes project: {rel}")))?;
        let first = actual.components().next().map(|c| c.as_os_str());
        if !meta.is_file()
            || first.is_some_and(|c| c == ".git" || c == "build" || c == "target")
            || [".infra/.tmp", ".infra/transactions", ".infra/.transactions"]
                .iter()
                .any(|excluded| actual.starts_with(excluded))
        {
            return Err(invalid(format!(
                "configuration input lies in a digest exclusion: {rel}"
            )));
        }
    }
    Ok(())
}
/// Reads only project scope for report destination safety checks.
pub(crate) fn project_scope(project: &Path) -> Result<Project, CoverageError> {
    validate_inputs(project)?;
    toml::from_str(&config_text(&project.join(".infra/project.toml"), "project config")?)
        .map_err(|e| invalid(format!("project config: {e}")))
}
fn config_text(path: &Path, label: &str) -> Result<String, CoverageError> {
    fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            invalid(format!("missing {label}: {}", path.display()))
        } else {
            CoverageError::Infrastructure(format!("read {label} {}: {e}", path.display()))
        }
    })
}
pub(crate) fn load(project: &Path) -> Result<(Config, Project), CoverageError> {
    validate_inputs(project)?;
    let cfg: Config = toml::from_str(&config_text(
        &project.join(".infra/coverage/config.toml"),
        "coverage config",
    )?)
    .map_err(|e| invalid(format!("coverage config: {e}")))?;
    let scope: Project = toml::from_str(&config_text(&project.join(".infra/project.toml"), "project config")?)
        .map_err(|e| invalid(format!("project config: {e}")))?;
    if cfg.schema != 1 || scope.schema != 1 || !matches!(scope.kind.as_str(), "library" | "application" | "header-only")
    {
        return Err(invalid("unsupported schema or project kind"));
    }
    if cfg.jobs == 0
        || cfg.timeout_seconds == 0
        || cfg.profile.is_empty()
        || cfg.binary_targets.is_empty()
        || cfg.required_source_roots.is_empty()
    {
        return Err(invalid("profile, targets, roots, jobs and timeout must be nonempty"));
    }
    if !scope.profiles.contains_key(&cfg.profile) {
        return Err(invalid("coverage profile not declared in project.toml"));
    }
    let profile = &scope.profiles[&cfg.profile];
    if profile.configure.is_empty() || profile.build.is_empty() || profile.test.as_ref().is_none_or(String::is_empty) {
        return Err(invalid("coverage profile needs configure/build/test presets"));
    }
    let _ = &profile.conan_context;
    let mut roots = Vec::new();
    for r in scope
        .source_roots
        .iter()
        .chain(&scope.test_roots)
        .chain(&scope.generated_roots)
        .chain(&scope.third_party_roots)
    {
        let p = owned(project, r)?;
        if !p.is_dir() {
            return Err(invalid(format!("root path is not directory: {r}")));
        }
        if roots.iter().any(|x: &PathBuf| x.starts_with(&p) || p.starts_with(x)) {
            return Err(invalid(format!("overlapping project root path: {r}")));
        }
        roots.push(p);
    }
    for r in &cfg.required_source_roots {
        let p = owned(project, r)?;
        if !scope.source_roots.iter().any(|s| p.starts_with(project.join(s))) {
            return Err(invalid(format!("required root outside source roots: {r}")));
        }
    }
    let mut names = std::collections::BTreeSet::new();
    for n in &cfg.binary_targets {
        if !names.insert(n) {
            return Err(invalid("duplicate binary target"));
        }
    }
    for e in &cfg.threshold_exempt_files {
        owned(project, &e.path)?;
        if e.reason.trim().is_empty() {
            return Err(invalid("threshold exemption needs a reason"));
        }
    }
    for metric in ["lines", "functions", "branches"] {
        if !cfg.thresholds.contains_key(metric) {
            return Err(invalid(format!("missing threshold {metric}")));
        }
    }
    for (name, value) in &cfg.thresholds {
        if !matches!(name.as_str(), "lines" | "functions" | "branches" | "regions")
            || !value.is_finite()
            || !(0.0..=100.0).contains(value)
        {
            return Err(invalid(format!("invalid threshold {name}")));
        }
    }
    let policy = project.join(".infra/coverage/policy/thresholds.toml");
    if policy.exists() {
        let p: Policy =
            toml::from_str(&config_text(&policy, "coverage policy")?).map_err(|e| invalid(e.to_string()))?;
        if p.schema != 1 {
            return Err(invalid("unsupported policy schema"));
        }
        for (k, v) in p.thresholds {
            if !v.is_finite() || !(0.0..=100.0).contains(&v) || cfg.thresholds.get(&k).is_none_or(|x| *x < v) {
                return Err(invalid(format!("threshold {k} weakens organization policy")));
            }
        }
    }
    Ok((cfg, scope))
}
