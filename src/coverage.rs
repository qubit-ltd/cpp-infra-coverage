// =============================================================================
//    Copyright (c) 2025 - 2026 Haixing Hu.
//
//    SPDX-License-Identifier: Apache-2.0
//
//    Licensed under the Apache License, Version 2.0.
// =============================================================================
//! Build preparation, LLVM collection, scope validation and rendering.
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use num_bigint::BigUint;
use serde_json::Value;
use serde_json::json;

use crate::config::Config;
use crate::config::Project;
use crate::config::owned;
use crate::coverage_error::CoverageError;
use crate::digest;
use crate::session::Session;
use crate::session::array;
use crate::session::counter;
use crate::session::external;
use crate::session::invalid;
use crate::session::read_json;
use crate::session::relative;
use crate::session::string;
use crate::session::text;
use crate::session::tool;
use crate::session::utf8;
use crate::session::write_json;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| (*x).into()).collect()
}
fn checked_relative(project: &Path, path: &Path) -> Result<String, CoverageError> {
    let p = path.canonicalize()?;
    let rel = relative(&p, project, &format!("path outside project: {}", p.display()))?;
    Ok(utf8(rel.as_os_str(), "non UTF-8 path")?.replace('\\', "/"))
}
fn compiler_version(v: &str) -> Result<u32, CoverageError> {
    v.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid(format!("unrecognized LLVM version: {v}")))
}
fn llvm_tools(s: &mut Session) -> Result<(PathBuf, PathBuf), CoverageError> {
    let cov = external("CPP_INFRA_LLVM_COV", "/usr/bin/llvm-cov-18");
    let prof = external("CPP_INFRA_LLVM_PROFDATA", "/usr/bin/llvm-profdata-18");
    for (name, path) in [("llvm-cov", &cov), ("llvm-profdata", &prof)] {
        let project = s.project.clone();
        let version = s.succeed_command(path, &args(&["--version"]), &project, &BTreeMap::new())?;
        if compiler_version(&version)? != 18 {
            return Err(invalid(format!("{name} requires official LLVM 18, got {version}")));
        }
        s.report["external_versions"][name] = json!(version.trim());
    }
    Ok((cov, prof))
}
fn validate_context(s: &Session, c: &Value, cfg: &Config) -> Result<PathBuf, CoverageError> {
    if c["schema"] != 1
        || Path::new(text(c, "project")?).canonicalize()? != s.project
        || c["profile"] != cfg.profile
        || c["source_digest"] != s.source
        || c["config_digest"] != s.config
    {
        return Err(invalid("build context project/profile/source/config digest mismatch"));
    }
    if c["compiler"]["id"] != "Clang"
        || compiler_version(text(&c["compiler"], "version")?)? != 18
        || c["generator"] != "Ninja"
        || !cfg!(all(target_os = "linux", target_arch = "x86_64"))
    {
        return Err(invalid("coverage requires Linux x86_64 official Clang 18 + Ninja"));
    }
    let context_exclusions = crate::report_paths::merge(&s.project, std::slice::from_ref(c), None)?;
    if context_exclusions.iter().any(|path| !s.source_excluded.contains(path)) {
        return Err(invalid("build context source exclusion list mismatch"));
    }
    text(c, "fingerprint")?;
    let dir = Path::new(text(c, "binary_dir")?).canonicalize()?;
    let managed = s.project.join("build/infra").canonicalize()?;
    if !dir.starts_with(&managed) || dir == managed {
        return Err(invalid("coverage binary_dir outside dedicated build/infra directory"));
    }
    Ok(dir)
}
/// Revalidates the mutable build after acquiring verify's shared build lock.
fn validate_build_state(s: &Session, c: &Value, dir: &Path) -> Result<(), CoverageError> {
    let marker = read_json(&dir.join(".cpp-infra-project.json"))?;
    if marker["schema"] != 1 || marker["project"] != s.project.to_string_lossy().as_ref() {
        return Err(invalid("build directory management marker mismatch"));
    }
    let state = read_json(&dir.join(".cpp-infra-build-state.json"))?;
    for key in [
        "schema",
        "project",
        "profile",
        "source_digest",
        "config_digest",
        "fingerprint",
        "source_excluded_paths",
        "configuration_inputs",
        "cmake_cache_digest",
    ] {
        if state[key] != c[key] {
            return Err(invalid(format!(
                "build state {key} fingerprint mismatch after acquiring lock"
            )));
        }
    }
    Ok(())
}
fn instrumentation(s: &Session, c: &Value, dir: &Path) -> Result<(), CoverageError> {
    let cc = Path::new(text(c, "compile_commands")?).canonicalize()?;
    if !cc.starts_with(dir) {
        return Err(invalid("compile_commands outside binary_dir"));
    }
    let commands = read_json(&cc)?;
    let list = array(&commands, "compile_commands must be an array")?;
    if list.is_empty() {
        return Err(invalid("empty compile_commands"));
    }
    let mut seen = 0;
    for item in list {
        let file = PathBuf::from(text(item, "file")?);
        let file = if file.is_absolute() {
            file
        } else {
            Path::new(text(item, "directory")?).join(file)
        };
        if !file.starts_with(&s.project) || file.starts_with(s.project.join("build")) {
            continue;
        }
        seen += 1;
        let line = if let Some(v) = item["arguments"].as_array() {
            v.iter().map(|v| v.as_str().unwrap_or("")).collect::<Vec<_>>().join(" ")
        } else {
            text(item, "command")?.into()
        };
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if !tokens.contains(&"-fprofile-instr-generate")
            || !tokens.contains(&"-fcoverage-mapping")
            || tokens
                .iter()
                .any(|t| matches!(*t, "-O2" | "-O3" | "-Os" | "-Oz" | "-Ofast"))
        {
            return Err(invalid(format!(
                "compile instrumentation or low optimization missing: {}",
                file.display()
            )));
        }
    }
    if seen == 0 {
        return Err(invalid("no project compile commands for instrumentation validation"));
    }
    let ninja = fs::read_to_string(dir.join("build.ninja"))?;
    if !ninja.contains("-fprofile-instr-generate") {
        return Err(invalid("link instrumentation missing -fprofile-instr-generate"));
    }
    Ok(())
}
fn objects(s: &Session, c: &Value, cfg: &Config) -> Result<Vec<Value>, CoverageError> {
    let targets = array(&c["targets"], "build context targets missing")?;
    let mut objects = Vec::new();
    let mut paths = BTreeSet::new();
    for name in &cfg.binary_targets {
        let matches: Vec<&Value> = targets.iter().filter(|t| t["name"] == *name).collect();
        if matches.len() != 1 {
            return Err(invalid(format!("unknown or duplicate coverage binary target {name}")));
        }
        let t = matches[0];
        if !matches!(
            t["type"].as_str(),
            Some("EXECUTABLE" | "SHARED_LIBRARY" | "MODULE_LIBRARY")
        ) {
            return Err(invalid(format!("target {name} is not executable or dynamic library")));
        }
        let artifacts = array(&t["artifacts"], "target artifacts missing")?;
        if artifacts.is_empty() {
            return Err(invalid(format!("target {name} has no artifacts")));
        }
        for a in artifacts {
            let p = Path::new(string(a, "invalid object path")?);
            let rel = checked_relative(&s.project, p)?;
            if !p.canonicalize()?.starts_with(Path::new(text(c, "binary_dir")?)) {
                return Err(invalid("coverage object outside binary_dir"));
            }
            if paths.insert(rel.clone()) {
                objects.push(json!({"target":name,"path":rel,"sha256":digest::file(p)?}));
            }
        }
    }
    if objects.is_empty() {
        return Err(invalid("no coverage objects"));
    }
    Ok(objects)
}
fn metric(value: &Value, name: &str) -> Result<Value, CoverageError> {
    let total = counter(&value["count"], &format!("missing/invalid {name} total"))?;
    let covered = counter(&value["covered"], &format!("missing/invalid {name} covered"))?;
    if covered > total {
        return Err(invalid(format!("{name}: covered exceeds total")));
    }
    Ok(json!({"covered":covered,"total":total}))
}
fn normalize(
    s: &Session,
    c: &Value,
    scope: &Project,
    cfg: &Config,
    export: &Value,
) -> Result<Vec<Value>, CoverageError> {
    if export["type"] != "llvm.coverage.json.export" || !text(export, "version")?.starts_with("2.") {
        return Err(invalid("unsupported LLVM coverage export type/version"));
    }
    let data = array(&export["data"], "LLVM data missing")?;
    if data.len() != 1 {
        return Err(invalid("expected one combined LLVM export dataset"));
    }
    let files = array(&data[0]["files"], "LLVM files missing")?;
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for file in files {
        let path = Path::new(text(file, "filename")?);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            s.project.join(path)
        };
        let path = path
            .canonicalize()
            .map_err(|e| invalid(format!("coverage source path {}: {e}", path.display())))?;
        if !path.starts_with(&s.project) {
            continue;
        }
        let rel = checked_relative(&s.project, &path)?;
        if !scope.source_roots.iter().any(|r| path.starts_with(s.project.join(r))) {
            continue;
        }
        if !seen.insert(rel.clone()) {
            return Err(invalid(format!("duplicate LLVM file {rel}")));
        }
        let mut metrics = serde_json::Map::new();
        for name in ["lines", "functions", "branches", "regions", "instantiations"] {
            if !file["summary"][name].is_null() {
                metrics.insert(name.into(), metric(&file["summary"][name], name)?);
            } else if cfg.thresholds.contains_key(name) {
                return Err(invalid(format!("enabled LLVM metric missing: {name}")));
            }
        }
        normalized.push(json!({"path":rel,"metrics":metrics,"llvm_summary":file["summary"]}));
    }
    normalized.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    validate_scope(s, scope, cfg, &normalized)?;
    let targets = array(&c["targets"], "targets missing")?;
    for t in targets {
        for source in array(&t["sources"], "target sources missing")? {
            let path = Path::new(string(source, "invalid target source")?);
            if !path.is_absolute() {
                return Err(invalid("target sources must be absolute"));
            }
            if !scope.source_roots.iter().any(|r| path.starts_with(s.project.join(r))) {
                continue;
            }
            if matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("c" | "cc" | "cpp" | "cxx" | "C")
            ) {
                let rel = checked_relative(&s.project, path)?;
                if !seen.contains(&rel) {
                    return Err(invalid(format!(
                        "missing coverage mapping for implementation {rel}; include static-library objects in coverage linking"
                    )));
                }
            }
        }
    }
    Ok(normalized)
}
fn validate_scope(s: &Session, scope: &Project, cfg: &Config, files: &[Value]) -> Result<(), CoverageError> {
    if files.is_empty() {
        return Err(invalid("coverage contains no project source mapping"));
    }
    let mut seen = BTreeSet::new();
    for f in files {
        let rel = text(f, "path")?;
        let path = owned(&s.project, rel)?;
        if !seen.insert(rel) || !scope.source_roots.iter().any(|r| path.starts_with(s.project.join(r))) {
            return Err(invalid(format!("duplicate or out-of-scope coverage path {rel}")));
        }
    }
    for root in &cfg.required_source_roots {
        if !files
            .iter()
            .any(|f| Path::new(f["path"].as_str().unwrap_or("")).starts_with(root))
        {
            return Err(invalid(format!("required source root has no coverage mapping: {root}")));
        }
    }
    for e in &cfg.threshold_exempt_files {
        if !files.iter().any(|f| f["path"] == e.path) {
            return Err(invalid(format!("exemption has no in-scope mapping: {}", e.path)));
        }
    }
    if files
        .iter()
        .all(|f| cfg.threshold_exempt_files.iter().any(|e| f["path"] == e.path))
    {
        return Err(invalid("all coverage files are threshold exempt"));
    }
    Ok(())
}
/// Compares integer counters against the configured decimal percentage exactly.
fn meets_threshold(covered: u64, total: u64, threshold: f64) -> Result<bool, CoverageError> {
    if threshold == 0.0 {
        return Ok(true);
    }
    let decimal = threshold.to_string();
    let (mantissa, exponent) = if let Some((mantissa, exponent)) = decimal.split_once(['e', 'E']) {
        (mantissa, exponent.parse::<i32>().unwrap_or(0))
    } else {
        (decimal.as_str(), 0)
    };
    let places = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len() as i32)
        - exponent;
    let digits = mantissa.replace('.', "");
    let numerator = BigUint::parse_bytes(digits.as_bytes(), 10).ok_or_else(|| invalid("invalid decimal threshold"))?;
    let mut left = BigUint::from(covered) * BigUint::from(100u32);
    let mut right = numerator * BigUint::from(total);
    if places >= 0 {
        left *= BigUint::from(10u32).pow(places as u32);
    } else {
        right *= BigUint::from(10u32).pow((-places) as u32);
    }
    Ok(left >= right)
}
fn aggregate(cfg: &Config, files: &[Value]) -> Result<(Value, bool), CoverageError> {
    let mut summary = serde_json::Map::new();
    let mut passed = true;
    for (name, threshold) in &cfg.thresholds {
        let mut covered = 0u64;
        let mut total = 0u64;
        for f in files {
            if cfg.threshold_exempt_files.iter().any(|e| f["path"] == e.path) {
                continue;
            }
            let m = &f["metrics"][name];
            let fc = counter(&m["covered"], &format!("invalid {name} covered"))?;
            let ft = counter(&m["total"], &format!("invalid {name} total"))?;
            if fc > ft {
                return Err(invalid("covered exceeds total"));
            }
            covered = covered
                .checked_add(fc)
                .ok_or_else(|| invalid("coverage count overflow"))?;
            total = total
                .checked_add(ft)
                .ok_or_else(|| invalid("coverage count overflow"))?;
        }
        if total == 0 {
            return Err(invalid(format!("enabled {name} has zero total")));
        }
        let ok = meets_threshold(covered, total, *threshold)?;
        passed &= ok;
        summary.insert(name.clone(),json!({"covered":covered,"total":total,"percent":100.0*covered as f64/total as f64,"threshold":threshold,"passed":ok}));
    }
    Ok((Value::Object(summary), passed))
}
fn cov_arguments(s: &Session, raw: &Value) -> Result<Vec<String>, CoverageError> {
    let objects = array(&raw["objects"], "objects missing")?;
    let mut result = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        let p = owned(&s.project, text(obj, "path")?)?;
        if i > 0 {
            result.push("-object".into());
        }
        result.push(p.to_string_lossy().into_owned());
    }
    let profile = owned(&s.project, text(&raw["profile_data"], "path")?)?;
    result.push(format!("-instr-profile={}", profile.display()));
    Ok(result)
}
fn render(
    s: &mut Session,
    cfg: &Config,
    scope: &Project,
    raw: &Value,
    html: bool,
    cov: Option<&Path>,
) -> Result<bool, CoverageError> {
    let files = array(&raw["files"], "raw files missing")?;
    validate_scope(s, scope, cfg, files)?;
    let (summary, gate) = aggregate(cfg, files)?;
    let passed = gate && raw["collection_status"] == "passed";
    let dir = s.work.clone();
    fs::create_dir_all(&dir)?;
    if html {
        let tool = if let Some(p) = cov {
            p.to_owned()
        } else {
            llvm_tools(s)?.0
        };
        let html_dir = dir.join(".html-pending");
        fs::create_dir_all(&html_dir)?;
        let mut arguments = vec!["show".into()];
        arguments.extend(cov_arguments(s, raw)?);
        arguments.extend(args(&[
            "-format=html",
            "-show-line-counts-or-regions",
            "-show-branches=count",
        ]));
        arguments.push(format!("-output-dir={}", html_dir.display()));
        arguments.extend(files.iter().map(|f| {
            s.project
                .join(f["path"].as_str().unwrap_or(""))
                .to_string_lossy()
                .into_owned()
        }));
        let project = s.project.clone();
        s.succeed_command(&tool, &arguments, &project, &BTreeMap::new())?;
        if !html_dir.join("index.html").is_file() {
            return Err(invalid("llvm-cov did not produce HTML index"));
        }
    }
    s.fresh()?;
    if html {
        let html_dir = dir.join("html");
        fs::rename(dir.join(".html-pending"), &html_dir)?;
        s.artifact("coverage-html", &html_dir, true)?;
    }
    let summary_path = dir.join("summary.json");
    write_json(
        &summary_path,
        &json!({"schema":1,"summary":summary,"passed":passed,"test_summary":raw["test_summary"],"threshold_exempt_files":cfg.threshold_exempt_files.iter().map(|e|json!({"path":e.path,"reason":e.reason})).collect::<Vec<_>>(),"limitations":["Headers count only instantiated coverage mappings; uninstantiated templates have no measurable coverage.","LLVM branch coverage is not MC/DC."]}),
    )?;
    let badge = dir.join("coverage-badge.json");
    write_json(
        &badge,
        &json!({"schemaVersion":1,"label":"coverage","message":if passed{format!("{:.1}%",summary["lines"]["percent"].as_f64().unwrap_or(0.0))}else{"failed".into()},"color":if passed{"brightgreen"}else{"red"}}),
    )?;
    s.artifact("coverage-summary", &summary_path, true)?;
    s.artifact("coverage-badge", &badge, true)?;
    let raw_path = dir.join("raw.json");
    write_json(&raw_path, raw)?;
    s.artifact("coverage-report", &raw_path, true)?;
    s.report["data"] = json!({"summary":summary,"passed":passed,"collection_status":raw["collection_status"]});
    Ok(passed)
}
/// Calls verify's authoritative prepare/reuse validator before taking its build
/// lock.
fn prepare_build(
    s: &mut Session,
    cfg: &Config,
    offline: bool,
    reference: Option<&Path>,
) -> Result<PathBuf, CoverageError> {
    let project = s.project.clone();
    let context = s.work.join("verified-build-context.json");
    let child_report = s.work.join("verify-prepare.json");
    let verify = tool("cpp-infra-verify")?;
    let mut arguments = args(&["--project"]);
    arguments.push(s.project.to_string_lossy().into_owned());
    arguments.push("--report".into());
    arguments.push(child_report.to_string_lossy().into_owned());
    if offline {
        arguments.push("--offline".into());
    }
    let mut child_context = read_json(
        s.context
            .as_ref()
            .ok_or_else(|| invalid("coverage run context missing"))?,
    )?;
    child_context["profile"] = json!(cfg.profile);
    if let Some(reference) = reference {
        if !reference.is_absolute() {
            return Err(invalid("run context build_context must be absolute"));
        }
        let reference = reference.canonicalize()?;
        if !reference.starts_with(s.project.join("build/infra-runs")) {
            return Err(invalid("run context build_context outside controlled report roots"));
        }
        let evidence = read_json(&reference)?;
        validate_evidence(&evidence)?;
        let transferred = s.work.join("input-build-context.json");
        write_json(&transferred, &evidence)?;
        child_context["build_context"] = json!(transferred);
    } else if let Some(object) = child_context.as_object_mut() {
        object.remove("build_context");
    }
    let validation_context = s.work.join("verify-run-context.json");
    write_json(&validation_context, &child_context)?;
    arguments.push("--run-context".into());
    arguments.push(validation_context.to_string_lossy().into_owned());
    arguments.extend(args(&["prepare", "--profile", &cfg.profile, "--output"]));
    arguments.push(context.to_string_lossy().into_owned());
    let (exit, _) = s.command(&verify, &arguments, &project, &BTreeMap::new())?;
    let expected_status = match exit {
        0 => "passed",
        1 => "failed",
        2 | 3 => "error",
        _ => {
            return Err(invalid(format!(
                "verify prepare returned unsupported protocol exit {exit}"
            )));
        }
    };
    let child = if child_report.is_file() {
        let report = read_json(&child_report)?;
        if report["schema"] != 1 || report["tool"]["name"] != "cpp-infra-verify" || report["status"] != expected_status
        {
            return Err(invalid(
                "verify prepare report schema/tool/status contradicts exit code",
            ));
        }
        s.artifact("verify-report", &child_report, true)?;
        Some(report)
    } else {
        None
    };
    let detail = child
        .as_ref()
        .and_then(|report| report["diagnostics"][0]["message"].as_str())
        .unwrap_or("no child diagnostic");
    let message = format!("{} exited {exit}: {detail}", verify.display());
    match exit {
        0 if child.is_none() => return Err(invalid("verify prepare succeeded without its required public report")),
        0 => {}
        1 => return Err(CoverageError::Failed(message)),
        2 => return Err(invalid(message)),
        3 => return Err(CoverageError::Infrastructure(message)),
        _ => return Err(invalid(message)),
    }
    Ok(context)
}
/// Rechecks authoritative file/cache evidence after taking the consumption
/// lock.
pub(crate) fn validate_evidence(context: &Value) -> Result<(), CoverageError> {
    let inputs = array(
        &context["configuration_inputs"],
        "build context configuration_inputs missing",
    )?;
    if inputs.is_empty() {
        return Err(invalid("build context configuration_inputs must not be empty"));
    }
    for input in inputs {
        let path = Path::new(text(input, "path")?);
        if !path.is_absolute() || path.canonicalize()? != path || fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(invalid(
                "configuration evidence path must be canonical without symlinks",
            ));
        }
        if !matches!(
            text(input, "kind")?,
            "project" | "dependency-generated" | "cmake-generated" | "cmake-tool"
        ) {
            return Err(invalid("configuration evidence kind is invalid"));
        }
        if digest::file(path)? != input["sha256"] {
            return Err(invalid(format!(
                "configuration evidence digest changed: {}",
                path.display()
            )));
        }
    }
    let cache = Path::new(text(context, "binary_dir")?).join("CMakeCache.txt");
    if fs::symlink_metadata(&cache)?.file_type().is_symlink() || digest::file(&cache)? != context["cmake_cache_digest"]
    {
        return Err(invalid("CMake cache digest changed or missing evidence"));
    }
    Ok(())
}
pub(crate) fn collect(s: &mut Session, cfg: &Config, scope: &Project, offline: bool) -> Result<(), CoverageError> {
    let (cov, prof) = llvm_tools(s)?;
    let project = s.project.clone();
    let prepared = read_json(
        s.context
            .as_ref()
            .ok_or_else(|| invalid("coverage run context missing"))?,
    )?
    .get("build_context")
    .cloned();
    let reference = if let Some(prepared) = prepared {
        let path = PathBuf::from(string(
            &prepared,
            "run context build_context must be an absolute path string",
        )?);
        if !path.is_absolute() || !path.canonicalize()?.starts_with(&s.root) {
            return Err(invalid("run context build_context outside report_root"));
        }
        Some(path)
    } else {
        None
    };
    let context = prepare_build(s, cfg, offline, reference.as_deref())?;
    let c = read_json(&context)?;
    let dir = validate_context(s, &c, cfg)?;
    let _lock = s.lock_build(&dir)?;
    validate_build_state(s, &c, &dir)?;
    validate_evidence(&c)?;
    s.evidence = Some(c.clone());
    s.fresh()?;
    let context_hash = digest::file(&context)?;
    instrumentation(s, &c, &dir)?;
    let objects = objects(s, &c, cfg)?;
    s.report["external_versions"]["compiler"] = c["compiler"]["version"].clone();
    let ctest = external("CPP_INFRA_CTEST", "ctest");
    let preset = text(&c, "test_preset")?;
    let test_listing = s.succeed_command(
        &ctest,
        &args(&["--preset", preset, "--show-only=json-v1"]),
        &project,
        &BTreeMap::new(),
    )?;
    let listed: Value = serde_json::from_str(&test_listing)?;
    let selected = array(&listed["tests"], "CTest discovery missing tests")?;
    let count = selected.len();
    let enabled = selected
        .iter()
        .filter(|test| {
            !test["properties"].as_array().is_some_and(|properties| {
                properties.iter().any(|property| {
                    property["name"] == "DISABLED" && (property["value"] == true || property["value"] == "TRUE")
                })
            })
        })
        .count();
    if enabled == 0 {
        return Err(CoverageError::Failed("CTest selected zero runnable tests".into()));
    }
    let junit = s.work.join("ctest-junit.xml");
    let raw_dir = s.work.join("raw");
    fs::create_dir_all(&raw_dir)?;
    if fs::read_dir(&raw_dir)?.next().is_some() {
        return Err(invalid(
            "run profile directory must be empty; refusing historical profraw reuse",
        ));
    }
    let mut env = BTreeMap::new();
    env.insert(
        "LLVM_PROFILE_FILE".into(),
        raw_dir.join("%m-%p.profraw").to_string_lossy().into_owned(),
    );
    let (test_exit, _) = s.command(
        &ctest,
        &args(&[
            "--preset",
            preset,
            "--output-on-failure",
            "--no-tests=error",
            "--output-junit",
            &junit.to_string_lossy(),
            "--parallel",
            &cfg.jobs.to_string(),
        ]),
        &project,
        &env,
    )?;
    let xml = fs::read_to_string(&junit)?;
    let document = roxmltree::Document::parse(&xml).map_err(|e| invalid(format!("CTest JUnit: {e}")))?;
    let cases: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name("testcase"))
        .collect();
    let skipped = cases
        .iter()
        .filter(|node| {
            node.attribute("status") == Some("notrun") || node.children().any(|child| child.has_tag_name("skipped"))
        })
        .count();
    let failures = cases
        .iter()
        .filter(|node| {
            node.children()
                .any(|child| child.has_tag_name("failure") || child.has_tag_name("error"))
        })
        .count();
    let executed = cases.len() - skipped;
    let test_passed = test_exit == 0 && executed > 0 && failures == 0;
    let test_summary = json!({"total":count,"selected":count,"enabled":enabled,"executed":executed,"skipped":skipped,"failures":failures,"exit_code":test_exit,"passed":test_passed});
    s.report["data"]["test_summary"] = test_summary.clone();
    s.artifact("ctest-junit", &junit, true)?;
    let mut profiles = Vec::new();
    for e in fs::read_dir(&raw_dir)? {
        let e = e?;
        if e.file_type()?.is_file() && e.path().extension().is_some_and(|x| x == "profraw") {
            if e.metadata()?.len() == 0 {
                return Err(invalid("empty profraw"));
            }
            profiles.push(e.path());
        }
    }
    profiles.sort();
    if profiles.is_empty() {
        if !test_passed {
            return Err(CoverageError::Failed(
                "CTest executed no passing tests or failed without coverage profiles".into(),
            ));
        }
        return Err(invalid("no fresh profraw produced; missing runtime instrumentation"));
    }
    let profdata = s.work.join("merged.profdata");
    let mut merge = args(&["merge", "-sparse"]);
    merge.extend(profiles.iter().map(|p| p.to_string_lossy().into_owned()));
    merge.push("-o".into());
    merge.push(profdata.to_string_lossy().into_owned());
    let (merge_exit, _) = s.command(&prof, &merge, &project, &BTreeMap::new())?;
    if merge_exit != 0 {
        return Err(invalid("LLVM profile merge failed: corrupt/incompatible profile"));
    }
    if fs::metadata(&profdata)?.len() == 0 {
        return Err(invalid("empty merged profile"));
    }
    let mut raw = json!({"schema":1,"backend":"llvm-source-based","llvm_export_version":null,"source_digest":s.source,"source_excluded_paths":s.source_excluded,"config_digest":s.config,"build_context_digest":context_hash,"build_context_path":checked_relative(&project,&context)?,"build_fingerprint":c["fingerprint"],"profile":cfg.profile,"objects":objects,"profile_data":{"path":checked_relative(&project,&profdata)?,"sha256":digest::file(&profdata)?},"test_summary":test_summary,"collection_status":if test_passed{"passed"}else{"failed"},"files":[]});
    let mut export = args(&["export"]);
    export.extend(cov_arguments(s, &raw)?);
    let (exit, output) = s.command(&cov, &export, &project, &BTreeMap::new())?;
    if exit != 0 {
        return Err(invalid(
            "LLVM export failed: missing mapping or incompatible objects/profile",
        ));
    }
    let exported: Value = serde_json::from_str(&output)?;
    for object in array(&raw["objects"], "objects missing")? {
        let path = owned(&project, text(object, "path")?)?;
        let probe = vec![
            "export".into(),
            path.to_string_lossy().into_owned(),
            format!("-instr-profile={}", profdata.display()),
            "-summary-only".into(),
        ];
        let (exit, output) = s.command(&cov, &probe, &project, &BTreeMap::new())?;
        if exit != 0 {
            return Err(invalid(format!(
                "selected object has no valid coverage mapping: {}",
                path.display()
            )));
        }
        let mapping: Value = serde_json::from_str(&output)?;
        if mapping["data"].as_array().is_none_or(|data| {
            !data
                .iter()
                .any(|d| d["files"].as_array().is_some_and(|f| !f.is_empty()))
        }) {
            return Err(invalid(format!(
                "selected object has no coverage mapping: {}",
                path.display()
            )));
        }
    }
    let export_path = s.work.join("llvm-export.json");
    write_json(&export_path, &exported)?;
    s.artifact("llvm-export", &export_path, true)?;
    raw["llvm_export_version"] = exported["version"].clone();
    raw["files"] = json!(normalize(s, &c, scope, cfg, &exported)?);
    let (_, gate) = aggregate(cfg, array(&raw["files"], "files missing")?)?;
    if !gate {
        raw["collection_status"] = json!("failed");
    }
    s.fresh()?;
    let passed = render(s, cfg, scope, &raw, true, Some(&cov))?;
    if !test_passed || !passed {
        return Err(CoverageError::Failed("coverage test or percentage gate failed".into()));
    }
    Ok(())
}
fn validate_raw(s: &Session, cfg: &Config, scope: &Project, raw: &Value) -> Result<(), CoverageError> {
    if raw["schema"] != 1
        || raw["backend"] != "llvm-source-based"
        || !text(raw, "llvm_export_version")?.starts_with("2.")
        || raw["source_digest"] != s.source
        || raw["config_digest"] != s.config
        || raw["profile"] != cfg.profile
    {
        return Err(invalid("raw coverage schema/backend/source/config digest mismatch"));
    }
    if !matches!(raw["collection_status"].as_str(), Some("passed" | "failed"))
        || raw["test_summary"]["total"].as_u64().is_none_or(|n| n == 0)
        || raw["test_summary"]["exit_code"].as_i64().is_none()
        || raw["test_summary"]["selected"].as_u64().is_none()
        || raw["test_summary"]["executed"].as_u64().is_none()
        || raw["test_summary"]["skipped"].as_u64().is_none()
        || raw["test_summary"]["failures"].as_u64().is_none()
        || raw["test_summary"]["passed"].as_bool().is_none()
    {
        return Err(invalid("invalid raw test summary/status"));
    }
    let context = owned(&s.project, text(raw, "build_context_path")?)?;
    if digest::file(&context)? != raw["build_context_digest"] {
        return Err(invalid("build context digest mismatch"));
    }
    let c = read_json(&context)?;
    validate_context(s, &c, cfg)?;
    if c["fingerprint"] != raw["build_fingerprint"] {
        return Err(invalid("build fingerprint mismatch"));
    }
    let expected_objects = objects(s, &c, cfg)?;
    if raw["objects"] != json!(expected_objects) {
        return Err(invalid(
            "raw selected object set/digest differs from verified build context",
        ));
    }
    let objects = array(&raw["objects"], "raw objects missing")?;
    if objects.is_empty() {
        return Err(invalid("raw objects empty"));
    }
    for object in objects.iter().chain(std::iter::once(&raw["profile_data"])) {
        let p = owned(&s.project, text(object, "path")?)?;
        if digest::file(&p)? != object["sha256"] {
            return Err(invalid(format!("object/profile digest mismatch: {}", p.display())));
        }
    }
    validate_scope(s, scope, cfg, array(&raw["files"], "raw files missing")?)?;
    Ok(())
}
pub(crate) fn reuse(
    s: &mut Session,
    cfg: &Config,
    scope: &Project,
    input: &Path,
    html: bool,
    offline: bool,
) -> Result<(), CoverageError> {
    let raw = read_json(input)?;
    let context = owned(&s.project, text(&raw, "build_context_path")?)?;
    let original = read_json(&context)?;
    validate_context(s, &original, cfg)?;
    validate_evidence(&original)?;
    let verified = prepare_build(s, cfg, offline, Some(&context))?;
    let c = read_json(&verified)?;
    if c["fingerprint"] != original["fingerprint"] {
        return Err(invalid("verified build fingerprint differs from coverage input"));
    }
    let dir = validate_context(s, &c, cfg)?;
    let _lock = s.lock_build(&dir)?;
    validate_build_state(s, &c, &dir)?;
    validate_evidence(&c)?;
    s.evidence = Some(c.clone());
    s.fresh()?;
    validate_raw(s, cfg, scope, &raw)?;
    let passed = render(s, cfg, scope, &raw, html, None)?;
    if raw["collection_status"] != "passed"
        || raw["test_summary"]["passed"] != true
        || raw["test_summary"]["exit_code"] != 0
        || raw["test_summary"]["executed"].as_u64().is_none_or(|n| n == 0)
        || raw["test_summary"]["failures"] != 0
        || (!html && !passed)
    {
        return Err(CoverageError::Failed(
            "coverage input preserves failed test/collection or percentage gate".into(),
        ));
    }
    Ok(())
}

/// Downgrades generated success indicators if inputs change before publication.
pub(crate) fn invalidate(s: &mut Session) -> Result<(), CoverageError> {
    for name in ["summary.json", "raw.json", "coverage-badge.json"] {
        let path = s.work.join(name);
        if !path.is_file() {
            continue;
        }
        let mut value = read_json(&path)?;
        match name {
            "summary.json" => {
                value["passed"] = json!(false);
                value["invalidation_reason"] = json!("inputs changed before publication");
            }
            "raw.json" => {
                value["collection_status"] = json!("failed");
                value["invalidation_reason"] = json!("inputs changed before publication");
            }
            _ => {
                value["message"] = json!("invalid");
                value["color"] = json!("red");
            }
        }
        write_json(&path, &value)?;
    }
    let index = s.work.join("html/index.html");
    if index.is_file() {
        fs::write(
            &index,
            "<!doctype html><title>Coverage invalid</title><p>Inputs changed before publication. Collect coverage again.</p>",
        )?;
    }
    if let Some(artifacts) = s.report["artifacts"].as_array_mut() {
        for artifact in artifacts {
            if matches!(
                artifact["kind"].as_str(),
                Some("coverage-report" | "coverage-summary" | "coverage-badge" | "coverage-html")
            ) {
                let path = s.root.join(text(artifact, "path")?);
                artifact["sha256"] = json!(if path.is_dir() {
                    digest::tree(&path, None, &[])?
                } else {
                    digest::file(&path)?
                });
            }
        }
    }
    s.report["data"]["passed"] = json!(false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::Parser;
    use serde_json::json;

    use super::invalidate;
    use super::render;
    use crate::cli::Cli;
    use crate::config;
    use crate::digest;
    use crate::session::Session;
    use crate::session::read_json;

    #[test]
    fn test_final_invalidation_downgrades_outputs_and_refreshes_artifact_digests() {
        let project = tempfile::tempdir().expect("temporary project");
        fs::create_dir_all(project.path().join(".infra/coverage")).unwrap();
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::write(project.path().join("src/a.cpp"), "int a();").unwrap();
        fs::write(project.path().join(".infra/project.toml"),"schema=1\nkind='application'\nsource_roots=['src']\ntest_roots=[]\ngenerated_roots=[]\nthird_party_roots=[]\n[profiles.coverage]\nconfigure='coverage'\nbuild='coverage'\ntest='coverage'\n").unwrap();
        fs::write(project.path().join(".infra/coverage/config.toml"),"schema=1\nprofile='coverage'\nbinary_targets=['tests']\nrequired_source_roots=['src']\nthreshold_exempt_files=[]\njobs=1\n[thresholds]\nlines=90.0\nfunctions=95.0\nbranches=80.0\n").unwrap();
        let cli = Cli::try_parse_from(["coverage", "--project", project.path().to_str().unwrap(), "plan"]).unwrap();
        let mut session = Session::new(&cli).unwrap();
        let (cfg, scope) = config::load(&session.project).unwrap();
        let metric = json!({"covered":10,"total":10});
        let raw = json!({"schema":1,"backend":"llvm-source-based","source_digest":session.source,"config_digest":session.config,"collection_status":"passed","test_summary":{"selected":1,"executed":1,"skipped":0,"failures":0,"passed":true},"files":[{"path":"src/a.cpp","metrics":{"lines":metric,"functions":metric,"branches":metric}}]});
        assert!(render(&mut session, &cfg, &scope, &raw, false, None).unwrap());
        let html = session.work.join("html");
        fs::create_dir(&html).unwrap();
        fs::write(html.join("index.html"), "original coverage").unwrap();
        session.artifact("coverage-html", &html, true).unwrap();
        fs::write(project.path().join("src/a.cpp"), "int changed();").unwrap();
        assert!(session.fresh().is_err());
        invalidate(&mut session).unwrap();
        assert_eq!(read_json(&session.work.join("summary.json")).unwrap()["passed"], false);
        assert_eq!(
            read_json(&session.work.join("raw.json")).unwrap()["collection_status"],
            "failed"
        );
        assert_eq!(
            read_json(&session.work.join("coverage-badge.json")).unwrap()["color"],
            "red"
        );
        assert_eq!(session.report["data"]["passed"], false);
        assert!(
            fs::read_to_string(html.join("index.html"))
                .unwrap()
                .contains("Coverage invalid")
        );
        for artifact in session.report["artifacts"].as_array().unwrap() {
            let path = session.root.join(artifact["path"].as_str().unwrap());
            let actual = if path.is_dir() {
                digest::tree(&path, None, &[]).unwrap()
            } else {
                digest::file(&path).unwrap()
            };
            assert_eq!(artifact["sha256"], actual);
        }
    }
}
